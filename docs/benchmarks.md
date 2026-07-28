# Performance benchmarks

The benchmark suite quantifies the cost of projecting checked Candid into a canonical Contract. It is not a contest between independent parsers: `candid-core` delegates parsing and type checking to the exact official `candid_parser` version pinned in `Cargo.toml`.

The direct comparison is therefore:

1. official `IDLProg` parsing plus `check_prog` or `check_file`;
2. the same semantic work plus `candid-core` lowering, structural validation, graph canonicalization, identity hashing, and optional source provenance.

Official checker and core outputs provide different guarantees. Ratios only describe the additional Contract projection work for this corpus and build.

## Run the suite

The statistical suite uses Criterion 0.5.1, pinned because benchmarks must continue to compile on the declared Rust 1.78 MSRV:

```sh
cargo bench --bench compilation --locked -- --noplot
```

Run the one-shot allocation probe separately so allocator instrumentation does not affect latency measurements:

```sh
cargo bench --bench allocation --locked
```

The probe emits JSON with allocation count, cumulative allocated bytes, and peak live bytes observed by its counting system allocator. Peak live bytes are not resident set size (RSS): allocator metadata, mapped-but-unused pages, stacks, shared libraries, and the executable are outside that counter. For a process-level observation, run the allocation probe under the platform's `time` or memory profiler and record the exact command and environment.

To exercise fixture validity and every benchmark path without collecting statistics:

```sh
cargo bench --benches --locked -- --test
```

## Groups and boundaries

`compile/<case>` uses identical import-free source bytes for all three paths:

- `official_parse_check` creates a fresh `IDLProg`, `TypeEnv`, and checked actor in every iteration;
- `core_minimal` disables `SourceInfo` while retaining the canonical Contract;
- `core_full` includes the source/provenance sidecar.

Fixture construction, validation of the fixture itself, and Criterion setup occur outside timed loops. Returned values pass through `black_box`. A fresh mutable checker environment is required because reusing it would give the official baseline an invalid cache advantage.

`compile/imported_bundle` is separate because it includes file access and import processing. `official_check_file` reads the checked-in bundle directly. `core_compile_with_resolver` reads the same bundle through `WorkspaceResolver` — which is why the group still declares `filesystem-compiler` — and then compiles it entirely in memory: since issue #21 the resolved bundle is merged into one virtual program and type-checked in place rather than materialized for the official file checker. The comparison therefore no longer runs the same checker on both sides; it is an end-to-end comparison of the two ways to get from a bundle on disk to a checked result, not a parser microbench. Nothing in this document claims a speedup from that change: the benchmark numbers on file were recorded against the materializing implementation and have not been re-recorded.

`artifact/ledger` isolates operations on one already compiled Contract — this group is entirely base-feature work, so its costs are what a `default-features = false` consumer pays:

- structural/identity validation;
- canonicalization;
- compact serde serialization;
- the validated, canonicalized pretty-JSON convenience path;
- JSON parse, validation, and canonicalization.

The final parse group uses today's `Contract::from_json` boundary. Issue #22's future incremental/context-aware decoding should be added as a distinct path, not silently substituted for the historical result.

## Corpus

The suite deliberately combines:

- small fixtures from `tests/fixtures/conformance`;
- the repository-authored ledger-style interface documented in `benches/corpus/README.md`;
- deterministic generated record-width, method-count, and recursive-depth cases;
- a three-file imported ledger/archive bundle.

The generated wide and long-chain cases provide performance signals relevant to Issue #6. They do not replace its deterministic work-accounting regression test and should not drive an optimization until repeated measurements identify a bottleneck. Issue #24 split the build surface with Cargo features rather than into separate crates, so the corpus and the comparison boundaries are unchanged; only the `required-features` declarations noted above were added.

Fixture contents and generator sizes are part of the benchmark definition. Changing them requires an explicit note because results before and after the change are not directly comparable.

## Compare a branch with `main`

Build both revisions with the same Rust toolchain, Cargo lockfile, feature set, target CPU settings, power mode, and allocator. Minimize other host activity and use the same checkout path when practical.

On `main`, save a named Criterion baseline:

```sh
cargo bench --bench compilation --locked -- --noplot --save-baseline main
```

On the candidate branch, compare against it:

```sh
cargo bench --bench compilation --locked -- --noplot --baseline main
```

Treat one result as evidence to investigate, not proof of a regression. Hardware, thermals, OS scheduling, filesystem caches, compiler version, target features, and allocator all affect results. Repeat suspected changes and inspect absolute time, throughput, confidence intervals, and the allocation probe—not only a percentage.

### Durable baselines and a checked comparison

Criterion's named baselines above are convenient but say nothing about *what* was measured: they will happily compare two runs over different corpora, feature sets, or toolchains and report a confident percentage. `tests/fixtures/benchmarks/compare.py` exists to make that precondition checkable, and it is built to refuse rather than guess — a missing or incompatible baseline reports that no comparison was made and exits non-zero, never a delta it cannot stand behind.

A run's identity has two halves. `cargo bench --bench manifest` emits what only the Rust side knows: the corpus fingerprints, generator sizes, feature set, and metric units. The script records what only it can observe: toolchain, target, host, and the lockfile digest.

