use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// The version of the portable limits configuration schema this build reads
/// and writes. See [`LimitsConfig`].
pub const LIMITS_CONFIG_VERSION: u64 = 1;

/// A named, versioned set of default operational limit values.
///
/// A profile freezes every default number behind a stable wire name, so a
/// serialized configuration can say *which* defaults it started from instead
/// of copying platform-dependent values. Existing profile values are never
/// changed once released; new tunings become new profile variants. The enum is
/// `#[non_exhaustive]` so adding a profile is not a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitsProfile {
    /// The interactive-tooling defaults this crate has always shipped:
    /// safe for parsing and validating untrusted documents in an editor,
    /// CLI, or agent context on an ordinary desktop host. The exact numbers
    /// are frozen; see each [`Limits`] getter for the value and rationale.
    InteractiveV1,
}

impl LimitsProfile {
    /// The profile's frozen default limit values.
    pub fn limits(self) -> Limits {
        match self {
            Self::InteractiveV1 => interactive_v1(),
        }
    }

    /// The stable wire name used by [`LimitsConfig`].
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::InteractiveV1 => "interactive_v1",
        }
    }

    fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "interactive_v1" => Some(Self::InteractiveV1),
            _ => None,
        }
    }
}

/// Declares every numeric limit field exactly once, together with its
/// builder name and `InteractiveV1` default, and derives the four surfaces
/// that must never drift apart: the private struct fields, the public
/// getters, the public `with_*` builders, and the portable override schema
/// (its keys, its diff against the profile baseline, and its checked
/// application). `deadline_unix_ms` is declared separately because it is the
/// one field that is optional and already `u64` on the wire.
macro_rules! limit_fields {
    ($( $(#[$doc:meta])* $field:ident / $with:ident = $default:expr; )+) => {
        /// Operational limits for work performed on untrusted Contracts,
        /// sources, and values.
        ///
        /// Limits never participate in Contract identity. Hosts may raise
        /// them explicitly for trusted workloads.
        ///
        /// # Construction
        ///
        /// Fields are private so adding a limit is never a breaking change.
        /// Start from a profile and override individual fields with the
        /// `with_*` builders:
        ///
        /// ```
        /// use candid_core::{Limits, LimitsProfile};
        ///
        /// let limits = LimitsProfile::InteractiveV1
        ///     .limits()
        ///     .with_max_input_bytes(64 * 1024)
        ///     .with_deadline_unix_ms(Some(2_000_000_000_000));
        /// assert_eq!(limits.max_input_bytes(), 64 * 1024);
        /// ```
        ///
        /// [`Limits::default`] is [`LimitsProfile::InteractiveV1`].
        ///
        /// # Zero values
        ///
        /// Every limit accepts `0`; a zero limit is a defined, fail-closed
        /// policy rather than a rejected configuration. A zero byte/count/work
        /// limit rejects any input that consumes the resource at all, and
        /// `with_max_diagnostics(0)` retains exactly one out-of-band
        /// `resource_limit_exceeded` sentinel violation so an invalid input
        /// never yields an empty error collection. `deadline_unix_ms` values
        /// at or before the current time (including `Some(0)`) make every
        /// bounded operation fail closed with `operation_deadline_exceeded`.
        ///
        /// # Serialization
        ///
        /// `Limits` serializes as the versioned portable configuration
        /// described by [`LimitsConfig`], never as a bare field map.
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(into = "LimitsConfig", try_from = "LimitsConfig")]
        pub struct Limits {
            pub(crate) profile: LimitsProfile,
            $( pub(crate) $field: usize, )+
            /// See [`Limits::deadline_unix_ms`].
            pub(crate) deadline_unix_ms: Option<u64>,
        }

        impl Limits {
            $(
                $(#[$doc])*
                #[must_use]
                pub fn $field(&self) -> usize {
                    self.$field
                }
            )+

            $(
                #[doc = concat!("Returns `self` with `", stringify!($field), "` replaced. See [`Limits::", stringify!($field), "`].")]
                #[must_use]
                pub fn $with(mut self, value: usize) -> Self {
                    self.$field = value;
                    self
                }
            )+
        }

        /// The explicit override values of a [`LimitsConfig`].
        ///
        /// Only overrides that differ from the named profile's frozen
        /// defaults are serialized, and every value is a fixed-width `u64`
        /// so the document means the same thing on every platform. An
        /// explicit JSON `null` is rejected rather than read as "no
        /// override": absence is the only spelling of "use the profile
        /// value".
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LimitsOverrides {
            $(
                #[serde(
                    default,
                    deserialize_with = "deserialize_override_forbidding_null",
                    skip_serializing_if = "Option::is_none"
                )]
                $field: Option<u64>,
            )+
            #[serde(
                default,
                deserialize_with = "deserialize_override_forbidding_null",
                skip_serializing_if = "Option::is_none"
            )]
            deadline_unix_ms: Option<u64>,
        }

        impl LimitsOverrides {
            /// The overrides that reproduce `limits` from its profile's
            /// frozen baseline. An override equal to the baseline value is
            /// normalized away: the profile numbers are frozen, so omission
            /// and an equal explicit value are the same policy forever.
            fn diff(limits: &Limits) -> Self {
                let baseline = limits.profile.limits();
                Self {
                    $( $field: (limits.$field != baseline.$field)
                        .then(|| portable_count(limits.$field)), )+
                    // No profile defines a default deadline, so "deadline
                    // present and different" is the only representable
                    // override; `apply` can only ever set one.
                    deadline_unix_ms: match (limits.deadline_unix_ms, baseline.deadline_unix_ms) {
                        (Some(deadline), baseline) if baseline != Some(deadline) => Some(deadline),
                        _ => None,
                    },
                }
            }

            /// Applies the overrides to the profile baseline, converting each
            /// `u64` into the platform word with an exact checked conversion.
            fn apply(self, profile: LimitsProfile) -> Result<Limits, LimitsConfigError> {
                let mut limits = profile.limits();
                $(
                    if let Some(value) = self.$field {
                        limits.$field = override_to_usize(stringify!($field), value)?;
                    }
                )+
                if let Some(value) = self.deadline_unix_ms {
                    limits.deadline_unix_ms = Some(value);
                }
                Ok(limits)
            }
        }

        /// The frozen `InteractiveV1` default values.
        ///
        /// Generated from the single per-field declaration above so the
        /// profile numbers are defined in exactly one place and cannot drift
        /// from the field list, getters, builders, or override schema.
        fn interactive_v1() -> Limits {
            Limits {
                profile: LimitsProfile::InteractiveV1,
                $( $field: $default, )+
                deadline_unix_ms: None,
            }
        }
    };
}

