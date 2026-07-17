use serde::{Deserialize, Serialize};

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
    pub max_import_depth: usize,
    pub max_import_edges: usize,
    /// Maximum lexical nesting accepted before invoking the upstream parser.
    pub max_source_nesting: usize,
    /// Maximum semantic type nesting lowered from a checked Candid program.
    pub max_type_depth: usize,
    pub max_type_nodes: usize,
    pub max_graph_edges: usize,
    pub max_declarations: usize,
    pub max_fields: usize,
    pub max_methods: usize,
    pub max_function_values: usize,
    pub max_string_bytes: usize,
    pub max_diagnostics: usize,
    pub max_canonicalization_work: usize,
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
            max_import_depth: 64,
            max_import_edges: 1024,
            max_source_nesting: 256,
            max_type_depth: 256,
            max_type_nodes: 100_000,
            max_graph_edges: 1_000_000,
            max_declarations: 100_000,
            max_fields: 500_000,
            max_methods: 100_000,
            max_function_values: 500_000,
            max_string_bytes: 1024 * 1024,
            max_diagnostics: 100,
            max_canonicalization_work: 10_000_000,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContext {
    pub limits: Limits,
}
