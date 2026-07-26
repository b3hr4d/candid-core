# Release verification gates

Verification status is decision-specific. An ADR remains **Implemented, verification pending** until every gate in its required-verification list has recorded evidence. ADR 0002 is **Verified** because the independent-vector gate completed as recorded below; that status does not imply that any other ADR's gates are complete.

## Enforced in this repository

- `Verify` CI runs the declared Rust 1.78 MSRV suite and current stable tests on Linux, macOS, and Windows.
- The feature-matrix job builds and tests every supported feature combination, so a change that only compiles with defaults fails before merge:

  ```sh
  cargo test --all-targets --locked                                                   # defaults
  cargo test --all-targets --locked --no-default-features                             # base model only
  cargo test --all-targets --locked --no-default-features --features host-value
  cargo test --all-targets --locked --no-default-features --features compiler
  cargo test --all-targets --locked --no-default-features --features compiler,host-value
  cargo test --all-targets --locked --no-default-features --features filesystem-compiler
  cargo test --all-targets --locked --all-features
  cargo clippy --all-targets --all-features --locked -- -D warnings
  ```

  Suites with meaningful pure-model coverage (`adr_conformance`, `adversarial_regression`, `api_portability`, `candid_name_hash`, `canonical_properties`, `conformance_vectors`, `contract_foundation`, `diagnostics_contract`, `model_public_api`) build under `--no-default-features` and gate individual cases, so all 11 conformance vectors and the canonicalization properties run with no features at all. Suites that need a feature throughout declare `required-features` in `Cargo.toml` and are skipped rather than emptied. Since [issue #21], the suites whose resolver coverage is `MemoryResolver`-only (`browser_wasm`, `input_bounds`, `provenance_bounds`, `source_identity_bounds`) and the `hermetic_bundle` example declare `compiler` rather than `filesystem-compiler`, so they run in the compiler-only configuration too; the individual `WorkspaceResolver` and `compile_did_file` cases inside them stay gated on `filesystem-compiler`.
- The WASM job builds the library for `wasm32-unknown-unknown` under each feature set that is meant to work there, and lints the browser configuration with warnings denied:

  ```sh
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features host-value
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features compiler
  cargo check --lib --target wasm32-unknown-unknown --locked                        # defaults
  cargo clippy --lib  --target wasm32-unknown-unknown --locked --no-default-features --features compiler -- -D warnings
  cargo clippy --test browser_wasm --target wasm32-unknown-unknown --locked --no-default-features --features compiler -- -D warnings
  ```

  The default build still succeeds on `wasm32-unknown-unknown` because `cap-std` is declared under `cfg(not(target_os = "unknown"))` in addition to being gated on `filesystem-compiler`. These are *build* checks; the runtime claim is the browser job below.
- The browser job is the runtime evidence for [issue #21]. Three things are pinned exactly: `wasm-pack` at `0.14.0`, the browser at Chrome for Testing `150.0.7871.124`, and the ChromeDriver taken from that same Chrome for Testing build (`browser-actions/setup-chrome@v2` with `install-chromedriver: true`, which matches the pair by construction). The driver path is then handed to `wasm-pack` explicitly via `--chromedriver`, which is what makes the pin hold: left to itself, `wasm-pack` downloads its own *latest* ChromeDriver, which can be a different major version from the installed browser and fails the WebDriver session before any test runs. No part of this is a rolling channel, and the job prints both versions before running so the evidence names what it ran on.

  ```sh
  cargo install wasm-pack --version 0.14.0 --locked
  # Both paths come from the pinned setup-chrome step's outputs. `--chrome` is
  # explicit: `--chromedriver` is documented to imply it, but wasm-pack 0.14.0
  # does not apply that implication and exits with a usage error without it.
  wasm-pack test --headless --chrome --chromedriver "$CHROMEDRIVER_PATH" -- --locked --test browser_wasm --no-default-features --features compiler
  ```

  `tests/browser_wasm.rs` compiles a self-contained source and a four-source imported bundle — a type import, an `import service`, and a diamond where one target is reached from two importers — inside the browser; pins its `contract_id`, `interface_id`, `source_bundle_id`, exact logical sources and exact import edges; round-trips the provenance sidecar through `SourceInfo::try_from_raw`; and asserts that an imported service with no main service, an unbound imported type, an unparseable imported source, an unresolvable import, an exhausted `sources` limit, cancellation, and an explicit deadline all fail with stable codes, phases, and resource triples and never leak a materialized name, a temporary directory, or a native path. Every case body is shared: on `wasm32-unknown-unknown` it is a `wasm_bindgen_test` with `run_in_browser`, and on every other target the identical assertions run under the ordinary harness in the native jobs, so the pinned identities cannot drift between the two. The browser dev-dependency (`wasm-bindgen-test =0.3.58`, which pins `wasm-bindgen =0.2.108` and declares `rust-version = "1.71"`) is target-specific, so it never enters the native or MSRV graph. A non-zero exit from `wasm-pack` — a build failure, a browser that will not start, or a failed assertion inside Chrome — fails the job.
- The dependency-boundary job runs `python3 tests/fixtures/packaging/verify_feature_graph.py`, which resolves `cargo metadata` for each feature set and target and asserts that the base graph excludes `candid`, `candid_parser`, `cap-std`, and `ic_principal`; that `host-value` adds `ic_principal` and nothing from the Candid engine; that `compiler` adds the parser stack but no `cap-std`; and that `cap-std` appears only for `filesystem-compiler` on targets that have a filesystem. It follows normal and build edges only, because dev-dependencies never reach a downstream consumer — which is also why the browser harness is invisible to it. This is the *dependency* boundary; `.crate` archive contents are unaffected by feature selection and are separate release-hardening work.
- The two compilation backends are pinned against each other by `src/compile/differential.rs`, a native unit-test module that runs the same `MemoryResolver` bundles through the promoted in-memory backend and through the materialized `candid_parser::check_file` backend `compile_did_file` uses. Valid bundles must produce byte-identical canonical Contracts, identities, and provenance; invalid bundles must produce identical stable diagnostic codes, phases, and resource triples. It covers plain type imports, service plus type imports, diamond and repeated imports, a target reached by both import kinds, recursion, actorless and class actors, and merge, type-check, duplicate-binding, and parse failures.
- The crate's internal Candid name hash is pinned against `candid_parser::candid::idl_hash` by `tests/candid_name_hash.rs` and by unit tests in `src/name_hash.rs`, in every feature configuration including `--no-default-features`. That is what keeps canonical bytes, `contract_id`, and `interface_id` unchanged now that base validation no longer links the parser.

[issue #21]: https://github.com/b3hr4d/candid-core/issues/21
- Property tests cover canonicalization idempotence, input-arena permutation (including a generated-permutation property over a graph with duplicate semantic nodes, an `idl_hash` collision, and mutual recursion), semantically equivalent source ordering, UTF-8/scalar declaration ordering, and the absence of Unicode normalization.
- Checked-in vectors are driven by `tests/fixtures/conformance/manifest.json`, whose required scenario set — actorless, empty actor, class, basic service, recursion, mutual recursion, `idl_hash` collision (with an id-versus-name method-order divergence), Unicode ordering/escaping, duplicate semantic nodes, arena permutations, and declaration-root traversal order (with a strict actor-reachable interface prefix) — is asserted by both the Rust tests and the Python reference, so a dropped scenario fails instead of passing silently. Every vector pins the canonical graph, canonical JSON text and UTF-8 hex, domain preimage, and IDs; the five legacy wire fixtures are additionally compared exactly, without re-canonicalizing them first, and the actorless vector keeps its byte-level pins in `tests/fixtures/conformance/actorless.identity.json`.
- An independent standard-library Python reference canonicalizer — `python3 tests/fixtures/conformance/verify_vectors.py` — recomputes every manifest vector's canonical graph, payload bytes, preimage, and IDs from the raw noncanonical inputs, without the Rust implementation, and the `Verify` workflow runs it as the dedicated `conformance-reference` job. It supersedes the earlier actorless-only `verify_actorless.py`. The recorded result below completes the independent-vector gate for ADR 0002.
- Detached artifact identity ([ADR 0007](adrs/0007-artifact-identity.md)) has its own independent reference and its own `artifact-identity-reference` job, deliberately separate from the closed semantic conformance set above so that set keeps meaning exactly what it did:

  ```sh
  python3 tests/fixtures/artifact-identity/verify_artifact_ids.py
  ```

  It recomputes every vector's `artifact_id` from the exact bytes on disk, pins the whole domain-framing preimage as hex for the empty vector, re-checks that identical bytes under two kinds produce two separately pinned IDs, and asserts that the nine documents embedding one shared `contract_id` — spanning all three kinds, including a raw Contract document and a `ProducerInfo`-rewritten copy of it — have nine distinct artifact IDs. `tests/artifact_identity.rs` drives the same manifest from Rust in the base configuration and additionally pins the raw Contract goldens as literals, and `tests/browser_wasm.rs` pins the framing anchor of each kind inside the browser so the digest cannot differ between native and WASM. `tests/fixtures/artifact-identity/**` is marked `text eol=lf` in `.gitattributes`, because an exact-octet identity cannot survive a checkout that rewrites line endings.
- The adversarial canonicalization test has deterministic work thresholds; a change that omits work charging or crosses the configured limit fails.
- Pull requests compile every fuzz target and replay its tracked seed and regression corpora with `-runs=0`, so a target that stops compiling, or a previously fixed crash that returns, fails on the pull request rather than on the next schedule. The replay performs no mutation and is therefore deterministic. Both fuzz jobs first assert that `fuzz/Cargo.lock` is current, since `cargo fuzz` accepts no `--locked` flag of its own.
- The weekly fuzz job exercises source parsing, Contract JSON, canonicalization, resolver IDs, provenance, HostValue JSON, and envelope parsing, seeded from the tracked corpora. The fuzz crate mirrors the library's features and each target declares the feature that owns the API it drives, so `cargo fuzz build` still builds all seven targets while a reduced feature set builds only the targets that remain meaningful. Both fuzz jobs upload their crash artifacts, so a red run yields a reproducer without re-running locally.
- Pull requests compile and exercise every benchmark once without enforcing wall-clock thresholds. Weekly and manually dispatched runs retain Criterion's raw estimates, allocation measurements, toolchain, host, and exact commit as downloadable CI artifacts.

## Recorded canonicalization v1 evidence

ADR 0002 requires an implementation outside the Rust crate to reproduce every checked-in vector's canonical bytes and IDs. The Rust reference test alone is deliberately insufficient evidence, and CI wiring without a recorded result is not evidence of execution.

| Evidence | Recorded value |
| --- | --- |
| Canonicalization profile | `candid-core-canon-1` |
| Independent implementation | `tests/fixtures/conformance/verify_vectors.py` (Python standard library only; does not call Rust) |
| Exact command | `python3 tests/fixtures/conformance/verify_vectors.py` |
| Required scenarios | 11, asserted by `tests/fixtures/conformance/manifest.json`, Rust, and Python |
| Pull request | [#73](https://github.com/b3hr4d/candid-core/pull/73) |
| Verified PR head | `b6d7c31de3a7ee7ea751d486f597545a19fd988c` |
| Merge commit | `7d29eb03e1a905de66900f2c083707885c1a3963` |
| CI evidence | [Verify run 29834439291](https://github.com/b3hr4d/candid-core/actions/runs/29834439291), including `conformance-reference` ("Independent conformance reference") |
| Result | All 11 canonical graphs, payload bytes, domain preimages, Contract IDs, and interface IDs reproduced; all 8 pull-request jobs succeeded, while 2 schedule-only jobs were skipped by design |

The recorded job counts describe the workflow as it stood for that run. The
feature-matrix and dependency-boundary jobs were added afterwards and do not
affect this record: canonicalization is base-feature behaviour, and the same
`verify_vectors.py` invocation reproduces the same 11 vectors.

This record completes ADR 0002's independent-vector gate. ADRs 0001 and 0003–0007 remain **Implemented, verification pending** until their own required-verification lists are completed and recorded. ADR 0007 ships its own independent Python reference and CI job, but this document records no run of it yet, so wiring is not evidence and its status is unchanged.