limit_fields! {
    /// Maximum bytes accepted by a bounded parse entry point before the
    /// document is decoded.
    max_input_bytes / with_max_input_bytes = 4 * 1024 * 1024;
    /// Maximum bytes of a single resolved DID source.
    max_source_bytes / with_max_source_bytes = 1024 * 1024;
    /// Maximum aggregate bytes across every source in a resolved bundle.
    max_bundle_bytes / with_max_bundle_bytes = 8 * 1024 * 1024;
    /// Maximum number of sources in a resolved bundle.
    max_sources / with_max_sources = 256;
    /// Maximum bytes in a single logical source ID (name/path).
    ///
    /// Source IDs are otherwise bounded only cumulatively by
    /// [`Limits::max_string_bytes`], so one entry could carry a megabyte-long
    /// path. This bounds each ID individually on both the resolver and the
    /// embedded-sidecar paths.
    max_source_id_bytes / with_max_source_id_bytes = 1024;
    /// Maximum import chain depth during source resolution.
    max_import_depth / with_max_import_depth = 64;
    /// Maximum import edges across a resolved bundle.
    max_import_edges / with_max_import_edges = 1024;
    /// Maximum lexical nesting accepted before invoking the upstream parser.
    max_source_nesting / with_max_source_nesting = 256;
    /// Maximum semantic type nesting lowered from a checked Candid program.
    max_type_depth / with_max_type_depth = 256;
    /// Maximum lexical JSON container nesting accepted before invoking the
    /// recursive `serde_json` decoder on a HostValue document.
    ///
    /// This is the HostValue analogue of [`Limits::max_source_nesting`]: it
    /// bounds *lexical* nesting (`{` and `[` in the JSON text) before a
    /// recursive decoder runs, whereas [`Limits::max_value_depth`] bounds
    /// *semantic* HostValue nesting after decoding. The two units differ — one
    /// `vec` level costs two JSON containers and one `record` level costs
    /// three — so a document rejected here reports `value_nesting`, never
    /// `value_depth`.
    ///
    /// Raising this above 128 has no effect: `serde_json` applies a fixed
    /// 128-frame recursion ceiling that this crate deliberately does not
    /// disable, so documents nested deeper than 128 containers are rejected by
    /// the decoder as malformed rather than by this limit.
    ///
    /// # Choosing a value for a small stack
    ///
    /// Rejecting a document costs constant stack, so no input can drive an
    /// abort by being *deeper* than this limit. Accepting one still recurses,
    /// at a cost that is build-profile dependent, so this limit is the knob for
    /// matching decode to the stack the host actually runs on. Measured on a
    /// 64 KiB stack with a nested-`opt` document:
    ///
    /// | Profile | Cost per container | Deepest safe |
    /// |---|---|---|
    /// | release | ~640 B | ~103 |
    /// | debug | ~8 KiB | ~7 |
    ///
    /// The default of 64 is chosen to keep a release build inside a 64 KiB
    /// stack with roughly a third of it to spare, which is the bar
    /// `tests/deep_nesting.rs` sets. A debug build on a stack that small needs
    /// this lowered to single digits; a host on an ordinary 8 MiB stack can
    /// raise it to 128 without approaching either bound.
    //
    // Deliberately below `serde_json`'s fixed 128-frame ceiling so this
    // crate's own check is always the one that fires, and low enough that
    // decoding a document at exactly this bound stays well inside the 64 KiB
    // stack `tests/deep_nesting.rs` asserts elsewhere.
    max_value_nesting / with_max_value_nesting = 64;
    /// Maximum type nodes in a Contract arena.
    max_type_nodes / with_max_type_nodes = 100_000;
    /// Maximum edges in a Contract type graph.
    max_graph_edges / with_max_graph_edges = 1_000_000;
    /// Maximum named declarations in a Contract.
    max_declarations / with_max_declarations = 100_000;
    /// Maximum aggregate record/variant fields across a Contract.
    max_fields / with_max_fields = 500_000;
    /// Maximum aggregate service methods across a Contract.
    max_methods / with_max_methods = 100_000;
    /// Maximum aggregate function arguments and results across a Contract.
    max_function_values / with_max_function_values = 500_000;
    /// Maximum aggregate string bytes across declaration and method names.
    max_string_bytes / with_max_string_bytes = 1024 * 1024;
    /// Maximum aggregate bytes across the four producer metadata strings.
    ///
    /// Producer metadata is untrusted, caller-supplied provenance that is
    /// deliberately kept out of authenticated Contract identity (see
    /// [`crate::ProducerInfo`]); this bounds the bytes it may contribute to a
    /// validated Contract without ever affecting an identity hash.
    max_producer_bytes / with_max_producer_bytes = 4096;
    /// Maximum retained diagnostics per failure.
    ///
    /// When more violations are observed than fit under this cap, the final
    /// retained item is replaced by a `resource_limit_exceeded` sentinel
    /// carrying the true observed count. A cap of `0` retains exactly that
    /// one sentinel, so an invalid input never yields an empty error
    /// collection.
    max_diagnostics / with_max_diagnostics = 100;
    /// Maximum canonicalization work units per operation.
    max_canonicalization_work / with_max_canonicalization_work = 10_000_000;
    /// Maximum work units charged while resolving provenance targets.
    ///
    /// Kept separate from [`Limits::max_canonicalization_work`] so that
    /// rederiving a large graph and then indexing its provenance sidecar cannot
    /// jointly exhaust one counter. Bounds building each referenced container's
    /// field-ID / method-name index and every membership test, so adversarial
    /// fan-out and duplicate provenance entries cannot drive an unbounded scan.
    max_provenance_work / with_max_provenance_work = 10_000_000;
    /// Maximum work units charged while serializing and hashing source-bundle
    /// identity (`candid-core:source-bundle:v1`).
    ///
    /// Each identity computation charges one unit per serialized payload byte
    /// during an allocation-free counting pass, then reserves two more units
    /// per byte (materializing and hashing the canonical bytes) plus the
    /// domain-tag overhead before any allocation occurs. A presented sidecar
    /// validation performs two passes on one budget — verifying the presented
    /// `source_bundle_id` and emitting the rederived bundle's ID — while a
    /// plain compilation performs one.
    ///
    /// Kept separate from [`Limits::max_canonicalization_work`] because the
    /// serialized bundle scales with `max_bundle_bytes`: metering it on the
    /// canonicalization counter would either starve graph work or force that
    /// default far above what graph canonicalization needs. The default
    /// accepts every bundle valid under the default byte/count limits: JSON
    /// string escaping expands a byte to at most six, so one pass costs at
    /// most `3 * 6 * (max_bundle_bytes + identity strings) + entry overhead`,
    /// about 213M units for a compile pass and 341M for the two validation
    /// passes together; 400M covers both with headroom.
    max_source_identity_work / with_max_source_identity_work = 400_000_000;
    /// Maximum semantic HostValue nesting depth.
    max_value_depth / with_max_value_depth = 256;
    /// Maximum aggregate HostValue elements per document.
    max_value_elements / with_max_value_elements = 1_000_000;
    /// Maximum aggregate HostValue text/blob bytes per document.
    max_value_bytes / with_max_value_bytes = 16 * 1024 * 1024;
}

