# Foundation architecture decisions

These Architecture Decision Records define the protocol boundaries that must be implemented before the Contract format is declared stable for external ecosystem use.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-contract-identities.md) | Separate interface, Contract, and source-bundle identities | Implemented, verification pending |
| [0002](0002-versioning-and-canonical-bytes.md) | Version schema, semantics, and canonical bytes independently | Verified |
| [0003](0003-validated-artifact-boundaries.md) | Make validated artifacts and provenance binding explicit | Implemented, verification pending |
| [0004](0004-hermetic-source-resolution.md) | Resolve imports through a hermetic capability boundary | Implemented, verification pending |
| [0005](0005-resource-limits.md) | Bound all untrusted work and avoid recursive execution | Implemented, verification pending |
| [0006](0006-lossless-host-value-abi.md) | Use a lossless tagged HostValue ABI | Implemented, verification pending |

Readiness is recorded per decision. “Implemented, verification pending” means the Rust reference API and serialized envelope enforce that ADR, while one or more gates in its required-verification list still lack recorded evidence. “Verified” means those ADR-specific gates have completed and their evidence is recorded.

ADR 0002 is Verified because the independent standard-library Python implementation reproduced every required canonical graph, payload, preimage, and ID and its dedicated [GitHub CI run](https://github.com/b3hr4d/candid-core/actions/runs/29834439291) passed. That result does not promote ADRs 0001 or 0003–0006; they remain pending until their own required-verification lists are completed and recorded.

These records deliberately keep UI policy, workflows, transports, agent prompts, and derived views such as blob/tuple/Result outside `contract-core`.
