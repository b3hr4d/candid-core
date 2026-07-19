use super::*;

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
    CompileError {
        diagnostics: error
            .violations
            .into_iter()
            .map(|violation| Diagnostic {
                code: violation.code,
                phase: DiagnosticPhase::Lower,
                severity: Severity::Error,
                message: format!("{}: {}", violation.path, violation.message),
                span: None,
                notes: Vec::new(),
                resource_limit: violation.resource_limit,
            })
            .collect(),
    }
}

pub(super) fn lower_error(message: impl Into<String>) -> CompileError {
    CompileError::single("contract_lowering_error", DiagnosticPhase::Lower, message)
}

pub(super) fn candid_file_error(error: candid_parser::Error) -> CompileError {
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
    candid_error(error, phase, None)
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

pub(super) fn candid_error(
    error: candid_parser::Error,
    phase: DiagnosticPhase,
    source_name: Option<String>,
) -> CompileError {
    let report = error.report();
    let label = report.labels.first();
    let range = label.map(|label| label.range.clone());
    let message = match &error {
        // Never render a `Parse` error through `Display`; see above.
        candid_parser::Error::Parse(_) => {
            parse_error_message(label.map(|label| label.message.as_str()), range.as_ref())
        }
        // These carry no token, so their `Display` is safe and stays verbatim.
        candid_parser::Error::Custom(_) | candid_parser::Error::CandidError(_) => error.to_string(),
    };
    let span = range.map(|range| SourceSpan {
        source_name,
        start_byte: range.start,
        end_byte: range.end,
    });
    let code = match phase {
        DiagnosticPhase::Parse => "did_parse_error",
        DiagnosticPhase::TypeCheck => "did_type_check_error",
        DiagnosticPhase::Load => "did_load_error",
        DiagnosticPhase::Lower => "contract_lowering_error",
    };
    CompileError {
        diagnostics: vec![Diagnostic {
            code: code.to_string(),
            phase,
            severity: Severity::Error,
            message,
            span,
            notes: report.notes,
            resource_limit: None,
        }],
    }
}