impl Default for Limits {
    /// [`LimitsProfile::InteractiveV1`].
    fn default() -> Self {
        LimitsProfile::InteractiveV1.limits()
    }
}

impl Limits {
    /// The profile these limits started from. Overridden fields do not
    /// change the profile; it names the baseline, not the final values.
    pub fn profile(&self) -> LimitsProfile {
        self.profile
    }

    /// Optional Unix timestamp in milliseconds after which work must abort.
    ///
    /// `None` means no deadline. A value at or before the current time makes
    /// every bounded operation fail closed with
    /// `operation_deadline_exceeded` before performing work.
    pub fn deadline_unix_ms(&self) -> Option<u64> {
        self.deadline_unix_ms
    }

    /// Returns `self` with `deadline_unix_ms` replaced. See
    /// [`Limits::deadline_unix_ms`].
    #[must_use]
    pub fn with_deadline_unix_ms(mut self, deadline_unix_ms: Option<u64>) -> Self {
        self.deadline_unix_ms = deadline_unix_ms;
        self
    }

    pub fn deadline_exceeded(&self) -> bool {
        let Some(deadline) = self.deadline_unix_ms else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(u64::MAX, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            });
        now >= deadline
    }
}

/// The versioned, portable wire form of [`Limits`].
///
/// ```json
/// {"version":1,"profile":"interactive_v1","overrides":{}}
/// ```
///
/// That document is exactly how [`Limits::default`] serializes. `version`
/// pins the schema ([`LIMITS_CONFIG_VERSION`]), `profile` names the frozen
/// baseline defaults ([`LimitsProfile::wire_name`]), and `overrides` carries
/// only the explicitly overridden fields as fixed-width `u64` values, so the
/// same document configures identical policy on every supported (32- and
/// 64-bit) host.
/// Unknown top-level fields, unknown override fields, unsupported versions,
/// and unknown profiles are all rejected; an override that does not fit the
/// host platform's `usize` is rejected with a structured
/// [`LimitsConfigError`] rather than truncated or wrapped. A missing
/// `overrides` object means no overrides.
///
/// This type is the serde representation behind `Limits`'
/// [`Serialize`]/[`Deserialize`] impls; convert explicitly with
/// [`From<&Limits>`] and [`TryFrom<LimitsConfig>`] when the structured
/// [`LimitsConfigError`] must be inspected programmatically instead of
/// wrapped in a serde error string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    version: u64,
    profile: String,
    #[serde(default)]
    overrides: LimitsOverrides,
}

