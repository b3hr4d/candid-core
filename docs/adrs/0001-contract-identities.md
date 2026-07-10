# ADR 0001: Separate interface, Contract, and source-bundle identities

- Status: Implemented
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The current `fingerprint` hashes the canonical type arena and actor but omits
declaration names and source provenance. Because the arena also contains types
rooted only by declarations, an unused novel declaration changes the
fingerprint even when the actor wire interface is unchanged. Conversely, an
additional alias can change the Contract document without changing the
fingerprint.

One identifier therefore cannot safely serve interface compatibility caches,
artifact registries, provenance binding, and human-facing package identity.

## Decision

The protocol will expose three domain-separated content identifiers:

1. `interface_id` identifies only the canonical graph reachable from the actor.
   It is absent for declaration-only Contracts. It is suitable for actor wire
   interface cache keys, but equality does not imply source or package equality.
2. `contract_id` identifies the complete canonical Contract payload, including
   actor, declarations, declaration names, and every retained type node. It is
   the identity used by registries, persisted references, signatures, and
   extension envelopes.
3. `source_bundle_id` identifies normalized logical source IDs, source bytes,
   and their import edges. It belongs to `SourceInfo`, not the semantic Contract.

Identifiers use an explicit domain and profile:

```text
ccr:interface:v1:sha256:<lowercase hex>
ccr:contract:v1:sha256:<lowercase hex>
ccr:source-bundle:v1:sha256:<lowercase hex>
```

The hash input is the domain prefix followed by the canonical bytes selected
by ADR 0002. A bare `TypeRef` is document-local. Any persisted or cross-process
type reference must be represented as `{ contract_id, type_ref }`. Actor method
selection uses `{ contract_id, method_name }`, never a bare function `TypeRef`.

Compiler identity and dependency versions are recorded as producer metadata
but do not participate in semantic IDs.

## Consequences

- Adding an actor-unreachable declaration leaves `interface_id` unchanged and
  changes `contract_id`.
- Renaming a declaration leaves `interface_id` unchanged and changes
  `contract_id`.
- Formatting and comments affect only `source_bundle_id`.
- Consumers must choose the identifier whose equality claim matches their use.
- Compatibility is not inferred from ID inequality; structural compatibility
  is a separate analysis operation.

## Implementation

The Contract envelope exposes `identities.contract` and optional
`identities.interface`; `SourceInfo` exposes `contract_id` and
`source_bundle_id`. Contract-bound type and method selectors prevent persisted
bare refs. The legacy `fingerprint` remains readable only for explicit migration
and compatibility diagnostics.

## Required verification

- Golden tests for all three IDs.
- Tests proving unused declarations affect only `contract_id`.
- Tests proving source-only edits affect only `source_bundle_id`.
- Cross-language conformance vectors for actorless, empty-actor, class, and
  recursive Contracts.
