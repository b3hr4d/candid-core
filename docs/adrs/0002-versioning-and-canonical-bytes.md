# ADR 0002: Version schema, semantics, and canonical bytes independently

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

Schema shape, Candid semantics, graph normalization, and canonical-byte rules evolve for different reasons. Coupling them to one version would force a large ecosystem to treat every change as the same kind of break. Identity bytes also need a complete, language-independent protocol specification rather than an implementation-specific serialization convention.

A large ecosystem needs to upgrade syntax, Candid semantics, canonicalization, and hash algorithms without treating every change as the same kind of break.

## Decision

Every persisted Contract envelope will declare:

```json
{
  "format": "candid-core",
  "format_version": 1,
  "semantics_profile": "candid-1",
  "canonicalization_profile": "candid-core-canon-1"
}
```

- `format_version` governs JSON fields and tagged unions.
- `semantics_profile` governs the interpreted Candid type system.
- `canonicalization_profile` governs graph minimization, collection ordering, graph reindexing, canonical bytes, and identity domains.
- Each content ID names its hash algorithm; algorithms are not inferred.
- `producer` metadata records implementation and dependency versions without changing semantic IDs.

For profile `candid-core-canon-1`, the semantic graph is bisimulation-minimized, collections are ordered by the rules in the Contract graph specification, and nodes are deterministically traversed from defined roots. The resulting payload is serialized with RFC 8785 JSON Canonicalization Scheme (JCS). Identity hashes are calculated over a UTF-8 domain prefix, a zero byte, and those JCS bytes.

The canonicalization specification and its golden fixtures are normative. The Rust implementation is a reference implementation, not the specification. Unknown versions or profiles fail closed. Because no earlier format was released, this envelope is adopted directly without compatibility fields or upgrade paths.

## Consequences

- Rust/TypeScript/WASM implementations can independently reproduce IDs.
- Canonical pretty JSON remains presentation only and is never hashed directly.
- A parser bug fix can be recorded in `producer`; a semantic change requires a new semantics profile or an explicit compatibility ruling.
- Profile proliferation is controlled through ADRs and conformance fixtures.

## Implementation

The envelope carries all four profile fields. Domain-separated identity hashes use the `candid-core-canon-1` graph normalization and JCS writer. Canonical Contract fixtures are checked in under `tests/fixtures/conformance`.

## Required verification

- Checked-in canonical payload and digest fixtures.
- Property tests for idempotence and input-arena permutation invariance.
- Cross-language JCS and graph-labeling conformance tests.
- Tests that unknown format, semantics, and canonicalization profiles fail.
