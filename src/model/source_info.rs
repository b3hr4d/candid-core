use super::contract::Contract;
use super::type_graph::TypeRef;
use super::validation_error::ContractValidationError;
use crate::limits::Limits;
use serde::{Deserialize, Serialize};

pub const SOURCE_INFO_VERSION: u32 = 1;

/// Optional source/provenance data returned alongside, but never embedded in,
/// the canonical Contract. It has no effect on Contract or interface identity.
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
/// Unvalidated source/provenance data.
///
/// Raw data cannot be converted infallibly into validated [`SourceInfo`]:
///
/// ```compile_fail
/// use candid_core::{RawSourceInfo, SourceInfo};
///
/// fn bypass_validation(raw: RawSourceInfo) -> SourceInfo {
///     raw.into()
/// }
/// ```
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

impl SourceInfo {
    pub(crate) fn from_raw_unchecked(raw: RawSourceInfo) -> Self {
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

    /// Recompiles the embedded source bundle and validates that every presented
    /// provenance field matches the compiler-derived sidecar for `contract`.
    pub fn try_from_raw(
        raw: RawSourceInfo,
        contract: &Contract,
        limits: &Limits,
    ) -> Result<Self, ContractValidationError> {
        let source_info = Self::from_raw_unchecked(raw);
        source_info.validate(contract, limits)?;
        Ok(source_info)
    }

    pub fn try_from_raw_with_context(
        raw: RawSourceInfo,
        contract: &Contract,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractValidationError> {
        let source_info = Self::from_raw_unchecked(raw);
        let mut budget = context.budget();
        source_info.validate_with_budget(contract, &mut budget)?;
        Ok(source_info)
    }

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

    pub(crate) fn validate_with_budget(
        &self,
        contract: &Contract,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<(), ContractValidationError> {
        crate::source::validate_source_info_with_budget(self, contract, budget)
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
