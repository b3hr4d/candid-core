# Release verification gates

Verification status is decision-specific. An ADR remains **Implemented, verification pending** until every gate in its required-verification list has recorded evidence. ADR 0002 is **Verified** because the independent-vector gate completed as recorded below; that status does not imply that any other ADR's gates are complete.

## Enforced in this repository

- `Verify` CI runs the declared Rust 1.78 MSRV suite and current stable tests on Linux, macOS, and Windows.
- The WASM job builds the library for `wasm32-unknown-unknown`.
- Property tests cover canonicalization idempotence, input-arena permutation (including a generated-permutation property over a graph with duplicate semantic nodes, an `idl_hash` collision, and mutual recursion), semantically equivalent source ordering, UTF-8/scalar declaration ordering, and the absence of Unicode normalization.
- Checked-in vectors are driven by `tests/fixtures/conformance/manifest.json`, whose required scenario set — actorless, empty actor, class, basic service, recursion, mutual recursion, `idl_hash` collision (with an id-versus-name method-order divergence), Unicode ordering/escaping, duplicate semantic nodes, arena permutations, and declaration-root traversal order (with a strict actor-reachable interface prefix) — is asserted by both the Rust tests and the Python reference, so a dropped scenario fails instead of passing silently. Every vector pins the canonical graph, canonical JSON text and UTF-8 hex, domain preimage, and IDs; the five legacy wire fixtures are additionally compared exactly, without re-canonicalizing them first, and the actorless vector keeps its byte-level pins in `tests/fixtures/conformance/actorless.identity.json`.
- An independent standard-library Python reference canonicalizer — `python3 tests/fixtures/conformance/verify_vectors.py` — recomputes every manifest vector's canonical graph, payload bytes, preimage, and IDs from the raw noncanonical inputs, without the Rust implementation, and the `Verify` workflow runs it as the dedicated `conformance-reference` job. It supersedes the earlier actorless-only `verify_actorless.py`. The recorded result below completes the independent-vector gate for ADR 0002.
- The adversarial canonicalization test has deterministic work thresholds; a change that omits work charging or crosses the configured limit fails.
- Pull requests compile every fuzz target and replay its tracked seed and regression corpora with `-runs=0`, so a target that stops compiling, or a previously fixed crash that returns, fails on the pull request rather than on the next schedule. The replay performs no mutation and is therefore deterministic. Both fuzz jobs first assert that `fuzz/Cargo.lock` is current, since `cargo fuzz` accepts no `--locked` flag of its own.
- The weekly fuzz job exercises source parsing, Contract JSON, canonicalization, resolver IDs, provenance, HostValue JSON, and envelope parsing, seeded from the tracked corpora. Both fuzz jobs upload their crash artifacts, so a red run yields a reproducer without re-running locally.
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
| CI evidence | [Verify run 29834439291](https://github.com/b3hr4d/candid-core/actions/runs/29834439291), including the dedicated independent-conformance job |
| Result | All 11 canonical graphs, payload bytes, domain preimages, Contract IDs, and interface IDs reproduced; all 8 required CI checks passed |

This record completes ADR 0002's independent-vector gate. ADRs 0001 and 0003–0006 remain **Implemented, verification pending** until their own required-verification lists are completed and recorded.
