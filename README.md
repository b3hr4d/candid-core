# Candid Core

An early, deliberately narrow runtime foundation for turning Candid DID files into a canonical validated Contract graph. The Rust core delegates parsing and type checking to the official `candid_parser` implementation; consumers never need to parse Candid source or reproduce its type rules.

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- validate ./contract.json
```

The compile command emits JSON containing a canonical validated `contract` and an optional, identity-bound `source_info` sidecar. The Contract exposes a full `contract_id` and an actor-only `interface_id`; source spelling/comments are identified separately by `source_bundle_id`. That source identity covers only the raw source files and import edges. At an external trust boundary, `SourceInfo::try_from_raw` recompiles that bundle and requires every presented provenance field to match the compiler-derived sidecar.

See [architecture](docs/architecture.md) and the [Contract graph](docs/contract-graph.md) for the v1 model, constraints, and the explicitly deferred host-value ↔ Candid binary bridge. See [release verification gates](docs/verification.md) for the checks required before declaring the format stable across implementations. See [performance benchmarks](docs/benchmarks.md) for reproducible comparisons with the pinned official Candid checker and for allocation measurements.

## Runnable examples

The examples show why the Contract is a graph, how semantically equivalent DID sources share an identity, and how strict JSON validation protects the core:

```sh
cargo run --example contract_walkthrough
cargo run --example semantic_equivalence
cargo run --example trust_boundary
cargo run --example hermetic_bundle
cargo run --example host_value_validation
cargo run --example bounded_parsing
```

`contract_walkthrough` prints a canonical recursive Contract and its provenance summary. `semantic_equivalence` compares interface identity with source identity. `trust_boundary` demonstrates rejection of injected metadata and a tampered identity. `hermetic_bundle` shows filesystem-free import resolution, while `host_value_validation` preserves a large `nat` and an IEEE NaN payload. `bounded_parsing` rejects oversized untrusted documents before decoding and shows the second limit serialization consumes.

## Foundation decisions

Six implemented [foundation ADRs](docs/adrs/README.md) define the boundaries for large-ecosystem use:

1. separate interface, Contract, and source-bundle identities;
2. independently version schema, Candid semantics, and canonical bytes;
3. make validated artifacts and provenance binding explicit;
4. resolve imports through a hermetic capability boundary;
5. bound all untrusted work; and
6. use a lossless tagged HostValue ABI.

All six decisions are implemented in the Rust reference runtime. Because the crate has not been released, this profile is the clean starting point rather than a compatibility layer over an earlier format.

## Rust version and dependencies

The crate advertises Rust 1.78 as its minimum supported Rust version (MSRV). Direct dependencies are pinned to versions that are expected to build on that toolchain, and dependency updates should preserve the advertised MSRV unless the `rust-version` field is intentionally raised in the same change. CI runs the locked dependency graph against Rust 1.78, so an incompatible direct or transitive dependency update fails before merge.

## Platform APIs

- `RawContract` → `Contract::try_from_raw` validates an external artifact.
- `Contract::build_raw` is the producer path that calculates fresh identities.
- `RawSourceInfo` → `SourceInfo::try_from_raw` recompiles the embedded source
  bundle and rejects any derived provenance that does not match exactly.
- `compile_with_resolver` compiles an immutable logical source bundle through `MemoryResolver` or sandboxed `WorkspaceResolver`.
- `Limits` and constructor-based `RuntimeContext` bound untrusted compilation
  and validation with one shared budget, monotonic deadlines, and cooperative
  `CancellationToken` support.
- `HostValue` plus `validate_host_value` provide the lossless tagged value ABI.
- `ContractEnvelope` keeps namespaced extensions outside the strict core.

## Bounded parsing and trusted serde integration

Untrusted bytes and already-validated values take different paths, and the crate does not let the two be confused.

- `Contract`, `Compilation`, `ContractEnvelope`, and `HostValue` do not implement `Deserialize`: a trait impl has no argument position for a resource policy, so it could only ever decode under limits the library chose.
- Untrusted Contract, Compilation, and envelope JSON goes through `from_json_with_limits`/`from_json_with_context` and `from_slice_with_limits`/`from_slice_with_context`. These enforce `max_input_bytes` before decoding and then share one budget with validation. `Contract::from_json` is the same path under `Limits::default`. The byte gate bounds peak decode allocation to a multiple of the caller's ceiling; it does not reject element by element during decode, which remains a follow-up.
- `HostValue` is the exception to that sentence: `HostValue::from_json_with_limits`/`from_json_with_context` gate on `max_value_bytes`, not `max_input_bytes`, and report `HostValueJsonError::Limit`, which carries no `resource` name. Lowering `max_input_bytes` alone does not bound HostValue decoding — lower `max_value_bytes` too.
- `Serialize` and the derived `Deserialize` on the raw DTOs (`RawContract`, `RawSourceInfo`) are the trusted serde integration: they consult no limits and revalidate nothing. Decoding a raw DTO is not a bounded operation, so callers must gate byte length themselves or use a bounded parse API.
- The `to_json_pretty_with_limits`/`to_json_pretty_with_context` serializers on `Contract`, `Compilation`, and `ContractEnvelope` validate before rendering and charge the rendered length against `max_canonicalization_work`. That is a second budget: raising a structural limit such as `max_string_bytes` to build a value does not by itself make that value renderable. (`Compilation` validates its Contract, not its already-authenticated sidecar; rederiving provenance is construction-time work.)

serde_json's own 128-frame recursion limit is the only depth bound that applies *during* decoding, so `Limits::max_value_depth` is not reachable above roughly 127 on any JSON path. The crate's own `max_value_depth` check still runs after decoding, on the materialized value.
