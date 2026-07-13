# ADR 0003: Make validated artifacts and provenance binding explicit

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The Rust `Contract` currently has public fields, so callers can construct an
invalid value despite its validated name. Its custom deserializer validates and
canonicalizes. `Compilation` and `SourceInfo`, however, derive deserialization;
if a noncanonical Contract is reindexed during deserialization, sidecar
`TypeRef`s are not remapped. A sidecar can also be paired with an unrelated
Contract without detection.

Extensions and agents will frequently handle partially trusted JSON. Invalid
states must be explicit rather than conventional.

## Decision

The model layer will distinguish:

- `RawContract`: serde DTO with no validity claim.
- `Contract`: immutable, validated, canonical Contract with private fields.
- `RawSourceInfo`: serde DTO with no binding claim.
- `SourceInfo`: validated provenance bound to a `contract_id` and
  `source_bundle_id`.
- `Compilation`: a validated pair constructed atomically from Contract and
  optional SourceInfo.

`TryFrom<RawContract>` performs structural validation, canonicalization, and ID
verification. `TryFrom<(RawContract, RawSourceInfo)>` performs one coordinated
remap and validates every provenance reference, node kind, source origin,
position, and occurrence path. Ordinary `Deserialize` is implemented only when
it can preserve those guarantees; otherwise callers deserialize raw DTOs.

Core structs remain closed with unknown fields rejected. Ecosystem metadata is
stored in a separate envelope:

```json
{
  "contract": { "...": "validated core" },
  "extensions": {
    "com.example.form/v1": { "...": "extension-owned data" }
  }
}
```

Extensions are namespaced, versioned, size-limited, and excluded from core
identities unless an outer package format explicitly hashes them.

## Consequences

- Safe APIs cannot accidentally expose a malformed Contract.
- Canonicalization cannot silently invalidate SourceInfo.
- Plugins cannot mutate wire semantics through unknown JSON keys.
- Callers doing editors or repair workflows work with explicit raw types.

## Implementation

`Contract` and `Compilation` fields are private and exposed through immutable
accessors. `RawContract` and `RawSourceInfo` are explicit DTOs. Compilation has
a coordinated deserializer that canonicalizes the Contract, remaps provenance,
and validates the bound sidecar. `ContractEnvelope` owns namespaced extensions.

## Required verification

- Compile-fail or API tests showing validated fields are immutable.
- Adversarial tests for mismatched Contract/SourceInfo pairs.
- Reindexing tests that prove sidecar references are remapped atomically.
- Extension-envelope tests proving core unknown fields still fail closed.
