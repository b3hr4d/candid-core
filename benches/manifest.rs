//! Emits the benchmark manifest: the identity of *what* was measured.
//!
//! A percentage delta between two benchmark runs means nothing unless both runs
//! measured the same thing on comparable machinery. `docs/benchmarks.md` has
//! always said so in prose — "fixture contents and generator sizes are part of
//! the benchmark definition" — but nothing recorded it in a form a tool could
//! check, so an incompatible comparison looked exactly like a compatible one.
//!
//! This binary emits the half of that identity only the Rust side can know: the
//! benchmark IDs, the corpus and its generator parameters, the feature set the
//! benchmarks were built with, and the units each metric is reported in. The
//! environment half — toolchain, target, host, lockfile digest — is recorded by
//! `tests/fixtures/benchmarks/compare.py`, which can observe it reliably and
//! this binary cannot.
//!
//! `harness = false` and it takes no measurements: emitting the manifest must
//! not perturb the run it describes.

#[allow(dead_code)]
mod support;

use serde::Serialize;

/// Bumped whenever the manifest's meaning changes in a way that makes an older
/// stored baseline incomparable. `compare.py` refuses to compare across
/// versions rather than silently reinterpreting old fields.
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// FNV-1a over the source bytes. Implemented here rather than pulled in as a
/// dependency: this identifies "did the corpus change", which needs determinism
/// and stability across platforms, not collision resistance against an
/// adversary. Adding a hash crate to dev-dependencies would change the packaged
/// manifest for no benefit a consumer can observe.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Serialize)]
struct CorpusCase {
    name: String,
    bytes: usize,
    /// Lowercase hex FNV-1a of the exact source text.
    fingerprint: String,
}

#[derive(Serialize)]
struct GeneratorSizes {
    wide_record_fields: usize,
    service_methods: usize,
    recursive_chain_depth: usize,
}

#[derive(Serialize)]
struct Manifest {
    schema_version: u32,
    /// Every case the statistical suite draws from, with its exact content
    /// identity. A changed fingerprint means results either side of the change
    /// are not comparable.
    corpus: Vec<CorpusCase>,
    /// Rolled up over the corpus in order, so one value answers "same corpus?".
    corpus_id: String,
    generator_sizes: GeneratorSizes,
    imported_bundle_bytes: u64,
    /// The feature set these benchmarks were compiled with. A benchmark built
    /// without `host-value` measures a different program.
    features: Vec<&'static str>,
    metric_units: MetricUnits,
}

#[derive(Serialize)]
struct MetricUnits {
    time: &'static str,
    throughput: &'static str,
    allocations: &'static str,
    allocated_bytes: &'static str,
    peak_live_bytes: &'static str,
}

fn features() -> Vec<&'static str> {
    let mut names = Vec::new();
    if cfg!(feature = "compiler") {
        names.push("compiler");
    }
    if cfg!(feature = "filesystem-compiler") {
        names.push("filesystem-compiler");
    }
    if cfg!(feature = "host-value") {
        names.push("host-value");
    }
    names
}

fn main() {
    // `cargo bench --benches -- --test` exercises every benchmark once. This
    // one has nothing to measure, so the smoke contract is simply that it
    // builds and emits a well-formed manifest; falling through to the normal
    // path satisfies that without a special case.
    let cases = support::import_free_cases();

    // The imported bundle is part of the corpus, not an aside: three of the
    // `compile/imported_bundle` benchmarks measure exactly these files. Earlier
    // this recorded only their aggregate byte count, which cannot see an edit
    // that preserves total length — so a changed bundle could pass the
    // compatibility check and be compared as if nothing had moved.
    let corpus: Vec<CorpusCase> = cases
        .iter()
        .map(|case| CorpusCase {
            name: case.name.to_string(),
            bytes: case.source.len(),
            fingerprint: format!("{:016x}", fnv1a64(case.source.as_bytes())),
        })
        .chain(
            support::imported_bundle_sources()
                .iter()
                .map(|(name, source)| CorpusCase {
                    name: format!("imports/{name}"),
                    bytes: source.len(),
                    fingerprint: format!("{:016x}", fnv1a64(source.as_bytes())),
                }),
        )
        .collect();

    // Order matters and is the declaration order of `import_free_cases`, so a
    // reordering is a manifest change. That is deliberate: Criterion reports
    // per-benchmark, and a reordered corpus is a different benchmark set.
    let rolled = corpus
        .iter()
        .fold(String::new(), |mut acc, case| {
            acc.push_str(&case.name);
            acc.push(':');
            acc.push_str(&case.fingerprint);
            acc.push(';');
            acc
        })
        .into_bytes();

    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        corpus,
        corpus_id: format!("{:016x}", fnv1a64(&rolled)),
        generator_sizes: GeneratorSizes {
            wide_record_fields: 256,
            service_methods: 128,
            recursive_chain_depth: 128,
        },
        imported_bundle_bytes: support::imported_bundle_bytes(),
        features: features(),
        metric_units: MetricUnits {
            time: "nanoseconds",
            throughput: "bytes_per_second",
            allocations: "count",
            allocated_bytes: "bytes",
            peak_live_bytes: "bytes",
        },
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&manifest).expect("benchmark manifest must serialize")
    );
}
