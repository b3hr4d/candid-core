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
/// hashes (see `docs/canonicalization-v1.md`) are computed over the type
/// graph, declarations, actor, and format/profile markers only — never over
/// `producer`. Two Contracts that differ only in their producer therefore
/// share the same `contract_id` and `interface_id`, even though they are
/// byte-different on the wire (producer *is* part of the canonical serialized
/// JSON — and identical in every feature configuration). A signature that
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
    /// The producer metadata describing this build of `candid-core` itself:
    /// the crate name and version plus the exact pinned `candid` and
    /// `candid_parser` dependency versions. This is the producer a
    /// [`ContractDraft`] builds with when none is supplied explicitly.
    ///
    /// The four fields are present and identical in every feature
    /// configuration. `candid` and `candid_parser` are optional dependencies
    /// enabled by the `compiler` feature, but the versions reported here are
    /// read out of this package's own manifest text at compile time, not from
    /// a linked crate, so a `default-features = false` build of a given
    /// `candid-core` version reports exactly the same producer bytes as a full
    /// one. That is deliberate: producer metadata is untrusted provenance
    /// about *this package*, and making it vary by feature would fork the
    /// serialized shape of otherwise identical Contracts.
    pub fn current() -> Self {
        Self {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            candid_version: exact_dependency_version(PACKAGE_MANIFEST, "candid"),
            candid_parser_version: exact_dependency_version(PACKAGE_MANIFEST, "candid_parser"),
        }
    }
}

fn exact_dependency_version(manifest: &str, dependency: &str) -> String {
    manifest_dependency_version(manifest, dependency).unwrap_or_else(|| {
        panic!("{dependency} must be declared as an exact dependency in Cargo.toml")
    })
}

/// Find `dependency`'s pinned version in a Cargo manifest, in every spelling
/// this crate's manifest can legitimately take.
///
/// Three forms have to be accepted, and the reason is not stylistic:
///
/// * `dep = "=X.Y.Z"` — the plain form;
/// * `dep = { version = "=X.Y.Z", optional = true }` — the form an *optional*
///   dependency is required to use, which is what `candid`/`candid_parser`
///   became when they moved behind the `compiler` feature;
/// * a `[dependencies.dep]` section with a `version = "=X.Y.Z"` line — which
///   is what **Cargo itself writes** when it normalizes a manifest for
///   publishing. A build from crates.io reads that normalized file through
///   `CARGO_MANIFEST_DIR`, not the manifest in this repository, so a reader
///   that only understood the first two forms would panic inside
///   [`ProducerInfo::current`] in exactly the builds that matter most.
///
/// The scan is section-aware so `[dev-dependencies.candid_parser]` can never be
/// mistaken for the real dependency, and a non-exact requirement still yields
/// `None`: the pin is what makes the reported version meaningful.
fn manifest_dependency_version(manifest: &str, dependency: &str) -> Option<String> {
    let inline_prefix = format!("{dependency} = ");
    let section_header = format!("[dependencies.{dependency}]");
    let mut in_dependencies = false;
    let mut in_dependency_section = false;

    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
            in_dependency_section = line == section_header;
        } else if in_dependencies {
            if let Some(version) = line
                .strip_prefix(&inline_prefix)
                .and_then(exact_version_literal)
            {
                return Some(version.to_string());
            }
        } else if in_dependency_section {
            if let Some(value) = line.strip_prefix("version = ") {
                return exact_version_literal(value).map(str::to_string);
            }
        }
    }
    None
}

