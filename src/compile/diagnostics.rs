use super::*;
use crate::diagnostics::RelatedLocation;

pub(super) fn budget_error(
    error: crate::budget::BudgetError,
    phase: DiagnosticPhase,
    operation: &str,
) -> CompileError {
    match error {
        crate::budget::BudgetError::Cancelled => CompileError::single(
            "operation_cancelled",
            phase,
            format!("{operation} was cancelled"),
        ),
        crate::budget::BudgetError::DeadlineExceeded => CompileError::single(
            "operation_deadline_exceeded",
            phase,
            format!("{operation} deadline has elapsed"),
        ),
        crate::budget::BudgetError::ResourceLimit {
            resource,
            limit,
            observed,
        } => CompileError::resource_limit(
            resource,
            limit,
            observed,
            format!("resource {resource} exceeded limit {limit}; observed {observed}"),
        ),
    }
}

pub(super) fn source_info_compile_error(error: crate::ContractValidationError) -> CompileError {
    error.into_compile_error(DiagnosticPhase::Lower)
}

pub(super) fn lower_error(message: impl Into<String>) -> CompileError {
    CompileError::single("contract_lowering_error", DiagnosticPhase::Lower, message)
}

/// One upstream report label: a byte range plus its message. The upstream
/// report type carries no file identity (`Diagnostic<()>`), so which text the
/// range indexes is decided by the caller's span policy.
pub(super) struct ReportLabel {
    pub(super) range: std::ops::Range<usize>,
    pub(super) message: String,
}

/// Convert report labels against **original** text into the primary span plus
/// ordered related locations. Label order is preserved: the first label is the
/// primary location, the rest become `related` in report order.
pub(super) fn exact_labels_to_locations(
    labels: Vec<ReportLabel>,
    source_name: Option<&str>,
) -> (Option<SourceSpan>, Vec<RelatedLocation>) {
    let mut labels = labels.into_iter();
    let span = labels.next().map(|label| {
        SourceSpan::exact(
            source_name.map(str::to_string),
            crate::limits::portable_count(label.range.start),
            crate::limits::portable_count(label.range.end),
        )
    });
    let related = labels
        .map(|label| RelatedLocation {
            message: label.message,
            span: Some(SourceSpan::exact(
                source_name.map(str::to_string),
                crate::limits::portable_count(label.range.start),
                crate::limits::portable_count(label.range.end),
            )),
        })
        .collect();
    (span, related)
}

/// Convert report labels whose ranges index **rewritten** (pretty-printed,
/// materialized) text, for a diagnostic whose main message already embeds the
/// primary label's message (the parse-error path). The offsets are not valid
/// for any original source and the report carries no file identity, so no
/// span is published and the later labels keep only their messages, in report
/// order.
pub(super) fn suppressed_labels_to_locations(
    labels: Vec<ReportLabel>,
) -> (Option<SourceSpan>, Vec<RelatedLocation>) {
    let mut labels = labels.into_iter();
    let _primary_message_is_in_the_diagnostic_message = labels.next();
    (None, suppressed_related(labels))
}

/// Convert rewritten-text report labels for a diagnostic whose main message
/// does **not** embed any label message (the `Display`-derived paths). Every
/// label message — the primary included — is retained as an ordered related
/// entry, so no upstream label message is ever silently lost; offsets stay
/// suppressed.
pub(super) fn all_suppressed_labels_to_related(labels: Vec<ReportLabel>) -> Vec<RelatedLocation> {
    suppressed_related(labels.into_iter())
}

fn suppressed_related(labels: impl Iterator<Item = ReportLabel>) -> Vec<RelatedLocation> {
    labels
        .map(|label| RelatedLocation {
            message: label.message,
            span: None,
        })
        .collect()
}

fn code_for_phase(phase: &DiagnosticPhase) -> &'static str {
    match phase {
        DiagnosticPhase::Parse => "did_parse_error",
        DiagnosticPhase::TypeCheck => "did_type_check_error",
        DiagnosticPhase::Load => "did_load_error",
        DiagnosticPhase::Lower => "contract_lowering_error",
    }
}

fn report_parts(error: &candid_parser::Error) -> (Vec<ReportLabel>, Vec<String>) {
    let report = error.report();
    let labels = report
        .labels
        .into_iter()
        .map(|label| ReportLabel {
            range: label.range,
            message: label.message,
        })
        .collect();
    (labels, report.notes)
}