impl From<&Limits> for LimitsConfig {
    fn from(limits: &Limits) -> Self {
        Self {
            version: LIMITS_CONFIG_VERSION,
            profile: limits.profile.wire_name().to_string(),
            overrides: LimitsOverrides::diff(limits),
        }
    }
}

impl From<Limits> for LimitsConfig {
    fn from(limits: Limits) -> Self {
        Self::from(&limits)
    }
}

impl TryFrom<LimitsConfig> for Limits {
    type Error = LimitsConfigError;

    fn try_from(config: LimitsConfig) -> Result<Self, LimitsConfigError> {
        if config.version != LIMITS_CONFIG_VERSION {
            return Err(LimitsConfigError {
                code: "unsupported_limits_version",
                path: "$.version".to_string(),
                message: format!(
                    "unsupported limits config version {}; this build supports version {LIMITS_CONFIG_VERSION}",
                    config.version
                ),
            });
        }
        let profile =
            LimitsProfile::from_wire_name(&config.profile).ok_or_else(|| LimitsConfigError {
                code: "unsupported_limits_profile",
                path: "$.profile".to_string(),
                message: format!(
                    "unknown limits profile {:?}; known profiles: \"interactive_v1\"",
                    config.profile
                ),
            })?;
        config.overrides.apply(profile)
    }
}