Capture a baseline on `main`:

```sh
cargo bench --bench manifest --locked > /tmp/manifest.json
cargo bench --bench compilation --locked -- --noplot
cargo bench --bench allocation --locked > /tmp/allocations.json
python3 tests/fixtures/benchmarks/compare.py capture \
  --manifest /tmp/manifest.json --criterion target/criterion \
  --allocations /tmp/allocations.json \
  --out benches/baselines/main.json --note "why this baseline was taken"
```

Run the same three commands on the candidate branch, then compare:

```sh
python3 tests/fixtures/benchmarks/compare.py compare \
  --baseline benches/baselines/main.json --manifest /tmp/manifest.json \
  --criterion target/criterion --allocations /tmp/allocations.json \
  --markdown /tmp/report.md
```

**Emit the manifest before running the suite.** That ordering is load-bearing, not stylistic: Criterion never deletes a `new/` directory, so a benchmark you renamed or removed keeps its result from whatever ran last in the same `target/criterion`. The manifest file's timestamp is the run's epoch, and any estimate older than it is treated as left over from a previous run — excluded from the comparison and reported, rather than imported as if this run had produced it.

What it refuses, and why:

| Situation | Behaviour |
| --- | --- |
| Baseline missing or unreadable | No comparison; exit 2 |
| Corpus (including the imported bundle), generator sizes, feature set, or metric units differ | No comparison; exit 2. The two runs did not measure the same thing, so no delta between them is interpretable — recapture the baseline |
| Toolchain, effective target, `RUSTFLAGS`, host, or lockfile differ | No comparison unless `--allow-environment-drift`, which renders the report marked **informational only** and refuses to gate on it |
| No Criterion estimates newer than the manifest | No comparison; exit 2 — the suite was not run after the manifest was emitted |
| Compatible | Renders Markdown and, with `--json`, machine-readable output |

Codegen flags are part of the identity because they change the binary without changing the toolchain: `RUSTFLAGS='-C target-cpu=native'` on one run and not the other produces two materially different programs on one machine, which would otherwise look like no drift at all.

All three allocation metrics are compared — `allocations`, `allocated_bytes`, and `peak_live_bytes`. A change that holds the allocation count constant while growing cumulative or peak bytes is a real memory regression, and comparing only the count would render it as a reassuring 0%.

`--fail-on-regression PCT` exits 1 when any median regresses by more than `PCT`. It is opt-in with no default threshold, because a calibrated threshold needs repeated controlled-runner data that does not exist yet ([issue #39]); and it is rejected outright on an environment-drifted comparison, where the deltas describe two machines as much as two revisions.

Baselines live in `benches/baselines/`, outside the `include` allowlist in `Cargo.toml`, so a baseline never ships to a consumer. Committing one is deliberate: updating a baseline should be a reviewed change that records why it moved.

[issue #39]: https://github.com/b3hr4d/candid-core/issues/39

## CI policy

**No timing or allocation measurement ever fails a workflow in this repository.** That is a recorded decision on [issue #39], not an unfinished feature: GitHub-hosted runners are shared and thermally variable, and a threshold calibrated from their noise would produce a red check maintainers learn to ignore. Detection without enforcement — a regression is surfaced with absolute values, intervals, and raw data, and a human decides. The `--fail-on-regression` flag exists for deliberate local pre-release checks on a machine the operator controls; it is never wired into CI.

Three tiers:

| Tier | When | What |
| --- | --- | --- |
| Smoke | Every pull request (`Verify`) | Compiles and exercises every benchmark once, including the allocation probe's measurement path. Correctness of the harness, no timing claims |
| Comparison | Every pull request (`Benchmark comparison`) | Runs the full statistical suite and compares against the reviewed baseline with `compare.py`, publishing a job summary and one bot-managed comment updated in place. Always `--allow-environment-drift`, always informational; a missing or incompatible baseline is reported as "no comparison was made" |
| Capture | Weekly schedule or manual dispatch (`Verify`) | Runs the full suite, uploads Criterion's raw estimates, allocation JSON, environment metadata, and a captured `baseline.json`, retained 90 days |

The comparison workflow lives in its own file because commenting needs `pull-requests: write`, and `Verify` and `Release candidate` deliberately hold nothing beyond `contents: read`.

### Updating the reviewed baseline

The baseline pull requests compare against is `benches/baselines/main.json`, and only a baseline captured on the same runner class as the comparison can ever be environment-compatible — one captured on a maintainer's machine is refused by construction. To update it:

1. Dispatch the `Verify` workflow on `main` and wait for `Scheduled benchmarks`.
2. Download the `benchmarks-<sha>` artifact and review `baseline.json`: the commit it names, the corpus identity, and the estimates themselves.
3. Commit it to `benches/baselines/main.json` with a message recording *why* the baseline moved (new accepted performance level, corpus change, toolchain rollover).

A failing candidate never replaces the reference, and nothing automates this: a baseline update is a reviewed change, deliberately.

## Initial baseline

The first checked-in local measurement is recorded in [`benchmarks/baseline-2026-07-14.md`](benchmarks/baseline-2026-07-14.md). It is a reproducibility example and historical reference, not a portable performance promise.
