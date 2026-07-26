# ADR 0001: Separate interface, Contract, and source-bundle identities

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

A single digest of the type arena and actor cannot safely serve interface compatibility caches, artifact registries, provenance binding, and human-facing package identity. Declaration-only types and declaration names belong to the complete Contract, while the actor wire interface and source bundle require narrower and broader equality claims respectively.

## Decision

The protocol will expose three domain-separated content identifiers over canonical bytes. Each is a content address over a different projection, and no unkeyed content ID authenticates itself.

1. `interface_id` identifies only the canonical graph reachable from the actor. It is absent for declaration-only Contracts. It is suitable for actor wire interface cache keys, but equality does not imply source or package equality.
2. `contract_id` identifies the complete canonical Contract payload, including actor, declarations, declaration names, and every retained type node. It is the identity to use when the claim being made is *semantic Contract equality*.
3. `source_bundle_id` identifies normalized logical source IDs, source bytes, and their import edges. It belongs to `SourceInfo`, not the semantic Contract, and it identifies only that raw bundle rather than the complete derived sidecar.

The first two are **semantic Contract identities**: they hash a canonical projection of meaning, so they are unmoved by anything that does not change what the Contract means. The third is not — it is a **raw-source bundle content identity**, which is precisely why formatting and comments inside a source do move it.

`contract_id` is deliberately **not** the identity of a produced artifact, and ADR 0007 corrects an earlier sentence here that assigned it to registries, persisted references, signatures, and extension envelopes. It excludes `producer`, it is computed over the Contract alone — so envelope extensions and a `SourceInfo` sidecar are outside it — and canonical bytes are not the only encoding of a Contract. A registry entry, persisted reference, or signature that must commit to a complete *file* commits to that file's `artifact_id` (ADR 0007), alongside `contract_id` when the semantic claim is also wanted.

Identifiers use an explicit domain and profile:

```text
candid-core:interface:v1:sha256:<lowercase hex>
candid-core:contract:v1:sha256:<lowercase hex>
candid-core:source-bundle:v1:sha256:<lowercase hex>
```

The hash input is the domain prefix followed by the canonical bytes selected by ADR 0002. A bare `TypeRef` is document-local. Any persisted or cross-process type reference must be represented as `{ contract_id, type_ref }`. Actor method selection uses `{ contract_id, method_name }`, never a bare function `TypeRef`.

Compiler identity and dependency versions are recorded as producer metadata but do not participate in semantic IDs.

## Consequences

- Adding an actor-unreachable declaration leaves `interface_id` unchanged and changes `contract_id`.
- Renaming a declaration leaves `interface_id` unchanged and changes `contract_id`.
- Among the three identifiers above, formatting and comments affect only `source_bundle_id`: they leave `contract_id` and `interface_id` unchanged, and they move `source_bundle_id` when they are in the raw sources it covers. The scope matters, because these three are not the only identity in the crate — an `artifact_id` (ADR 0007) also moves whenever those changed bytes are part of the exact artifact supplied to it, which for a compilation document they are.
- Rewriting `producer`, editing an envelope extension, replacing a `SourceInfo` sidecar, and re-encoding a document all leave `contract_id` and `interface_id` unchanged. `source_bundle_id` follows its own scope: replacing a sidecar moves it exactly when the raw sources or import edges differ, and a change confined to derived sidecar data need not move it at all. Both behaviours are correct for the claims those IDs make, and are exactly why an artifact needs its own identity; see ADR 0007.
- Consumers must choose the identifier whose equality claim matches their use.
- Compatibility is not inferred from ID inequality; structural compatibility is a separate analysis operation.

## Implementation

The Contract envelope exposes `identities.contract` and optional `identities.interface`; `SourceInfo` exposes `contract_id` and `source_bundle_id`. Contract-bound type and method selectors prevent persisted bare refs.

## Required verification

- Golden tests for all three IDs.
- Tests proving unused declarations affect only `contract_id`.
- Tests proving source-only edits affect only `source_bundle_id` among these three identifiers.
- Cross-language conformance vectors for actorless, empty-actor, class, and recursive Contracts.