/// Classify and convert an error from `candid_parser::check_file` over the
/// materialized bundle.
///
/// Everything positional in these errors refers to the rewritten bundle, not
/// to original sources: byte ranges index pretty-printed text and file
/// identities are the numeric `{index}.did` names inside the private temp
/// directory. Ranges are therefore suppressed rather than republished as if
/// they were original offsets, and file identities are mapped back to logical
/// source IDs — both in the message text and as a source-scoped span when the
/// error names exactly one source.
pub(super) fn candid_file_error(
    error: candid_parser::Error,
    bundle: &MaterializedBundle,
) -> CompileError {
    let phase = match &error {
        candid_parser::Error::Parse(_) => DiagnosticPhase::Parse,
        candid_parser::Error::Custom(inner)
            if inner.to_string().contains("Cannot import")
                || inner.to_string().contains("Cannot open")
                || inner.to_string().contains("io error") =>
        {
            DiagnosticPhase::Load
        }
        candid_parser::Error::Custom(_) | candid_parser::Error::CandidError(_) => {
            DiagnosticPhase::TypeCheck
        }
    };
    let (labels, notes) = report_parts(&error);
    let (message, span, related) = match &error {
        // Never render a `Parse` error through `Display`; see
        // `parse_error_message`. The range is withheld from the message too —
        // it indexes rewritten text.
        candid_parser::Error::Parse(_) => {
            let message =
                parse_error_message(labels.first().map(|label| label.message.as_str()), None);
            let (span, related) = suppressed_labels_to_locations(labels);
            (message, span, related)
        }
        candid_parser::Error::Custom(_) | candid_parser::Error::CandidError(_) => {
            let (message, referenced) = bundle.map_materialized_names(error.to_string());
            let span = match referenced.as_slice() {
                [name] => Some(SourceSpan::source_only(*name)),
                _ => None,
            };
            // The Display-derived message embeds no label message, so every
            // label — the primary included — is kept as a related entry.
            (message, span, all_suppressed_labels_to_related(labels))
        }
    };
    let mut diagnostic = Diagnostic::compiler(code_for_phase(&phase), phase, message);
    diagnostic.span = span;
    diagnostic.related = related;
    diagnostic.notes = notes;
    CompileError {
        diagnostics: vec![diagnostic],
    }
}

/// Describe a parse error without rendering the token it carries.
///
/// `candid_parser` stores an unescaped string literal by pushing raw bytes
/// through `String::as_mut_vec`, so a `\XX` byte escape can leave
/// `Token::Text` holding bytes that are not valid UTF-8. That is deliberate on
/// its side — the WebAssembly text format allows it — and documented at
/// `candid_parser-0.4.0/src/token.rs:331-334`.
///
/// The consequence here is that `Display for Token` forwards to `Debug`, which
/// reaches `<str as Debug>::fmt` and panics on those bytes. `Display for
/// candid_parser::Error` renders the token for every `Parse` error, so
/// `error.to_string()` turns a five-byte source into a panic instead of a
/// `CompileError`.
///
/// `Error::report()` never renders a token: it carries byte offsets and fixed
/// labels. Parse errors are therefore described from the report, and the
/// offending text is located by the structured [`SourceSpan`] rather than
/// interpolated into the message.
///
/// Catching the panic is not an alternative. `libfuzzer-sys` installs a panic
/// hook that aborts the process before unwinding, so `catch_unwind` would
/// still leave `fuzz_targets/source_parsing.rs` crashing on the same input —
/// verified against libfuzzer-sys 0.4.13.
fn parse_error_message(label: Option<&str>, range: Option<&std::ops::Range<usize>>) -> String {
    let detail = label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or("invalid syntax");
    match range {
        Some(range) => format!(
            "Candid parser error: {detail} at bytes {}..{}",
            range.start, range.end
        ),
        None => format!("Candid parser error: {detail}"),
    }
}

