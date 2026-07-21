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

For profile `candid-core-canon-1`, the semantic graph is bisimulation-minimized, collections are ordered by the rules in the Contract graph specification, and nodes are deterministically traversed from defined roots. The resulting payload is serialized with the constrained canonical JSON writer defined in the [canonicalization v1 specification](../canonicalization-v1.md) — byte-identical to RFC 8785 (JCS) for the identity-payload vocabulary, whose object keys are fixed ASCII schema names and whose numbers are all `u32`; the specification states precisely why full JCS UTF-16 property ordering and IEEE-754 number serialization are outside the claimed profile. Identity hashes are calculated over a UTF-8 domain prefix, a zero byte, and those canonical bytes.

The [canonicalization v1 specification](../canonicalization-v1.md) and its golden fixtures (`tests/fixtures/conformance/manifest.json`) are normative. The Rust implementation is a reference implementation, not the specification. Unknown versions or profiles fail closed. Because no earlier format was released, this envelope is adopted directly without compatibility fields or upgrade paths.

## Consequences

- Rust/TypeScript/WASM implementations can independently reproduce IDs.
- Canonical pretty JSON remains presentation only and is never hashed directly.
- A parser bug fix can be recorded in `producer`; a semantic change requires a new semantics profile or an explicit compatibility ruling.
- Profile proliferation is controlled through ADRs and conformance fixtures.

## Implementation

The envelope carries all four profile fields. Domain-separated identity hashes use the `candid-core-canon-1` graph normalization and constrained canonical JSON writer, both specified normatively in [canonicalization v1](../canonicalization-v1.md). Canonical Contract fixtures are checked in under `tests/fixtures/conformance` behind an asserted manifest of required scenarios, and an independent standard-library Python reference (`tests/fixtures/conformance/verify_vectors.py`) recomputes every canonical graph, payload byte, and ID from the raw vectors as a dedicated CI job.

## Required verification

- Checked-in canonical payload and digest fixtures.
- Property tests for idempotence and input-arena permutation invariance.
- Cross-language canonical-bytes and graph-labeling conformance tests.
- Tests that unknown format, semantics, and canonicalization profiles fail.

The status above stays **Implemented, verification pending** until the release gates in [verification](../verification.md) record a GitHub CI result for the independent-implementation job; the local command and CI wiring exist, but the recorded CI evidence does not yet.