/// Read the `=X.Y.Z` text out of a value that is either a bare string literal
/// or an inline table containing a `version` key.
fn exact_version_literal(declaration: &str) -> Option<&str> {
    let literal = match declaration.strip_prefix('{') {
        Some(table) => table.split_once("version = ")?.1,
        None => declaration,
    };
    literal
        .strip_prefix('"')
        .and_then(|value| value.strip_prefix('='))
        .and_then(|value| value.split_once('"').map(|(version, _)| version))
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    /// The manifest this build actually compiled against must answer, whatever
    /// spelling it uses — that is the invariant `ProducerInfo::current` rests
    /// on.
    #[test]
    fn the_packages_own_manifest_reports_both_engine_versions() {
        assert!(!exact_dependency_version(PACKAGE_MANIFEST, "candid").is_empty());
        assert!(!exact_dependency_version(PACKAGE_MANIFEST, "candid_parser").is_empty());
    }

    #[test]
    fn every_spelling_cargo_can_produce_is_read_identically() {
        let plain = "[dependencies]\ncandid = \"=0.10.30\"\n";
        let inline_table = "[dependencies]\ncandid = { version = \"=0.10.30\", optional = true }\n";
        // Exactly what `cargo package` writes into the published manifest.
        let normalized = "[dependencies.candid]\nversion = \"=0.10.30\"\noptional = true\n";
        for manifest in [plain, inline_table, normalized] {
            assert_eq!(exact_dependency_version(manifest, "candid"), "0.10.30");
        }
    }

    #[test]
    fn a_dev_dependency_is_never_mistaken_for_the_real_one() {
        // Both sections exist in this package's normalized manifest, and the
        // dev one may legitimately carry a different requirement.
        let manifest = "[dependencies.candid_parser]\nversion = \"=0.4.0\"\noptional = true\n\
                        \n[dev-dependencies.candid_parser]\nversion = \"=0.9.9\"\n";
        assert_eq!(exact_dependency_version(manifest, "candid_parser"), "0.4.0");
        // A dev-only declaration is not an answer at all.
        assert_eq!(
            manifest_dependency_version(
                "[dev-dependencies.candid_parser]\nversion = \"=0.4.0\"\n",
                "candid_parser"
            ),
            None
        );
    }

    #[test]
    fn a_floating_requirement_is_not_an_exact_pin() {
        for manifest in [
            "[dependencies]\ncandid = \"0.10.30\"\n",
            "[dependencies]\ncandid = { version = \"^0.10\" }\n",
            "[dependencies.candid]\nversion = \"0.10.30\"\n",
        ] {
            assert_eq!(manifest_dependency_version(manifest, "candid"), None);
        }
    }
}

/// The wire-semantics Contract consumed by host runtimes.
///
/// `declarations` supplies named roots for the arena. Comments, source spelling,
/// and raw source are kept in the `SourceInfo` sidecar (`compiler` feature),
/// not here.
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

/// A producer-side Contract draft: the parts an authoring tool supplies, and
/// nothing it must not.
///
/// A draft carries only the type graph, named declarations, an optional
/// actor, and optional producer metadata. It deliberately has **no**
/// format/version/profile markers and **no** identity fields: building stamps
/// the current [`CONTRACT_FORMAT`]/[`FORMAT_VERSION`]/[`SEMANTICS_PROFILE`]/
/// [`CANONICALIZATION_PROFILE`] constants and calculates fresh identities
/// under the same validation and canonicalization budgets as every other
/// entry point, so a draft can never carry a fake, stale, or placeholder
/// identity. [`RawContract`] is the opposite boundary — the serde DTO for
/// *decoded external* artifacts, whose supplied identities
/// [`Contract::try_from_raw`] verifies instead of recalculating.
///
/// # Serialized shape
///
/// A serialized draft contains exactly the four fields above. Unknown keys
/// are rejected; `declarations` defaults to empty when absent; `actor` is
/// omitted when absent, and an explicit `"actor": null` is rejected just as
/// [`RawContract`] rejects it; `producer` is omitted when absent and defaults
/// at build time to [`ProducerInfo::current`], while a present `producer`
/// overrides that default.
///
/// ```
/// use candid_core::{ContractDraft, PrimitiveType, TypeNode};
///
/// let contract = ContractDraft::new(
///     vec![TypeNode::Primitive { primitive: PrimitiveType::Nat }],
///     vec![candid_core::Declaration { name: "Amount".to_string(), ty: 0 }],
///     None,
/// )
/// .build()?;
/// assert!(contract.contract_id().starts_with("candid-core:contract:v1:sha256:"));
/// assert_eq!(contract.producer(), &candid_core::ProducerInfo::current());
/// # Ok::<(), candid_core::ContractValidationError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractDraft {
    pub types: Vec<TypeNode>,
    #[serde(default)]
    pub declarations: Vec<Declaration>,
    /// An actorless draft omits this property entirely; `"actor": null` is
    /// rejected on decode, exactly as [`RawContract`] rejects it.
    #[serde(
        default,
        deserialize_with = "deserialize_actor_forbidding_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub actor: Option<Actor>,
    /// Untrusted provenance about the authoring tool. `None` builds with
    /// [`ProducerInfo::current`]. Never part of authenticated identity; see
    /// [`ProducerInfo`]. An explicit `"producer": null` is rejected on
    /// decode: absence is the only spelling of "default producer", mirroring
    /// the `actor` rule.
    #[serde(
        default,
        deserialize_with = "deserialize_producer_forbidding_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub producer: Option<ProducerInfo>,
}

