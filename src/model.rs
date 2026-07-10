use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::fmt;

pub const CONTRACT_VERSION: u32 = 1;
pub const SOURCE_INFO_VERSION: u32 = 1;
pub type TypeRef = u32;

/// The wire-semantics Contract consumed by host runtimes.
///
/// `declarations` supplies named roots for the arena, but names are provenance
/// and are deliberately excluded from the semantic fingerprint. Comments,
/// source spelling, and raw source are kept in [`SourceInfo`], not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contract {
    pub contract_version: u32,
    pub fingerprint: String,
    pub types: Vec<TypeNode>,
    #[serde(default)]
    pub declarations: Vec<Declaration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
}

impl Contract {
    /// Validate graph structure and verify the semantic fingerprint.
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        crate::validate::validate_contract(self)
    }

    /// Return a deterministically re-indexed copy with a freshly calculated
    /// fingerprint. This is useful at JSON trust boundaries.
    pub fn canonicalize(&self) -> Result<Self, ContractValidationError> {
        crate::canonical::canonicalize_contract(self)
    }

    /// Serialize validated canonical JSON.
    pub fn to_json_pretty(&self) -> Result<String, ContractValidationError> {
        self.validate()?;
        let canonical = self.canonicalize()?;
        serde_json::to_string_pretty(&canonical).map_err(|error| {
            ContractValidationError::single(
                "contract_json_serialization_failed",
                "$",
                error.to_string(),
            )
        })
    }

    /// Parse, validate, and canonicalize a Contract JSON document.
    pub fn from_json(input: &str) -> Result<Self, ContractJsonError> {
        let raw: RawContract = serde_json::from_str(input)
            .map_err(|error| ContractJsonError::MalformedJson(error.to_string()))?;
        Self::from_raw(raw).map_err(ContractJsonError::InvalidContract)
    }

    fn from_raw(raw: RawContract) -> Result<Self, ContractValidationError> {
        let contract = Self {
            contract_version: raw.contract_version,
            fingerprint: raw.fingerprint,
            types: raw.types,
            declarations: raw.declarations,
            actor: raw.actor,
        };
        contract.validate()?;
        contract.canonicalize()
    }
}

/// Deliberately private: callers can only deserialize through the validating
/// `Contract` implementation below.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContract {
    contract_version: u32,
    fingerprint: String,
    types: Vec<TypeNode>,
    #[serde(default)]
    declarations: Vec<Declaration>,
    #[serde(default)]
    actor: Option<Actor>,
}

impl<'de> Deserialize<'de> for Contract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawContract::deserialize(deserializer)?;
        Self::from_raw(raw).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Actor {
    Service { service: TypeRef },
    Class { class: TypeRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypeNode {
    Primitive {
        primitive: PrimitiveType,
    },
    Opt {
        inner: TypeRef,
    },
    Vec {
        inner: TypeRef,
    },
    Record {
        fields: Vec<Field>,
    },
    Variant {
        fields: Vec<Field>,
    },
    Func {
        args: Vec<TypeRef>,
        results: Vec<TypeRef>,
        mode: MethodMode,
    },
    Service {
        methods: Vec<ServiceMethod>,
    },
    Class {
        init: Vec<TypeRef>,
        service: TypeRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveType {
    Null,
    Bool,
    Nat,
    Int,
    Nat8,
    Nat16,
    Nat32,
    Nat64,
    Int8,
    Int16,
    Int32,
    Int64,
    Float32,
    Float64,
    Text,
    Reserved,
    Empty,
    Principal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodMode {
    /// The absence of a Candid annotation, made explicit in the Contract.
    Update,
    Query,
    CompositeQuery,
    Oneway,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    /// The authoritative Candid label ID: numeric label or `idl_hash(name)`.
    pub id: u32,
    #[serde(rename = "type")]
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceMethod {
    /// Method text is required to invoke a service. `id` is retained as the
    /// authoritative Candid hash for reflection and validation.
    pub name: String,
    pub id: u32,
    #[serde(rename = "function")]
    pub function: TypeRef,
}

/// Optional source/provenance data returned alongside, but never embedded in,
/// the canonical Contract. It has no effect on wire semantics or fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo {
    pub source_info_version: u32,
    pub sources: Vec<SourceFileInfo>,
    #[serde(default)]
    pub declarations: Vec<SourceDeclaration>,
    #[serde(default)]
    pub field_labels: Vec<FieldLabelProvenance>,
    #[serde(default)]
    pub methods: Vec<SourceMethodInfo>,
    #[serde(default)]
    pub function_arguments: Vec<SourceFunctionArgumentInfo>,
    #[serde(default)]
    pub actors: Vec<SourceActorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFileInfo {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDeclaration {
    pub source: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceActorInfo {
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLabelProvenance {
    /// A source occurrence is retained even when multiple spellings lower to
    /// the same semantic container node.
    pub origin: SourceOrigin,
    /// A stable AST-shaped occurrence path within `origin`, not a byte span.
    pub path: String,
    pub container: TypeRef,
    pub id: u32,
    pub label: SourceLabel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceOrigin {
    Declaration { source: String, name: String },
    Actor { source: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMethodInfo {
    pub origin: SourceOrigin,
    pub path: String,
    pub service: TypeRef,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFunctionArgumentInfo {
    pub origin: SourceOrigin,
    pub path: String,
    pub function: TypeRef,
    pub direction: SourceFunctionArgumentDirection,
    pub position: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFunctionArgumentDirection {
    Argument,
    Result,
}

/// Source spelling is intentionally separate from the semantic field ID.
/// `positional` differentiates tuple syntax from an explicitly numeric label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceLabel {
    Named { name: String },
    Numeric,
    Positional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractViolation {
    pub code: String,
    pub path: String,
    pub message: String,
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
