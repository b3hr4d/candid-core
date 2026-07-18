use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractViolation {
    pub code: String,
    pub path: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_limit: Option<crate::diagnostics::ResourceLimitInfo>,
}

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
            violations: vec![ContractViolation {
                code: code.into(),
                path: path.into(),
                message: message.into(),
                resource_limit: None,
            }],
        }
    }

    pub(crate) fn resource_limit(resource: &str, limit: usize, observed: usize) -> Self {
        Self {
            violations: vec![ContractViolation {
                code: "resource_limit_exceeded".to_string(),
                path: "$".to_string(),
                message: format!("resource {resource} exceeded limit {limit}; observed {observed}"),
                resource_limit: Some(crate::diagnostics::ResourceLimitInfo {
                    resource: resource.to_string(),
                    limit,
                    observed,
                }),
            }],
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
