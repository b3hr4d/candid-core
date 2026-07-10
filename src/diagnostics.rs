use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPhase {
    Parse,
    TypeCheck,
    Load,
    Lower,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimitInfo {
    pub resource: String,
    pub limit: usize,
    pub observed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub phase: DiagnosticPhase,
    pub severity: Severity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_limit: Option<ResourceLimitInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub diagnostics: Vec<Diagnostic>,
}

impl CompileError {
    pub(crate) fn single(
        code: impl Into<String>,
        phase: DiagnosticPhase,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostics: vec![Diagnostic {
                code: code.into(),
                phase,
                severity: Severity::Error,
                message: message.into(),
                span: None,
                notes: Vec::new(),
                resource_limit: None,
            }],
        }
    }

    pub(crate) fn resource_limit(
        resource: &str,
        limit: usize,
        observed: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostics: vec![Diagnostic {
                code: "resource_limit_exceeded".to_string(),
                phase: DiagnosticPhase::Load,
                severity: Severity::Error,
                message: message.into(),
                span: None,
                notes: Vec::new(),
                resource_limit: Some(ResourceLimitInfo {
                    resource: resource.to_string(),
                    limit,
                    observed,
                }),
            }],
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(formatter, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(formatter, "Candid compilation failed")
        }
    }
}

impl std::error::Error for CompileError {}