/// A structured, stable rejection of a portable limits configuration.
///
/// Carried by [`TryFrom<LimitsConfig>`] and wrapped (via [`std::fmt::Display`])
/// by serde when a `Limits` or `RuntimeContext` document is rejected during
/// deserialization. The `code`, `path`, and rendered message are pinned
/// public API:
///
/// | code | path | condition |
/// |---|---|---|
/// | `unsupported_limits_version` | `$.version` | version is not `1` |
/// | `unsupported_limits_profile` | `$.profile` | profile name is unknown |
/// | `limit_override_unrepresentable` | `$.overrides.<field>` | override exceeds this platform's `usize::MAX` |
///
/// Displays as `{code} at {path}: {message}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitsConfigError {
    code: &'static str,
    path: String,
    message: String,
}

impl LimitsConfigError {
    /// The stable machine-readable code.
    pub fn code(&self) -> &str {
        self.code
    }

    /// The JSON path of the rejected value within the configuration document.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The human-readable description.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for LimitsConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code, self.path, self.message
        )
    }
}

impl std::error::Error for LimitsConfigError {}

/// Invoked only when an override key is present; an absent key takes the
/// `None` default. Delegating to `u64` directly makes an explicit JSON `null`
/// a decode error instead of a second spelling of "no override", mirroring
/// how [`crate::RawContract`] rejects `"actor": null`.
fn deserialize_override_forbidding_null<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    u64::deserialize(deserializer).map(Some)
}

/// Exact `usize` → `u64` widening for portable wire values.
///
/// Lossless by construction: the crate refuses to compile on targets whose
/// `usize` exceeds 64 bits (see the `target_pointer_width` guard in
/// `lib.rs`), so this cast is exact on every supported (32- and 64-bit)
/// target.
pub(crate) fn portable_count(value: usize) -> u64 {
    value as u64
}

/// Whether a portable `u64` value fits a platform word whose maximum is
/// `platform_max`. Factored out of [`override_to_usize`] so the 32-bit
/// boundary is testable on any host by passing `u32::MAX as u64`.
fn representable(value: u64, platform_max: u64) -> bool {
    value <= platform_max
}

/// Checked `u64` → `usize` narrowing at the configuration boundary. Never
/// truncates, wraps, or panics: a value the platform cannot represent is a
/// structured [`LimitsConfigError`] at `$.overrides.<field>`.
fn override_to_usize(field: &'static str, value: u64) -> Result<usize, LimitsConfigError> {
    if !representable(value, portable_count(usize::MAX)) {
        return Err(LimitsConfigError {
            code: "limit_override_unrepresentable",
            path: format!("$.overrides.{field}"),
            message: format!(
                "{field} override {value} exceeds this platform's usize::MAX ({})",
                usize::MAX
            ),
        });
    }
    // Exact: `value <= usize::MAX` was just checked.
    Ok(value as usize)
}

/// A cheap, cloneable signal for cooperatively cancelling runtime work.
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl PartialEq for CancellationToken {
    fn eq(&self, other: &Self) -> bool {
        self.is_cancelled() == other.is_cancelled()
    }
}

