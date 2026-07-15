# Release verification gates

The Rust reference runtime is not a stable cross-language protocol until every gate below is satisfied. ADR statuses therefore use **Implemented, verification pending** until the independent-vector gate is complete.

## Enforced in this repository

- `Verify` CI runs the declared Rust 1.78 MSRV suite and current stable tests on Linux, macOS, and Windows.
- The WASM job builds the library for `wasm32-unknown-unknown`.
- Property tests cover canonicalization idempotence, input-arena permutation, and semantically equivalent source ordering.
- Checked-in vectors cover actorless, empty-actor, class, basic service, and recursive Contracts, including every canonical ID.
- The adversarial canonicalization test has deterministic work thresholds; a change that omits work charging or crosses the configured limit fails.
- The weekly fuzz job exercises source parsing, Contract JSON, canonicalization, resolver IDs, provenance, and HostValue JSON.
- Pull requests compile and exercise every benchmark once without enforcing wall-clock thresholds. Weekly and manually dispatched runs retain Criterion's raw estimates, allocation measurements, toolchain, host, and exact commit as downloadable CI artifacts.

## Required before a stable format declaration

An implementation outside this Rust crate must reproduce every checked-in vector's canonical bytes and IDs. Add its source, exact command, and CI result here before changing any ADR status to **Verified**. The Rust reference test alone is deliberately insufficient evidence of cross-language conformance.
