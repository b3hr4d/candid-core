# ADR 0002: Version schema, semantics, and canonical bytes independently

- Status: Implemented
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

`contract_version` currently selects the JSON shape, graph normalization, and
fingerprint behavior together. The hash is produced from Rust struct
serialization after canonical graph traversal. That is deterministic inside
this crate, but it is not yet a complete, language-independent protocol
specification.

A large ecosystem needs to upgrade syntax, Candid semantics, canonicalization,
and hash algorithms without treating every change as the same kind of break.

## Decision

Every persisted Contract envelope will declare:

```json
{
  "format": "candid-contract",
  "format_version": 1,
  "semantics_profile": "candid-1",
  "canonicalization_profile": "ccr-canon-1"
}
```

- `format_version` governs JSON fields and tagged unions.
- `semantics_profile` governs the interpreted Candid type system.
- `canonicalization_profile` governs graph minimization, collection ordering,
  graph reindexing, canonical bytes, and identity domains.
- Each content ID names its hash algorithm; algorithms are not inferred.
- `producer` metadata records implementation and dependency versions without
  changing semantic IDs.

For profile `ccr-canon-1`, the semantic graph is bisimulation-minimized,
collections are ordered by the rules in the Contract graph specification, and
nodes are deterministically traversed from defined roots. The resulting payload
is serialized with RFC 8785 JSON Canonicalization Scheme (JCS). Identity hashes
are calculated over a UTF-8 domain prefix, a zero byte, and those JCS bytes.

The canonicalization specification and its golden fixtures are normative. The
Rust implementation is a reference implementation, not the specification.
Unknown versions or profiles fail closed. Explicit migration functions produce
a new artifact and report any information loss; ordinary deserialization never
silently upgrades versions.

## Consequences

- Rust/TypeScript/WASM implementations can independently reproduce IDs.
- Canonical pretty JSON remains presentation only and is never hashed directly.
- A parser bug fix can be recorded in `producer`; a semantic change requires a
  new semantics profile or an explicit compatibility ruling.
- Profile proliferation is controlled through ADRs and conformance fixtures.

## Implementation

The envelope now carries all four profile fields. Domain-separated identity
hashes use the `ccr-canon-1` graph normalization and JCS writer. Legacy JSON is
rejected by normal decoding and accepted only through
`migrate_legacy_v1_json`, which verifies its old fingerprint before producing a
new artifact. Canonical Contract fixtures are checked in under
`tests/fixtures/conformance`.

## Required verification

- Checked-in canonical payload and digest fixtures.
- Property tests for idempotence and input-arena permutation invariance.
- Cross-language JCS and graph-labeling conformance tests.
- Tests that unknown format, semantics, and canonicalization profiles fail.
