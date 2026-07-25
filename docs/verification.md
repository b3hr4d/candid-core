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

  Suites with meaningful pure-model coverage (`adr_conformance`, `adversarial_regression`, `api_portability`, `candid_name_hash`, `canonical_properties`, `conformance_vectors`, `contract_foundation`, `diagnostics_contract`, `model_public_api`) build under `--no-default-features` and gate individual cases, so all 11 conformance vectors and the canonicalization properties run with no features at all. Suites that need a feature throughout declare `required-features` in `Cargo.toml` and are skipped rather than emptied.
- The WASM job builds the library for `wasm32-unknown-unknown` under each feature set that is meant to work there:

  ```sh
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features host-value
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features compiler
  cargo check --lib --target wasm32-unknown-unknown --locked                        # defaults
  ```

  The default build still succeeds on `wasm32-unknown-unknown` because `cap-std` is declared under `cfg(not(target_os = "unknown"))` in addition to being gated on `filesystem-compiler`. That is a *build* check, not a runtime claim: no browser runtime testing is asserted here, and imported-bundle compilation in browsers remains [issue #21]'s subject.
- The dependency-boundary job runs `python3 tests/fixtures/packaging/verify_feature_graph.py`, which resolves `cargo metadata` for each feature set and target and asserts that the base graph excludes `candid`, `candid_parser`, `cap-std`, and `ic_principal`; that `host-value` adds `ic_principal` and nothing from the Candid engine; that `compiler` adds the parser stack but no `cap-std`; and that `cap-std` appears only for `filesystem-compiler` on targets that have a filesystem. It follows normal and build edges only, because dev-dependencies never reach a downstream consumer. This is the *dependency* boundary; `.crate` archive contents are unaffected by feature selection and are separate release-hardening work.
- The crate's internal Candid name hash is pinned against `candid_parser::candid::idl_hash` by `tests/candid_name_hash.rs` and by unit tests in `src/name_hash.rs`, in every feature configuration including `--no-default-features`. That is what keeps canonical bytes, `contract_id`, and `interface_id` unchanged now that base validation no longer links the parser.

[issue #21]: https://github.com/b3hr4d/candid-core/issues/21
- Property tests cover canonicalization idempotence, input-arena permutation (including a generated-permutation property over a graph with duplicate semantic nodes, an `idl_hash` collision, and mutual recursion), semantically equivalent source ordering, UTF-8/scalar declaration ordering, and the absence of Unicode normalization.
- Checked-in vectors are driven by `tests/fixtures/conformance/manifest.json`, whose required scenario set — actorless, empty actor, class, basic service, recursion, mutual recursion, `idl_hash` collision (with an id-versus-name method-order divergence), Unicode ordering/escaping, duplicate semantic nodes, arena permutations, and declaration-root traversal order (with a strict actor-reachable interface prefix) — is asserted by both the Rust tests and the Python reference, so a dropped scenario fails instead of passing silently. Every vector pins the canonical graph, canonical JSON text and UTF-8 hex, domain preimage, and IDs; the five legacy wire fixtures are additionally compared exactly, without re-canonicalizing them first, and the actorless vector keeps its byte-level pins in `tests/fixtures/conformance/actorless.identity.json`.
- An independent standard-library Python reference canonicalizer — `python3 tests/fixtures/conformance/verify_vectors.py` — recomputes every manifest vector's canonical graph, payload bytes, preimage, and IDs from the raw noncanonical inputs, without the Rust implementation, and the `Verify` workflow runs it as the dedicated `conformance-reference` job. It supersedes the earlier actorless-only `verify_actorless.py`. The recorded result below completes the independent-vector gate for ADR 0002.
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

This record completes ADR 0002's independent-vector gate. ADRs 0001 and 0003–0006 remain **Implemented, verification pending** until their own required-verification lists are completed and recorded.
