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

/// A logical source location.
///
/// Two forms exist, and producers must never mix them up:
///
/// * an **exact** span carries `start_byte`/`end_byte` offsets that are valid
///   for the named source's original text (see [`SourceSpan::exact`]);
/// * a **source-scoped** location names a logical source without offsets (see
///   [`SourceSpan::source_only`]), used when the underlying tool reported a
///   position against rewritten text whose offsets do not apply to the
///   original source.
///
/// `start_byte` and `end_byte` are set together or not at all, and a span
/// carries a source name, offsets, or both — deserialization rejects
/// half-spans and empty spans. Offsets are omitted from JSON when absent, so
/// pre-existing exact-span output is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSourceSpan")]
pub struct SourceSpan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<usize>,
}

/// Decode-side mirror of [`SourceSpan`] so the two-forms invariant is checked
/// on every deserialization, not only at the constructors.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSourceSpan {
    #[serde(default)]
    source_name: Option<String>,
    #[serde(default)]
    start_byte: Option<usize>,
    #[serde(default)]
    end_byte: Option<usize>,
}

impl TryFrom<RawSourceSpan> for SourceSpan {
    type Error = String;

    fn try_from(raw: RawSourceSpan) -> Result<Self, String> {
        match (raw.start_byte.is_some(), raw.end_byte.is_some()) {
            (true, true) => {}
            (false, false) if raw.source_name.is_some() => {}
            (false, false) => {
                return Err("a source span names a source, carries offsets, or both".to_string())
            }
            _ => return Err("start_byte and end_byte are set together or not at all".to_string()),
        }
        Ok(Self {
            source_name: raw.source_name,
            start_byte: raw.start_byte,
            end_byte: raw.end_byte,
        })
    }
}

impl SourceSpan {
    /// An exact byte range into the original text of `source_name`.
    pub fn exact(source_name: Option<String>, start_byte: usize, end_byte: usize) -> Self {
        Self {
            source_name,
            start_byte: Some(start_byte),
            end_byte: Some(end_byte),
        }
    }

    /// A location scoped to a logical source, with no byte offsets.
    pub fn source_only(source_name: impl Into<String>) -> Self {
        Self {
            source_name: Some(source_name.into()),
            start_byte: None,
            end_byte: None,
        }
    }
}

/// A secondary location attached to a [`Diagnostic`], in the order the
/// underlying tool reported it after the primary location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelatedLocation {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
}

/// The one serializable failure item shared by every domain in this crate.
///
/// `Diagnostic` is the single item algebra behind compiler failures
/// ([`CompileError`]), Contract/provenance validation
/// ([`crate::ContractValidationError`], whose items are the compatibility
/// alias [`crate::ContractViolation`]), and HostValue validation
/// ([`crate::HostValueValidationError`], items
/// [`crate::HostValueViolation`]). Domains differ only in which optional
/// fields they populate:
///
/// * compile diagnostics always carry `phase` and `severity`;
/// * validation violations always carry `path` and never `phase`/`severity`.
///
/// Every optional field is omitted from JSON when absent, so each domain's
/// pre-existing serialized shape is unchanged. Construct items through
/// [`Diagnostic::compiler`], [`Diagnostic::violation`], or
/// [`Diagnostic::resource_violation`] so no producer silently drops fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<DiagnosticPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedLocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_limit: Option<ResourceLimitInfo>,
}

impl Diagnostic {
    /// A compile-domain item: `phase` and `severity` are always present.
    pub fn compiler(
        code: impl Into<String>,
        phase: DiagnosticPhase,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            phase: Some(phase),
            severity: Some(Severity::Error),
            path: None,
            message: message.into(),
            span: None,
            related: Vec::new(),
            notes: Vec::new(),
            resource_limit: None,
        }
    }

    /// A validation-domain item: `path` is always present and
    /// `phase`/`severity` never are.
    pub fn violation(
        code: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            phase: None,
            severity: None,
            path: Some(path.into()),
            message: message.into(),
            span: None,
            related: Vec::new(),
            notes: Vec::new(),
            resource_limit: None,
        }
    }

    /// The canonical resource-limit violation shared by every validation
    /// domain: code `resource_limit_exceeded`, path `$`, the standard message
    /// template, and the exact `{resource, limit, observed}` triple.
    pub fn resource_violation(resource: &str, limit: usize, observed: usize) -> Self {
        Self::violation(
            "resource_limit_exceeded",
            "$",
            format!("resource {resource} exceeded limit {limit}; observed {observed}"),
        )
        .with_resource_limit(ResourceLimitInfo {
            resource: resource.to_string(),
            limit,
            observed,
        })
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_related(mut self, related: Vec<RelatedLocation>) -> Self {
        self.related = related;
        self
    }

    pub fn with_notes(mut self, notes: Vec<String>) -> Self {
        self.notes = notes;
        self
    }

    pub fn with_resource_limit(mut self, resource_limit: ResourceLimitInfo) -> Self {
        self.resource_limit = Some(resource_limit);
        self
    }
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
            diagnostics: vec![Diagnostic::compiler(code, phase, message)],
        }
    }

    pub(crate) fn resource_limit(
        resource: &str,
        limit: usize,
        observed: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostics: vec![Diagnostic::compiler(
                "resource_limit_exceeded",
                DiagnosticPhase::Load,
                message,
            )
            .with_resource_limit(ResourceLimitInfo {
                resource: resource.to_string(),
                limit,
                observed,
            })],
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
