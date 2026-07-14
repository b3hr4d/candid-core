# Performance benchmarks

The benchmark suite quantifies the cost of projecting checked Candid into a
canonical Contract. It is not a contest between independent parsers:
`candid-core` delegates parsing and type checking to the exact official
`candid_parser` version pinned in `Cargo.toml`.

The direct comparison is therefore:

1. official `IDLProg` parsing plus `check_prog` or `check_file`;
2. the same semantic work plus `candid-core` lowering, structural validation,
   graph canonicalization, identity hashing, and optional source provenance.

Official checker and core outputs provide different guarantees. Ratios only
describe the additional Contract projection work for this corpus and build.

## Run the suite

The statistical suite uses Criterion 0.5.1, pinned because benchmarks must
continue to compile on the declared Rust 1.78 MSRV:

```sh
cargo bench --bench compilation --locked -- --noplot
```

Run the one-shot allocation probe separately so allocator instrumentation does
not affect latency measurements:

```sh
cargo bench --bench allocation --locked
```

The probe emits JSON with allocation count, cumulative allocated bytes, and
peak live bytes observed by its counting system allocator. Peak live bytes are
not resident set size (RSS): allocator metadata, mapped-but-unused pages,
stacks, shared libraries, and the executable are outside that counter. For a
process-level observation, run the allocation probe under the platform's
`time` or memory profiler and record the exact command and environment.

To exercise fixture validity and every benchmark path without collecting
statistics:

```sh
cargo bench --benches --locked -- --test
```

## Groups and boundaries

`compile/<case>` uses identical import-free source bytes for all three paths:

- `official_parse_check` creates a fresh `IDLProg`, `TypeEnv`, and checked
  actor in every iteration;
- `core_minimal` disables `SourceInfo` while retaining the canonical Contract;
- `core_full` includes the source/provenance sidecar.

Fixture construction, validation of the fixture itself, and Criterion setup
occur outside timed loops. Returned values pass through `black_box`. A fresh
mutable checker environment is required because reusing it would give the
official baseline an invalid cache advantage.

`compile/imported_bundle` is separate because it includes file access and
import processing. `official_check_file` reads the checked-in bundle directly.
`core_compile_with_resolver` reads the same bundle through `WorkspaceResolver`,
materializes the hermetic checked view, invokes the official file checker, and
projects the result. This is an end-to-end comparison, not a parser microbench.

`artifact/ledger` isolates operations on one already compiled Contract:

- structural/identity validation;
- canonicalization;
- compact serde serialization;
- the validated, canonicalized pretty-JSON convenience path;
- JSON parse, validation, and canonicalization.

The final parse group uses today's `Contract::from_json` boundary. Issue #22's
future incremental/context-aware decoding should be added as a distinct path,
not silently substituted for the historical result.

## Corpus

The suite deliberately combines:

- small fixtures from `tests/fixtures/conformance`;
- the repository-authored ledger-style interface documented in
  `benches/corpus/README.md`;
- deterministic generated record-width, method-count, and recursive-depth
  cases;
- a three-file imported ledger/archive bundle.

The generated wide and long-chain cases provide performance signals relevant
to Issue #6. They do not replace its deterministic work-accounting regression
test and should not drive an optimization until repeated measurements identify
a bottleneck. The benchmark crate split may need adjustment when Issue #24 is
implemented, but the comparison boundaries should remain stable.

Fixture contents and generator sizes are part of the benchmark definition.
Changing them requires an explicit note because results before and after the
change are not directly comparable.

## Compare a branch with `main`

Build both revisions with the same Rust toolchain, Cargo lockfile, target CPU
settings, power mode, and allocator. Minimize other host activity and use the
same checkout path when practical.

On `main`, save a named Criterion baseline:

```sh
cargo bench --bench compilation --locked -- --noplot --save-baseline main
```

On the candidate branch, compare against it:

```sh
cargo bench --bench compilation --locked -- --noplot --baseline main
```

Treat one result as evidence to investigate, not proof of a regression.
Hardware, thermals, OS scheduling, filesystem caches, compiler version, target
features, and allocator all affect results. Repeat suspected changes and inspect
absolute time, throughput, confidence intervals, and the allocation probe—not
only a percentage.

## CI policy

Ordinary pull requests smoke-run every benchmark once. They do not enforce
wall-clock thresholds on shared GitHub-hosted runners. The weekly schedule and
manual workflow dispatch run the statistical suite and allocation probe, then
upload:

- Criterion's raw machine-readable estimates and samples;
- allocation-probe JSON;
- Rust/Cargo versions, host information, and the exact Git commit.

Artifacts are retained for 90 days. A sustained regression larger than roughly
10% across repeated comparable runs should be investigated and explained, but
is not an automatic correctness failure. A dedicated stable runner is required
before introducing blocking timing thresholds.

## Initial baseline

The first checked-in local measurement is recorded in
[`benchmarks/baseline-2026-07-14.md`](benchmarks/baseline-2026-07-14.md). It is a
reproducibility example and historical reference, not a portable performance
promise.