/// Invoked only when the `producer` key is present; an absent key takes the
/// `None` default. Delegating to [`ProducerInfo`] directly makes an explicit
/// JSON `null` a decode error instead of a second spelling of "default
/// producer".
fn deserialize_producer_forbidding_null<'de, D>(
    deserializer: D,
) -> Result<Option<ProducerInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    ProducerInfo::deserialize(deserializer).map(Some)
}

impl ContractDraft {
    pub fn new(types: Vec<TypeNode>, declarations: Vec<Declaration>, actor: Option<Actor>) -> Self {
        Self {
            types,
            declarations,
            actor,
            producer: None,
        }
    }

    /// Returns `self` with an explicit producer, overriding the
    /// [`ProducerInfo::current`] default applied at build time.
    #[must_use]
    pub fn with_producer(mut self, producer: ProducerInfo) -> Self {
        self.producer = Some(producer);
        self
    }

    /// Validate the draft graph, canonicalize it, and calculate its
    /// identities under [`Limits::default`].
    pub fn build(self) -> Result<Contract, ContractValidationError> {
        self.build_with_limits(&Limits::default())
    }

    /// Build under caller-supplied limits.
    pub fn build_with_limits(self, limits: &Limits) -> Result<Contract, ContractValidationError> {
        let mut budget = crate::budget::Budget::from_limits(limits);
        self.build_with_budget(&mut budget)
    }

    /// Build under the caller's context, sharing its budget, deadline, and
    /// cancellation token.
    pub fn build_with_context(
        self,
        context: &crate::RuntimeContext,
    ) -> Result<Contract, ContractValidationError> {
        let mut budget = context.budget();
        self.build_with_budget(&mut budget)
    }

    fn build_with_budget(
        self,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<Contract, ContractValidationError> {
        let mut contract = Contract::new_unchecked(self.types, self.declarations, self.actor);
        if let Some(producer) = self.producer {
            contract.producer = producer;
        }
        crate::validate::validate_structure_with_budget(&contract, budget)?;
        Ok(
            crate::canonical::canonicalize_with_mapping_unchecked_with_budget(&contract, budget)?
                .contract,
        )
    }
}

/// Unvalidated Contract data decoded from an external artifact.
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
/// This type is reserved for artifacts that already carry format markers and
/// identities: [`Contract::try_from_raw`] verifies the supplied identities
/// against recomputation, and [`From<&Contract>`] projects a validated
/// Contract back onto the wire shape. To *author* a Contract — where no
/// trustworthy identity exists yet — use [`ContractDraft`], which carries no
/// identity fields at all and calculates them on build.
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
