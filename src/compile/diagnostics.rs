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

pub(super) fn candid_error(
    error: candid_parser::Error,
    phase: DiagnosticPhase,
    source_name: Option<String>,
) -> CompileError {
    let message = error.to_string();
    let report = error.report();
    let span = report.labels.first().map(|label| SourceSpan {
        source_name,
        start_byte: label.range.start,
        end_byte: label.range.end,
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