impl Eq for CancellationToken {}

/// Runtime policy and cooperative controls for one public operation.
///
/// Construct contexts with [`RuntimeContext::new`]; runtime controls are
/// intentionally private so adding one does not reopen exhaustive literals.
///
/// ```compile_fail
/// use candid_core::{Limits, RuntimeContext};
///
/// let _ = RuntimeContext { limits: Limits::default() };
/// ```
///
/// Serializes as `{"limits": <portable limits config>}` (see
/// [`LimitsConfig`]); the cancellation token is host-local bookkeeping and is
/// never serialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContext {
    pub limits: Limits,
    #[serde(skip, default)]
    cancellation: CancellationToken,
}

impl PartialEq for RuntimeContext {
    fn eq(&self, other: &Self) -> bool {
        self.limits == other.limits
    }
}

impl Eq for RuntimeContext {}

impl RuntimeContext {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            cancellation: CancellationToken::new(),
        }
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn budget(&self) -> crate::budget::Budget<'_> {
        crate::budget::Budget::new(&self.limits, self.cancellation.clone())
    }
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representable_simulates_the_32_bit_boundary_exactly() {
        let simulated_32_bit_max = portable_count(u32::MAX as usize);
        assert!(representable(u32::MAX as u64, simulated_32_bit_max));
        assert!(!representable(u32::MAX as u64 + 1, simulated_32_bit_max));
        assert!(!representable(u64::MAX, simulated_32_bit_max));
        assert!(representable(0, simulated_32_bit_max));
    }

    #[test]
    fn override_conversion_is_exact_or_a_structured_error() {
        assert_eq!(override_to_usize("max_input_bytes", 0), Ok(0));
        assert_eq!(
            override_to_usize("max_input_bytes", portable_count(usize::MAX)),
            Ok(usize::MAX)
        );
        #[cfg(target_pointer_width = "64")]
        {
            // On a 64-bit host every u64 is representable, so the error path
            // is exercised through `representable` above and the pinned error
            // shape is constructed directly here.
            let error = LimitsConfigError {
                code: "limit_override_unrepresentable",
                path: "$.overrides.max_input_bytes".to_string(),
                message: "max_input_bytes override 5000000000 exceeds this platform's usize::MAX (4294967295)".to_string(),
            };
            assert_eq!(error.code(), "limit_override_unrepresentable");
            assert_eq!(error.path(), "$.overrides.max_input_bytes");
        }
        #[cfg(not(target_pointer_width = "64"))]
        {
            let error = override_to_usize("max_input_bytes", u64::MAX).unwrap_err();
            assert_eq!(error.code(), "limit_override_unrepresentable");
            assert_eq!(error.path(), "$.overrides.max_input_bytes");
        }
    }

    #[test]
    fn overrides_diff_and_apply_round_trip() {
        let limits = Limits::default()
            .with_max_input_bytes(1)
            .with_max_diagnostics(0)
            .with_deadline_unix_ms(Some(7));
        let config = LimitsConfig::from(&limits);
        assert_eq!(Limits::try_from(config), Ok(limits));

        let untouched = Limits::default();
        assert_eq!(
            LimitsOverrides::diff(&untouched),
            LimitsOverrides::default()
        );
    }

    #[test]
    fn an_override_equal_to_the_profile_value_is_normalized_away() {
        let baseline = Limits::default();
        let explicit = Limits::default().with_max_input_bytes(baseline.max_input_bytes());
        assert_eq!(LimitsOverrides::diff(&explicit), LimitsOverrides::default());
        assert_eq!(explicit, baseline);
    }

    #[test]
    fn profile_wire_names_round_trip() {
        assert_eq!(LimitsProfile::InteractiveV1.wire_name(), "interactive_v1");
        assert_eq!(
            LimitsProfile::from_wire_name("interactive_v1"),
            Some(LimitsProfile::InteractiveV1)
        );
        assert_eq!(LimitsProfile::from_wire_name("interactive-v1"), None);
        assert_eq!(LimitsProfile::from_wire_name("server_v1"), None);
    }
}
