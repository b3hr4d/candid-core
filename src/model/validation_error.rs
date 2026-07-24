use crate::diagnostics::Diagnostic;
#[cfg(feature = "compiler")]
use crate::diagnostics::{CompileError, DiagnosticPhase, Severity};
use std::fmt;

/// Compatibility name for the shared diagnostic item in the Contract
/// validation domain.
///
/// Contract violations are [`Diagnostic`] values that always carry `path` and
/// never carry `phase`/`severity`, so their serialized shape is unchanged:
/// `{code, path, message, resource_limit?}` plus any optional location data.
pub type ContractViolation = Diagnostic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractValidationError {
    pub violations: Vec<ContractViolation>,
}

impl ContractValidationError {
    pub(crate) fn single(
        code: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            violations: vec![Diagnostic::violation(code, path, message)],
        }
    }

    pub(crate) fn resource_limit(resource: &str, limit: usize, observed: usize) -> Self {
        Self {
            violations: vec![Diagnostic::resource_violation(
                resource,
                crate::limits::portable_count(limit),
                crate::limits::portable_count(observed),
            )],
        }
    }

    /// Lossless item-by-item conversion into a [`CompileError`].
    ///
    /// Every violation keeps its code, structured path, span, related
    /// locations, notes, and resource metadata, and gains the compile-domain
    /// `phase`/`severity`. The message keeps the pre-existing `{path}:
    /// {message}` rendering so compile output stays byte-compatible.
    #[cfg(feature = "compiler")]
    pub(crate) fn into_compile_error(self, phase: DiagnosticPhase) -> CompileError {
        CompileError {
            diagnostics: self
                .violations
                .into_iter()
                .map(|violation| {
                    let message = match &violation.path {
                        Some(path) => format!("{}: {}", path, violation.message),
                        None => violation.message.clone(),
                    };
                    Diagnostic {
                        phase: Some(phase.clone()),
                        severity: Some(Severity::Error),
                        message,
                        ..violation
                    }
                })
                .collect(),
        }
    }
}

impl fmt::Display for ContractValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Contract validation failed with {} violation(s)",
            self.violations.len()
        )
    }
}

impl std::error::Error for ContractValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractJsonError {
    MalformedJson(String),
    InvalidContract(ContractValidationError),
}

impl fmt::Display for ContractJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedJson(message) => write!(formatter, "Malformed Contract JSON: {message}"),
            Self::InvalidContract(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ContractJsonError {}