/// Convert an error whose report ranges index the **original** text that
/// `source_name` identifies. Every report label is retained: the first as the
/// primary exact span, the rest as ordered related locations.
pub(super) fn candid_error(
    error: candid_parser::Error,
    phase: DiagnosticPhase,
    source_name: Option<String>,
) -> CompileError {
    let (labels, notes) = report_parts(&error);
    let message = match &error {
        // Never render a `Parse` error through `Display`; see above.
        candid_parser::Error::Parse(_) => {
            let primary = labels.first();
            parse_error_message(
                primary.map(|label| label.message.as_str()),
                primary.map(|label| &label.range),
            )
        }
        // These carry no token, so their `Display` is safe and stays verbatim.
        candid_parser::Error::Custom(_) | candid_parser::Error::CandidError(_) => error.to_string(),
    };
    let (span, related) = exact_labels_to_locations(labels, source_name.as_deref());
    let mut diagnostic = Diagnostic::compiler(code_for_phase(&phase), phase, message);
    diagnostic.span = span;
    diagnostic.related = related;
    diagnostic.notes = notes;
    CompileError {
        diagnostics: vec![diagnostic],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(start: usize, end: usize, message: &str) -> ReportLabel {
        ReportLabel {
            range: start..end,
            message: message.to_string(),
        }
    }

    #[test]
    fn every_exact_label_is_retained_in_report_order() {
        let labels = vec![
            label(4, 9, "primary label"),
            label(20, 25, "first related"),
            label(1, 2, "second related"),
        ];
        let (span, related) = exact_labels_to_locations(labels, Some("memory:/entry.did"));
        assert_eq!(
            span,
            Some(SourceSpan::exact(
                Some("memory:/entry.did".to_string()),
                4,
                9
            ))
        );
        assert_eq!(
            related,
            vec![
                RelatedLocation {
                    message: "first related".to_string(),
                    span: Some(SourceSpan::exact(
                        Some("memory:/entry.did".to_string()),
                        20,
                        25
                    )),
                },
                RelatedLocation {
                    message: "second related".to_string(),
                    span: Some(SourceSpan::exact(
                        Some("memory:/entry.did".to_string()),
                        1,
                        2
                    )),
                },
            ]
        );
    }

    #[test]
    fn suppressed_labels_keep_messages_but_never_offsets() {
        let labels = vec![
            label(4, 9, "primary label"),
            label(20, 25, "first related"),
            label(1, 2, "second related"),
        ];
        let (span, related) = suppressed_labels_to_locations(labels);
        assert_eq!(span, None, "rewritten offsets must not be published");
        assert_eq!(
            related,
            vec![
                RelatedLocation {
                    message: "first related".to_string(),
                    span: None,
                },
                RelatedLocation {
                    message: "second related".to_string(),
                    span: None,
                },
            ]
        );
    }

    #[test]
    fn display_derived_diagnostics_keep_every_suppressed_label_message() {
        // When the main message embeds no label message, dropping the primary
        // label would silently lose it: all labels become related entries, in
        // report order, still without offsets.
        let labels = vec![
            label(4, 9, "primary label"),
            label(20, 25, "first related"),
            label(1, 2, "second related"),
        ];
        assert_eq!(
            all_suppressed_labels_to_related(labels),
            vec![
                RelatedLocation {
                    message: "primary label".to_string(),
                    span: None,
                },
                RelatedLocation {
                    message: "first related".to_string(),
                    span: None,
                },
                RelatedLocation {
                    message: "second related".to_string(),
                    span: None,
                },
            ]
        );
    }

    #[test]
    fn empty_label_lists_produce_no_locations() {
        let (span, related) = exact_labels_to_locations(Vec::new(), Some("memory:/entry.did"));
        assert_eq!(span, None);
        assert!(related.is_empty());
        let (span, related) = suppressed_labels_to_locations(Vec::new());
        assert_eq!(span, None);
        assert!(related.is_empty());
    }

    #[test]
    fn source_info_conversion_is_lossless_item_by_item() {
        let error = crate::ContractValidationError {
            violations: vec![Diagnostic::violation(
                "source_type_ref_out_of_bounds",
                "$.declarations[0]",
                "declaration references a missing node",
            )
            .with_resource_limit(crate::ResourceLimitInfo {
                resource: "source_declarations".to_string(),
                limit: 1,
                observed: 2,
            })],
        };
        let converted = source_info_compile_error(error);
        assert_eq!(
            converted.diagnostics,
            vec![Diagnostic {
                code: "source_type_ref_out_of_bounds".to_string(),
                phase: Some(DiagnosticPhase::Lower),
                severity: Some(crate::Severity::Error),
                path: Some("$.declarations[0]".to_string()),
                message: "$.declarations[0]: declaration references a missing node".to_string(),
                span: None,
                related: Vec::new(),
                notes: Vec::new(),
                resource_limit: Some(crate::ResourceLimitInfo {
                    resource: "source_declarations".to_string(),
                    limit: 1,
                    observed: 2,
                }),
            }]
        );
    }

    #[test]
    fn materialized_parse_errors_suppress_rewritten_offsets() {
        // A parse error crossing the check_file boundary carries a range into
        // pretty-printed materialized text: neither the span nor the message
        // may republish those offsets, while the expected-token notes stay.
        let error = "type Broken = ;"
            .parse::<IDLProg>()
            .expect_err("not a valid program");
        let bundle = MaterializedBundle::for_tests(
            std::env::temp_dir().join(format!("candid-core-parse-arm-test-{}", std::process::id())),
            vec!["memory:/entry.did".to_string()],
        );
        let converted = candid_file_error(error, &bundle);
        let diagnostic = &converted.diagnostics[0];
        assert_eq!(diagnostic.code, "did_parse_error");
        assert_eq!(diagnostic.phase, Some(DiagnosticPhase::Parse));
        assert_eq!(
            diagnostic.span, None,
            "rewritten offsets must not be published"
        );
        // The primary label's message ("Unexpected token") must survive the
        // complete conversion path verbatim inside the main message, with the
        // rewritten byte range withheld.
        assert_eq!(diagnostic.message, "Candid parser error: Unexpected token");
        assert_eq!(
            diagnostic.related,
            Vec::new(),
            "a single-label parse report has no secondary labels"
        );
        assert!(
            !diagnostic.notes.is_empty(),
            "expected-token notes must survive"
        );
    }
}
