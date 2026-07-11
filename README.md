# Candid Core

An early, deliberately narrow runtime foundation for turning Candid DID files
into a canonical validated Contract graph. The Rust core delegates parsing and
type checking to the official `candid_parser` implementation; consumers never
need to parse Candid source or reproduce its type rules.

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- validate ./contract.json
```

The compile command emits JSON containing a canonical validated `contract` and
an optional, identity-bound `source_info` sidecar. The Contract exposes a full
`contract_id` and an actor-only `interface_id`;
source spelling/comments are identified separately by `source_bundle_id`.

See [architecture](docs/architecture.md) and the
[Contract graph](docs/contract-graph.md) for the v1 model, constraints, and
the explicitly deferred host-value ↔ Candid binary bridge.

## Runnable examples

The examples show why the Contract is a graph, how semantically equivalent DID
sources share an identity, and how strict JSON validation protects the core:

```sh
cargo run --example contract_walkthrough
cargo run --example semantic_equivalence
cargo run --example trust_boundary
cargo run --example hermetic_bundle
cargo run --example host_value_validation
```

`contract_walkthrough` prints a canonical recursive Contract and its provenance
summary. `semantic_equivalence` compares interface identity with source identity.
`trust_boundary` demonstrates rejection of injected metadata and a tampered
identity. `hermetic_bundle` shows filesystem-free import resolution, while
`host_value_validation` preserves a large `nat` and an IEEE NaN payload.

## Foundation decisions

Six implemented [foundation ADRs](docs/adrs/README.md) define the boundaries for
large-ecosystem use:

1. separate interface, Contract, and source-bundle identities;
2. independently version schema, Candid semantics, and canonical bytes;
3. make validated artifacts and provenance binding explicit;
4. resolve imports through a hermetic capability boundary;
5. bound all untrusted work; and
6. use a lossless tagged HostValue ABI.

All six decisions are implemented in the Rust reference runtime. Because the
crate has not been released, this profile is the clean starting point rather
than a compatibility layer over an earlier format.

## Platform APIs

- `RawContract` → `Contract::try_from_raw` validates an external artifact.
- `Contract::build_raw` is the producer path that calculates fresh identities.
- `compile_with_resolver` compiles an immutable logical source bundle through
  `MemoryResolver` or sandboxed `WorkspaceResolver`.
- `Limits` and `RuntimeContext` bound untrusted compilation and validation.
- `HostValue` plus `validate_host_value` provide the lossless tagged value ABI.
- `ContractEnvelope` keeps namespaced extensions outside the strict core.
