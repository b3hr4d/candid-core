use super::type_graph::{Actor, Declaration, TypeNode, TypeRef};
use super::validation_error::{ContractJsonError, ContractValidationError};
use crate::limits::Limits;
use serde::{Deserialize, Serialize, Serializer};

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

/// Untrusted, caller-supplied provenance about the tool that produced a
/// Contract.
///
/// Producer metadata is deliberately **outside authenticated Contract
/// identity**. The `candid-core:contract:v1` and `candid-core:interface:v1`
/// hashes ([`crate::canonical`]) are computed over the type graph, declarations,
/// actor, and format/profile markers only — never over `producer`. Two
/// Contracts that differ only in their producer therefore share the same
/// `contract_id` and `interface_id`, even though they are byte-different on the
/// wire (producer *is* part of the canonical serialized JSON). A signature that
/// authenticates a Contract by its identity does not authenticate its producer
/// claims, and callers must treat those claims as unverified.
///
/// This boundary is load-bearing for compatibility: binding `producer` into the
/// identity payload would change every existing `contract_id`. The bytes are
/// still bounded — see [`crate::Limits::max_producer_bytes`] — so untrusted
/// producer strings cannot grow without limit; they simply never influence an
/// identity.
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

    /// Serialize validated canonical JSON under [`Limits::default`].
    ///
    /// A Contract built with raised limits can fail here. Use
    /// [`Self::to_json_pretty_with_limits`] or
    /// [`Self::to_json_pretty_with_context`] to serialize under the caller's
    /// own policy.
    pub fn to_json_pretty(&self) -> Result<String, ContractValidationError> {
        self.to_json_pretty_with_context(&crate::RuntimeContext::default())
    }

    /// Serialize validated canonical JSON under caller-supplied limits.
    ///
    /// This revalidates and recanonicalizes, so it consumes the structural
    /// limits construction consumed *and* charges the rendered length against
    /// `max_canonicalization_work`. Raising only the limit that gated
    /// construction is therefore not always sufficient; see
    /// [`Self::to_json_pretty_with_context`].
    pub fn to_json_pretty_with_limits(
        &self,
        limits: &Limits,
    ) -> Result<String, ContractValidationError> {
        self.to_json_pretty_with_context(&crate::RuntimeContext::new(limits.clone()))
    }

    /// Serialize validated canonical JSON under the caller's context.
    ///
    /// Consumes two distinct budgets: the structural limits that gated
    /// construction, and `max_canonicalization_work`, against which the
    /// rendered byte length is charged. A caller who raised only a structural
    /// limit (for example `max_string_bytes`) to build the Contract may still
    /// need to raise `max_canonicalization_work` to render it.
    ///
    /// For a completely unbounded render, `serde` [`Serialize`] is implemented
    /// on [`Contract`] directly; it consults no limits and performs no
    /// revalidation. That path is for trusted, already-validated values.
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

    /// Parse, validate, and canonicalize a Contract JSON document under
    /// [`Limits::default`].
    pub fn from_json(input: &str) -> Result<Self, ContractJsonError> {
        Self::from_json_with_limits(input, &Limits::default())
    }

    pub fn from_json_with_limits(input: &str, limits: &Limits) -> Result<Self, ContractJsonError> {
        Self::from_json_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    /// Bounded parse: `max_input_bytes` is enforced before the document is
    /// decoded, and decode and validation share one budget.
    pub fn from_json_with_context(
        input: &str,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractJsonError> {
        let mut budget = context.budget();
        let raw: RawContract = crate::budget::decode_bounded(&mut budget, input.len(), || {
            serde_json::from_str(input)
        })?;
        Self::from_raw_with_mapping_and_budget(raw, &mut budget)
            .map(|(contract, _)| contract)
            .map_err(ContractJsonError::InvalidContract)
    }

    /// Parse, validate, and canonicalize Contract JSON bytes under
    /// caller-supplied limits.
    pub fn from_slice_with_limits(
        input: &[u8],
        limits: &Limits,
    ) -> Result<Self, ContractJsonError> {
        Self::from_slice_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    /// Bounded parse from bytes. Equivalent to [`Self::from_json_with_context`]
    /// without requiring the caller to validate UTF-8 first.
    pub fn from_slice_with_context(
        input: &[u8],
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractJsonError> {
        let mut budget = context.budget();
        let raw: RawContract = crate::budget::decode_bounded(&mut budget, input.len(), || {
            serde_json::from_slice(input)
        })?;
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

/// Unvalidated Contract data.
///
/// This is the serde entry point for Contract JSON. [`Contract`] itself
/// deliberately does not implement [`Deserialize`]: a trait impl has no
/// argument position for a resource policy, so it could only ever decode under
/// limits the library chose. Callers decode this DTO and convert through a
/// policy-taking constructor such as [`Contract::try_from_raw_with_context`],
/// or use a bounded parse entry point such as
/// [`Contract::from_json_with_context`], which enforces `max_input_bytes`
/// before decoding.
///
/// Decoding this DTO is *not* a trust boundary and carries no allocation
/// bound; gate the byte length yourself, or use the bounded parse APIs.
///
/// ```compile_fail
/// // A validated Contract cannot be produced by serde alone.
/// let _: candid_core::Contract = serde_json::from_str("{}").unwrap();
/// ```
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
    /// An actorless Contract omits this property entirely. `"actor": null` is
    /// not part of the v1 wire format: serialization never emits it, the
    /// identity payload never hashes it, and decoding rejects it.
    #[serde(
        default,
        deserialize_with = "deserialize_actor_forbidding_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub actor: Option<Actor>,
}

/// Invoked only when the `actor` key is present; an absent key takes the
/// `None` default. Delegating to [`Actor`] directly makes an explicit JSON
/// `null` a decode error instead of a second spelling of "no actor".
fn deserialize_actor_forbidding_null<'de, D>(deserializer: D) -> Result<Option<Actor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Actor::deserialize(deserializer).map(Some)
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
