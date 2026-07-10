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
