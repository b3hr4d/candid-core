# Foundation architecture decisions

These Architecture Decision Records define the protocol boundaries that must
be implemented before the Contract format is declared stable for external
ecosystem use.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-contract-identities.md) | Separate interface, Contract, and source-bundle identities | Implemented |
| [0002](0002-versioning-and-canonical-bytes.md) | Version schema, semantics, and canonical bytes independently | Implemented |
| [0003](0003-validated-artifact-boundaries.md) | Make validated artifacts and provenance binding explicit | Implemented |
| [0004](0004-hermetic-source-resolution.md) | Resolve imports through a hermetic capability boundary | Implemented |
| [0005](0005-resource-limits.md) | Bound all untrusted work and avoid recursive execution | Implemented |
| [0006](0006-lossless-host-value-abi.md) | Use a lossless tagged HostValue ABI | Implemented |

“Implemented” means the Rust reference API, serialized envelope, compatibility
bridge, and conformance tests enforce the decision. The checked-in JSON fixtures
are portable inputs for future TypeScript/WASM implementations.

These records deliberately keep UI policy, workflows, transports, agent
prompts, and derived views such as blob/tuple/Result outside `contract-core`.
