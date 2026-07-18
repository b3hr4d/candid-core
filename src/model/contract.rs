use super::type_graph::{Actor, Declaration, TypeNode, TypeRef};
use super::validation_error::{ContractJsonError, ContractValidationError};
use crate::limits::Limits;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

pub const CONTRACT_FORMAT: &str = "candid-core";
pub const FORMAT_VERSION: u32 = 1;
pub const SEMANTICS_PROFILE: &str = "candid-1";
pub const CANONICALIZATION_PROFILE: &str = "candid-core-canon-1";
const PACKAGE_MANIFEST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));

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
            candid_version: exact_dependency_version(PACKAGE_MANIFEST, "candid"),
            candid_parser_version: exact_dependency_version(PACKAGE_MANIFEST, "candid_parser"),
        }
    }
}

fn exact_dependency_version(manifest: &str, dependency: &str) -> String {
    let prefix = format!("{dependency} = ");
    manifest
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_prefix('='))
                .and_then(|value| value.split_once('"').map(|(version, _)| version))
        })
        .unwrap_or_else(|| {
            panic!("{dependency} must be declared as an exact string dependency in Cargo.toml")
        })
        .to_string()
}

/// The wire-semantics Contract consumed by host runtimes.
///
/// `declarations` supplies named roots for the arena. Comments, source spelling,
/// and raw source are kept in [`crate::SourceInfo`], not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub(crate) format: String,
    pub(crate) format_version: u32,
    pub(crate) semantics_profile: String,
    pub(crate) canonicalization_profile: String,
    pub(crate) identities: ContractIdentities,
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

    pub fn identities(&self) -> &ContractIdentities {
        &self.identities
    }

    pub fn contract_id(&self) -> &str {
        &self.identities.contract
    }

    pub fn interface_id(&self) -> Option<&str> {
        self.identities.interface.as_deref()
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

    /// Validate graph structure and verify its content identities.
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        self.validate_with_limits(&Limits::default())
    }

    pub fn validate_with_limits(&self, limits: &Limits) -> Result<(), ContractValidationError> {
        self.validate_with_context(&crate::RuntimeContext::new(limits.clone()))
    }

    pub fn validate_with_context(
        &self,
        context: &crate::RuntimeContext,
    ) -> Result<(), ContractValidationError> {
        let mut budget = context.budget();
        crate::validate::validate_contract_with_budget(self, &mut budget)
    }

    /// Return a deterministically re-indexed copy with freshly calculated
    /// identities. This is useful at JSON trust boundaries.
    pub fn canonicalize(&self) -> Result<Self, ContractValidationError> {
        self.canonicalize_with_limits(&Limits::default())
    }

    pub fn canonicalize_with_limits(
        &self,
        limits: &Limits,
    ) -> Result<Self, ContractValidationError> {
        self.canonicalize_with_context(&crate::RuntimeContext::new(limits.clone()))
    }

    pub fn canonicalize_with_context(
        &self,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractValidationError> {
        let mut budget = context.budget();
        crate::canonical::canonicalize_contract_with_budget(self, &mut budget)
    }

    /// Serialize validated canonical JSON.
    pub fn to_json_pretty(&self) -> Result<String, ContractValidationError> {
        self.to_json_pretty_with_context(&crate::RuntimeContext::default())
    }

    pub fn to_json_pretty_with_context(
        &self,
        context: &crate::RuntimeContext,
    ) -> Result<String, ContractValidationError> {
        let mut budget = context.budget();
        let canonical =
            crate::validate::validate_and_canonicalize_with_budget(self, &mut budget)?.contract;
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        let json = serde_json::to_string_pretty(&canonical).map_err(|error| {
            ContractValidationError::single(
                "contract_json_serialization_failed",
                "$",
                error.to_string(),
            )
        })?;
        let max_work = budget.limits().max_canonicalization_work;
        budget
            .charge("canonicalization_work", max_work, json.len())
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        Ok(json)
    }

    /// Parse, validate, and canonicalize a Contract JSON document.
    pub fn from_json(input: &str) -> Result<Self, ContractJsonError> {
        Self::from_json_with_limits(input, &Limits::default())
    }

    pub fn from_json_with_limits(input: &str, limits: &Limits) -> Result<Self, ContractJsonError> {
        Self::from_json_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    pub fn from_json_with_context(
        input: &str,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractJsonError> {
        let limits = &context.limits;
        if input.len() > limits.max_input_bytes {
            return Err(ContractJsonError::InvalidContract(
                ContractValidationError::resource_limit(
                    "input_bytes",
                    limits.max_input_bytes,
                    input.len(),
                ),
            ));
        }
        let mut budget = context.budget();
        budget
            .observe("input_bytes", limits.max_input_bytes, input.len())
            .map_err(crate::budget::BudgetError::into_contract_error)
            .map_err(ContractJsonError::InvalidContract)?;
        let raw: RawContract = serde_json::from_str(input)
            .map_err(|error| ContractJsonError::MalformedJson(error.to_string()))?;
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)
            .map_err(ContractJsonError::InvalidContract)?;
        Self::from_raw_with_mapping_and_budget(raw, &mut budget)
            .map(|(contract, _)| contract)
            .map_err(ContractJsonError::InvalidContract)
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

    pub fn try_from_raw_with_context(
        raw: RawContract,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractValidationError> {
        let mut budget = context.budget();
        Ok(Self::from_raw_with_mapping_and_budget(raw, &mut budget)?.0)
    }

    /// Validate the raw graph structure, canonicalize it, and calculate fresh
    /// identities. This producer API intentionally ignores supplied identity
    /// values; trust boundaries should use [`Self::try_from_raw`] instead.
    pub fn build_raw(raw: RawContract, limits: &Limits) -> Result<Self, ContractValidationError> {
        let mut budget = crate::budget::Budget::from_limits(limits);
        Self::build_raw_with_budget(raw, &mut budget)
    }

    pub fn build_raw_with_context(
        raw: RawContract,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractValidationError> {
        let mut budget = context.budget();
        Self::build_raw_with_budget(raw, &mut budget)
    }

    pub(crate) fn build_raw_with_budget(
        raw: RawContract,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<Self, ContractValidationError> {
        let mut contract = Self::new_unchecked(raw.types, raw.declarations, raw.actor);
        contract.format = raw.format;
        contract.format_version = raw.format_version;
        contract.semantics_profile = raw.semantics_profile;
        contract.canonicalization_profile = raw.canonicalization_profile;
        contract.producer = raw.producer;
        crate::validate::validate_structure_with_budget(&contract, budget)?;
        Ok(
            crate::canonical::canonicalize_with_mapping_unchecked_with_budget(&contract, budget)?
                .contract,
        )
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
        let mut budget = crate::budget::Budget::from_limits(limits);
        Self::from_raw_with_mapping_and_budget(raw, &mut budget)
    }

    pub(crate) fn from_raw_with_mapping_and_budget(
        raw: RawContract,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<(Self, Vec<TypeRef>), ContractValidationError> {
        let contract = Self {
            format: raw.format,
            format_version: raw.format_version,
            semantics_profile: raw.semantics_profile,
            canonicalization_profile: raw.canonicalization_profile,
            identities: raw.identities,
            producer: raw.producer,
            types: raw.types,
            declarations: raw.declarations,
            actor: raw.actor,
        };
        let canonicalized =
            crate::validate::validate_and_canonicalize_with_budget(&contract, budget)?;
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
            identities: ContractIdentities {
                contract: format!("candid-core:contract:v1:sha256:{}", "0".repeat(64)),
                interface: actor
                    .as_ref()
                    .map(|_| format!("candid-core:interface:v1:sha256:{}", "0".repeat(64))),
            },
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
    pub identities: ContractIdentities,
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
            identities: contract.identities.clone(),
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
