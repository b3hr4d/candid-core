use crate::limits::Limits;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub const CONTRACT_VERSION: u32 = 1;
pub const SOURCE_INFO_VERSION: u32 = 1;
pub const CONTRACT_FORMAT: &str = "candid-contract";
pub const FORMAT_VERSION: u32 = 1;
pub const SEMANTICS_PROFILE: &str = "candid-1";
pub const CANONICALIZATION_PROFILE: &str = "ccr-canon-1";
pub type TypeRef = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractIdentities {
    pub contract: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerInfo {
    pub name: String,
    pub version: String,
    pub candid_version: String,
    pub candid_parser_version: String,
}

impl ProducerInfo {
    pub(crate) fn current() -> Self {
        Self {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            candid_version: "0.10.30".to_string(),
            candid_parser_version: "0.4.0".to_string(),
        }
    }
}

/// The wire-semantics Contract consumed by host runtimes.
///
/// `declarations` supplies named roots for the arena, but names are provenance
/// and are deliberately excluded from the semantic fingerprint. Comments,
/// source spelling, and raw source are kept in [`SourceInfo`], not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub(crate) format: String,
    pub(crate) format_version: u32,
    pub(crate) semantics_profile: String,
    pub(crate) canonicalization_profile: String,
    pub(crate) contract_version: u32,
    pub(crate) identities: ContractIdentities,
    pub(crate) fingerprint: String,
    pub(crate) producer: ProducerInfo,
    pub(crate) types: Vec<TypeNode>,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) actor: Option<Actor>,
}

impl Contract {
    pub fn format(&self) -> &str {
        &self.format
    }

    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    pub fn semantics_profile(&self) -> &str {
        &self.semantics_profile
    }

    pub fn canonicalization_profile(&self) -> &str {
        &self.canonicalization_profile
    }

    pub fn contract_version(&self) -> u32 {
        self.contract_version
    }

    pub fn identities(&self) -> &ContractIdentities {
        &self.identities
    }

    pub fn contract_id(&self) -> &str {
        &self.identities.contract
    }

    pub fn interface_id(&self) -> Option<&str> {
        self.identities.interface.as_deref()
    }

    /// Legacy pre-stable semantic fingerprint. New integrations should choose
    /// [`Self::contract_id`] or [`Self::interface_id`].
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn producer(&self) -> &ProducerInfo {
        &self.producer
    }

