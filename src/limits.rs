use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Operational limits for work performed on untrusted Contracts, sources, and values.
///
/// Limits never participate in Contract identity. Hosts may raise them explicitly
/// for trusted workloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_source_bytes: usize,
    pub max_bundle_bytes: usize,
    pub max_sources: usize,
    /// Maximum bytes in a single logical source ID (name/path).
    ///
    /// Source IDs are otherwise bounded only cumulatively by
    /// [`Limits::max_string_bytes`], so one entry could carry a megabyte-long
    /// path. This bounds each ID individually on both the resolver and the
    /// embedded-sidecar paths.
    pub max_source_id_bytes: usize,
    pub max_import_depth: usize,
    pub max_import_edges: usize,
    /// Maximum lexical nesting accepted before invoking the upstream parser.
    pub max_source_nesting: usize,
    /// Maximum semantic type nesting lowered from a checked Candid program.
    pub max_type_depth: usize,
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
    pub max_value_nesting: usize,
    pub max_type_nodes: usize,
    pub max_graph_edges: usize,
    pub max_declarations: usize,
    pub max_fields: usize,
    pub max_methods: usize,
    pub max_function_values: usize,
    pub max_string_bytes: usize,
    /// Maximum aggregate bytes across the four producer metadata strings.
    ///
    /// Producer metadata is untrusted, caller-supplied provenance that is
    /// deliberately kept out of authenticated Contract identity (see
    /// [`crate::ProducerInfo`]); this bounds the bytes it may contribute to a
    /// validated Contract without ever affecting an identity hash.
    pub max_producer_bytes: usize,
    pub max_diagnostics: usize,
    pub max_canonicalization_work: usize,
    /// Maximum work units charged while resolving provenance targets.
    ///
    /// Kept separate from [`Limits::max_canonicalization_work`] so that
    /// rederiving a large graph and then indexing its provenance sidecar cannot
    /// jointly exhaust one counter. Bounds building each referenced container's
    /// field-ID / method-name index and every membership test, so adversarial
    /// fan-out and duplicate provenance entries cannot drive an unbounded scan.
    pub max_provenance_work: usize,
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
    ///
    /// Compatibility: this field is additive and pre-1.0. Callers using struct
    /// update syntax (`..Limits::default()`) are unaffected; exhaustive
    /// literals must add it. Serialized `Limits` documents without the field
    /// deserialize to the default, but documents that include it are rejected
    /// by older releases (`deny_unknown_fields`).
    pub max_source_identity_work: usize,
    pub max_value_depth: usize,
    pub max_value_elements: usize,
    pub max_value_bytes: usize,
    /// Optional Unix timestamp in milliseconds after which work must abort.
    pub deadline_unix_ms: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 4 * 1024 * 1024,
            max_source_bytes: 1024 * 1024,
            max_bundle_bytes: 8 * 1024 * 1024,
            max_sources: 256,
            max_source_id_bytes: 1024,
            max_import_depth: 64,
            max_import_edges: 1024,
            max_source_nesting: 256,
            max_type_depth: 256,
            // Deliberately below `serde_json`'s fixed 128-frame ceiling so this
            // crate's own check is always the one that fires, and low enough
            // that decoding a document at exactly this bound stays well inside
            // the 64 KiB stack `tests/deep_nesting.rs` asserts elsewhere.
            max_value_nesting: 64,
            max_type_nodes: 100_000,
            max_graph_edges: 1_000_000,
            max_declarations: 100_000,
            max_fields: 500_000,
            max_methods: 100_000,
            max_function_values: 500_000,
            max_string_bytes: 1024 * 1024,
            max_producer_bytes: 4096,
            max_diagnostics: 100,
            max_canonicalization_work: 10_000_000,
            max_provenance_work: 10_000_000,
            max_source_identity_work: 400_000_000,
            max_value_depth: 256,
            max_value_elements: 1_000_000,
            max_value_bytes: 16 * 1024 * 1024,
            deadline_unix_ms: None,
        }
    }
}

impl Limits {
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
