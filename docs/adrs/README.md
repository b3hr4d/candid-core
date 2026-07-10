# Foundation architecture decisions

These Architecture Decision Records define the protocol boundaries that must
be implemented before the Contract format is declared stable for external
ecosystem use.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-contract-identities.md) | Separate interface, Contract, and source-bundle identities | Accepted; implementation pending |
| [0002](0002-versioning-and-canonical-bytes.md) | Version schema, semantics, and canonical bytes independently | Accepted; implementation pending |
| [0003](0003-validated-artifact-boundaries.md) | Make validated artifacts and provenance binding explicit | Accepted; implementation pending |
| [0004](0004-hermetic-source-resolution.md) | Resolve imports through a hermetic capability boundary | Accepted; implementation pending |
| [0005](0005-resource-limits.md) | Bound all untrusted work and avoid recursive execution | Accepted; implementation pending |
| [0006](0006-lossless-host-value-abi.md) | Use a lossless tagged HostValue ABI | Accepted; implementation pending |

“Accepted; implementation pending” means the decision is settled and new work
must conform to it, but the current pre-stable Rust API and Contract v1 JSON
have not yet completed the migration. Each ADR identifies the compatibility
bridge and the tests required before its status can become `Implemented`.

These records deliberately keep UI policy, workflows, transports, agent
prompts, and derived views such as blob/tuple/Result outside `contract-core`.

