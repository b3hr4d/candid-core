# Changelog

What changed between released versions of `@candid-core/cli`. This file
ships inside the published tarball, so the record travels with the artifact.

**Every entry names the exact revisions it embeds** — the `candid-core`
compiler crate and the `candid-core-ts` generator are compiled into the wasm
artifact from one repository commit, and the pairing is the artifact's
provenance: the parity gates prove the emitted module and contract are
byte-identical to that commit's Rust-native outputs over the golden
fixtures.

`@candid-core/cli` is pre-1.0. Until 1.0 any release may change the CLI
grammar, the library API, and the request/response shapes. Pin an exact
version.

## 0.1.0 — 2026-08-27

Embeds `candid-core` 0.1.0-beta.3 and the `candid-core-ts` generator from
the same repository commit the release is dispatched from; the release
record names the exact SHA.

The first version: the JavaScript-only on-ramp.

- **`gen <service.did> [-o <dir>]`** emits the generated
  `@candid-core/schema` module, the one-document `ContractEnvelope` with the
  `org.candid-core.field-names/v1` field-name table, and prints the content-addressed identities. Imports resolve
  from the entry's directory as an in-memory bundle; every generation
  double-runs and the tool refuses to write on any byte mismatch.
- **`didToContract` / `didToModule`** — the same two operations as a
  library, for Node and browsers alike: one wasm-bindgen artifact over
  `candid-core`'s `compiler` feature (`compile_with_resolver` +
  `MemoryResolver`; no filesystem, no eval, no dynamic import) plus the
  `candid-core-ts` generator. Failures are `{ ok: false, diagnostics }` with
  the compiler's diagnostics verbatim; generator refusals surface as
  `ts_generation_refused`, malformed requests as `invalid_request`.
- **Parity is a gate, not a claim**: CI compares the wasm-built outputs
  byte-for-byte against the repository's reviewed goldens and against the
  native `candid-core compile --envelope` fixture, builds the artifact twice
  from clean and requires identical bytes, and runs `didToContract` inside
  headless Chrome feeding `schemaFromContract`, envelope-carried names
  included.