    pub fn types(&self) -> &[TypeNode] {
        &self.types
    }

    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    pub fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }

    /// Validate graph structure and verify the semantic fingerprint.
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        self.validate_with_limits(&Limits::default())
    }

    pub fn validate_with_limits(&self, limits: &Limits) -> Result<(), ContractValidationError> {
        crate::validate::validate_contract_with_limits(self, limits)
    }

    /// Return a deterministically re-indexed copy with a freshly calculated
    /// fingerprint. This is useful at JSON trust boundaries.
    pub fn canonicalize(&self) -> Result<Self, ContractValidationError> {
        self.canonicalize_with_limits(&Limits::default())
    }

    pub fn canonicalize_with_limits(
        &self,
        limits: &Limits,
    ) -> Result<Self, ContractValidationError> {
        crate::canonical::canonicalize_contract_with_limits(self, limits)
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
        Self::from_json_with_limits(input, &Limits::default())
    }

    pub fn from_json_with_limits(input: &str, limits: &Limits) -> Result<Self, ContractJsonError> {
        if input.len() > limits.max_input_bytes {
            return Err(ContractJsonError::InvalidContract(
                ContractValidationError::resource_limit(
                    "input_bytes",
                    limits.max_input_bytes,
                    input.len(),
                ),
            ));
        }
        let raw: RawContract = serde_json::from_str(input)
            .map_err(|error| ContractJsonError::MalformedJson(error.to_string()))?;
        Self::from_raw_with_limits(raw, limits).map_err(ContractJsonError::InvalidContract)
    }

    pub fn try_from_raw(raw: RawContract) -> Result<Self, ContractValidationError> {
        Self::from_raw_with_limits(raw, &Limits::default())
    }

    pub fn try_from_raw_with_limits(
        raw: RawContract,
        limits: &Limits,
    ) -> Result<Self, ContractValidationError> {
        Self::from_raw_with_limits(raw, limits)
    }

    /// Validate the raw graph structure, canonicalize it, and calculate fresh
    /// identities. This producer API intentionally ignores supplied identity
    /// values; trust boundaries should use [`Self::try_from_raw`] instead.
    pub fn build_raw(raw: RawContract, limits: &Limits) -> Result<Self, ContractValidationError> {
        let mut contract = Self::new_unchecked(raw.types, raw.declarations, raw.actor);
        contract.format = raw.format;
        contract.format_version = raw.format_version;
        contract.semantics_profile = raw.semantics_profile;
        contract.canonicalization_profile = raw.canonicalization_profile;
        contract.contract_version = raw.contract_version;
        contract.producer = raw.producer;
        crate::validate::validate_structure_with_limits(&contract, limits)?;
        crate::canonical::canonicalize_contract_with_limits(&contract, limits)
    }

    fn from_raw_with_limits(
        raw: RawContract,
        limits: &Limits,
    ) -> Result<Self, ContractValidationError> {
        Ok(Self::from_raw_with_mapping(raw, limits)?.0)
    }

    pub(crate) fn from_raw_with_mapping(
        raw: RawContract,
        limits: &Limits,
    ) -> Result<(Self, Vec<TypeRef>), ContractValidationError> {
        let contract = Self {
            format: raw.format,
            format_version: raw.format_version,
            semantics_profile: raw.semantics_profile,
            canonicalization_profile: raw.canonicalization_profile,
            contract_version: raw.contract_version,
            identities: raw.identities,
            fingerprint: raw.fingerprint,
            producer: raw.producer,
            types: raw.types,
            declarations: raw.declarations,
            actor: raw.actor,
        };
        contract.validate_with_limits(limits)?;
        let canonicalized =
            crate::canonical::canonicalize_with_mapping_unchecked_and_limits(&contract, limits)?;
        Ok((canonicalized.contract, canonicalized.old_to_new))
    }

    pub(crate) fn new_unchecked(
        types: Vec<TypeNode>,
        declarations: Vec<Declaration>,
        actor: Option<Actor>,
    ) -> Self {
        Self {
            format: CONTRACT_FORMAT.to_string(),
            format_version: FORMAT_VERSION,
            semantics_profile: SEMANTICS_PROFILE.to_string(),
            canonicalization_profile: CANONICALIZATION_PROFILE.to_string(),
            contract_version: CONTRACT_VERSION,
            identities: ContractIdentities {
                contract: format!("ccr:contract:v1:sha256:{}", "0".repeat(64)),
                interface: actor
                    .as_ref()
                    .map(|_| format!("ccr:interface:v1:sha256:{}", "0".repeat(64))),
            },
            fingerprint: format!("sha256:{}", "0".repeat(64)),
            producer: ProducerInfo::current(),
            types,
            declarations,
            actor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawContract {
    pub format: String,
    pub format_version: u32,
    pub semantics_profile: String,
    pub canonicalization_profile: String,
    pub contract_version: u32,
    pub identities: ContractIdentities,
    pub fingerprint: String,
    pub producer: ProducerInfo,
    pub types: Vec<TypeNode>,
    #[serde(default)]
    pub declarations: Vec<Declaration>,
    #[serde(default)]
    pub actor: Option<Actor>,
}

impl RawContract {
    pub fn new(types: Vec<TypeNode>, declarations: Vec<Declaration>, actor: Option<Actor>) -> Self {
        Self::from(&Contract::new_unchecked(types, declarations, actor))
    }
}

impl From<&Contract> for RawContract {
    fn from(contract: &Contract) -> Self {
        Self {
            format: contract.format.clone(),
            format_version: contract.format_version,
            semantics_profile: contract.semantics_profile.clone(),
            canonicalization_profile: contract.canonicalization_profile.clone(),
            contract_version: contract.contract_version,
            identities: contract.identities.clone(),
            fingerprint: contract.fingerprint.clone(),
            producer: contract.producer.clone(),
            types: contract.types.clone(),
            declarations: contract.declarations.clone(),
            actor: contract.actor.clone(),
        }
    }
}

impl Serialize for Contract {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        RawContract::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Contract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawContract::deserialize(deserializer)?;
        Self::try_from_raw(raw).map_err(D::Error::custom)
    }
}

impl TryFrom<RawContract> for Contract {
    type Error = ContractValidationError;

    fn try_from(raw: RawContract) -> Result<Self, Self::Error> {
        Self::try_from_raw(raw)
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo {
    pub(crate) source_info_version: u32,
    pub(crate) contract_id: String,
    pub(crate) source_bundle_id: String,
    pub(crate) sources: Vec<SourceFileInfo>,
    #[serde(default)]
    pub(crate) imports: Vec<SourceImportInfo>,
    #[serde(default)]
    pub(crate) declarations: Vec<SourceDeclaration>,
    #[serde(default)]
    pub(crate) field_labels: Vec<FieldLabelProvenance>,
    #[serde(default)]
    pub(crate) methods: Vec<SourceMethodInfo>,
    #[serde(default)]
    pub(crate) function_arguments: Vec<SourceFunctionArgumentInfo>,
    #[serde(default)]
    pub(crate) actors: Vec<SourceActorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSourceInfo {
    pub source_info_version: u32,
    pub contract_id: String,
    pub source_bundle_id: String,
    pub sources: Vec<SourceFileInfo>,
    #[serde(default)]
    pub imports: Vec<SourceImportInfo>,
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

impl From<RawSourceInfo> for SourceInfo {
    fn from(raw: RawSourceInfo) -> Self {
        Self {
            source_info_version: raw.source_info_version,
            contract_id: raw.contract_id,
            source_bundle_id: raw.source_bundle_id,
            sources: raw.sources,
            imports: raw.imports,
            declarations: raw.declarations,
            field_labels: raw.field_labels,
            methods: raw.methods,
            function_arguments: raw.function_arguments,
            actors: raw.actors,
        }
    }
}

impl SourceInfo {
    pub fn source_info_version(&self) -> u32 {
        self.source_info_version
    }

    pub fn contract_id(&self) -> &str {
        &self.contract_id
    }

    pub fn source_bundle_id(&self) -> &str {
        &self.source_bundle_id
    }

    pub fn sources(&self) -> &[SourceFileInfo] {
        &self.sources
    }

    pub fn imports(&self) -> &[SourceImportInfo] {
        &self.imports
    }

    pub fn declarations(&self) -> &[SourceDeclaration] {
        &self.declarations
    }

    pub fn field_labels(&self) -> &[FieldLabelProvenance] {
        &self.field_labels
    }

    pub fn methods(&self) -> &[SourceMethodInfo] {
        &self.methods
    }

    pub fn function_arguments(&self) -> &[SourceFunctionArgumentInfo] {
        &self.function_arguments
    }

    pub fn actors(&self) -> &[SourceActorInfo] {
        &self.actors
    }

    pub fn validate(
        &self,
        contract: &Contract,
        limits: &Limits,
    ) -> Result<(), ContractValidationError> {
        crate::source::validate_source_info(self, contract, limits)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFileInfo {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceImportKind {
    Type,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImportInfo {
    pub from: String,
    pub import: String,
    pub to: String,
    pub kind: SourceImportKind,
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
