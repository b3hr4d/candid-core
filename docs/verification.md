# Release verification gates

The Rust reference runtime is not a stable cross-language protocol until every gate below is satisfied. ADR statuses therefore use **Implemented, verification pending** until the independent-vector gate is complete.

## Enforced in this repository

- `Verify` CI runs the declared Rust 1.78 MSRV suite and current stable tests on Linux, macOS, and Windows.
- The WASM job builds the library for `wasm32-unknown-unknown`.
- Property tests cover canonicalization idempotence, input-arena permutation, and semantically equivalent source ordering.
- Checked-in vectors cover actorless, empty-actor, class, basic service, and recursive Contracts, including every canonical ID.
- The actorless vector is additionally pinned to exact identity bytes (`tests/fixtures/conformance/actorless.identity.json`: canonical JCS text, its UTF-8 bytes as hex, the domain preimage, and the resulting `contract_id`), and the Rust test suite recomputes the pinned-bytes-to-ID link with the hash primitive alone. An independent standard-library Python verifier — `python3 tests/fixtures/conformance/verify_actorless.py` — rebuilds those bytes and that ID from the wire fixture without the Rust implementation; CI does not run it, so that reproduction is a manual command, and it is deliberately narrower than the cross-language gate below, which still requires a full second implementation.
- The adversarial canonicalization test has deterministic work thresholds; a change that omits work charging or crosses the configured limit fails.
- Pull requests compile every fuzz target and replay its tracked seed and regression corpora with `-runs=0`, so a target that stops compiling, or a previously fixed crash that returns, fails on the pull request rather than on the next schedule. The replay performs no mutation and is therefore deterministic. Both fuzz jobs first assert that `fuzz/Cargo.lock` is current, since `cargo fuzz` accepts no `--locked` flag of its own.
- The weekly fuzz job exercises source parsing, Contract JSON, canonicalization, resolver IDs, provenance, HostValue JSON, and envelope parsing, seeded from the tracked corpora. Both fuzz jobs upload their crash artifacts, so a red run yields a reproducer without re-running locally.
- Pull requests compile and exercise every benchmark once without enforcing wall-clock thresholds. Weekly and manually dispatched runs retain Criterion's raw estimates, allocation measurements, toolchain, host, and exact commit as downloadable CI artifacts.

## Required before a stable format declaration

An implementation outside this Rust crate must reproduce every checked-in vector's canonical bytes and IDs. Add its source, exact command, and CI result here before changing any ADR status to **Verified**. The Rust reference test alone is deliberately insufficient evidence of cross-language conformance.
