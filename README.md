# Candid Contract Runtime

An early, deliberately narrow runtime foundation for turning Candid DID files
into a canonical validated Contract graph. The Rust core delegates parsing and
type checking to the official `candid_parser` implementation; consumers never
need to parse Candid source or reproduce its type rules.

```sh
cargo run --bin candid-contract -- compile ./service.did
cargo run --bin candid-contract -- validate ./contract.json
```

The compile command emits JSON containing a canonical `contract` and an
optional `source_info` sidecar. The Contract carries only wire semantics;
source spelling/comments are outside its fingerprint.

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
```

`contract_walkthrough` prints a canonical recursive Contract and its provenance
summary. `semantic_equivalence` compares wire identity with source identity.
`trust_boundary` demonstrates rejection of injected metadata and a tampered
fingerprint.

## Foundation decisions

Six accepted [foundation ADRs](docs/adrs/README.md) define the boundaries that
must be implemented before the format is declared stable for a large ecosystem:

1. separate interface, Contract, and source-bundle identities;
2. independently version schema, Candid semantics, and canonical bytes;
3. make validated artifacts and provenance binding explicit;
4. resolve imports through a hermetic capability boundary;
5. bound all untrusted work; and
6. use a lossless tagged HostValue ABI.

The current JSON remains pre-stable while those migrations are implemented.
