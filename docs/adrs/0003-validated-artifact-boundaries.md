# ADR 0003: Make validated artifacts and provenance binding explicit

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The Rust `Contract` currently has public fields, so callers can construct an invalid value despite its validated name. Its custom deserializer validates and canonicalizes. `Compilation` and `SourceInfo`, however, derive deserialization; if a noncanonical Contract is reindexed during deserialization, sidecar `TypeRef`s are not remapped. A sidecar can also be paired with an unrelated Contract without detection.

Extensions and agents will frequently handle partially trusted JSON. Invalid states must be explicit rather than conventional.

## Decision

The model layer will distinguish:

- `ContractDraft`: producer-side authoring DTO carrying only types,
  declarations, an optional actor, and optional producer metadata — no format
  markers and no identity fields. Building calculates fresh identities; a
  draft cannot carry a fake or placeholder identity because the fields do not
  exist.
- `RawContract`: serde DTO for decoded external artifacts, with no validity claim.
- `Contract`: immutable, validated, canonical Contract with private fields.
- `RawSourceInfo`: serde DTO with no binding claim.
- `SourceInfo`: validated provenance bound to a `contract_id` and `source_bundle_id`.
- `Compilation`: a validated pair constructed atomically from Contract and optional SourceInfo.

Raw-to-validated Contract conversion performs structural validation, canonicalization, and ID verification. Raw-to-validated Compilation conversion performs one coordinated remap, recompiles the authoritative source/import bundle through the same compiler pipeline, and requires the regenerated Contract identity and every provenance collection to match exactly. This verifies declaration and actor origins, named-label spelling and hashes, occurrence paths, method/function relationships, argument names, and documentation instead of merely checking that their references are locally plausible. Ordinary `Deserialize` is implemented only when it can both preserve those guarantees and accept a caller-supplied resource policy; because a trait impl has no argument position for a policy, validated types do not implement it. Untrusted JSON reaches them only through bounded parse entry points that take `Limits` or a `RuntimeContext`; otherwise callers deserialize raw DTOs and gate byte length themselves.

Core structs remain closed with unknown fields rejected. Ecosystem metadata is stored in a separate envelope:

```json
{
  "contract": { "...": "validated core" },
  "extensions": {
    "com.example.form/v1": { "...": "extension-owned data" }
  }
}
```

Extensions are namespaced, versioned, size-limited, and excluded from core identities unless an outer package format explicitly hashes them.

## Consequences

- Safe APIs cannot accidentally expose a malformed Contract.
- Canonicalization cannot silently invalidate SourceInfo.
- Plugins cannot mutate wire semantics through unknown JSON keys.
- Callers doing editors or repair workflows work with explicit raw types.

## Implementation

`Contract` and `Compilation` fields are private and exposed through immutable accessors. `RawContract` and `RawSourceInfo` are explicit DTOs reserved for decoded external artifacts; `ContractDraft` is the producer entry point, and its `build`/`build_with_limits`/`build_with_context` methods validate, canonicalize, and calculate identities under the same budgets as the bounded parse paths, defaulting absent producer metadata to `ProducerInfo::current()`. The earlier `RawContract::new`/`Contract::build_raw` pairing — which fabricated placeholder zero identities that the intuitively paired `Contract::try_from_raw` then rejected — was removed pre-1.0 in favor of that draft type. `Compilation::from_json_with_limits` and its context and slice variants are its only entry points from bytes; they enforce `max_input_bytes` before decoding, then canonicalize the Contract, remap provenance, and rederive the bound sidecar from its source bundle under the same budget. `try_from_raw` and `try_from_raw_with_context` remain for callers who already hold decoded DTOs; they take a policy but no byte gate applies, because there are no bytes left to gate. `Contract` and `ContractEnvelope` expose the same bounded pairs. `Serialize` and the derived `Deserialize` on `RawContract` and `RawSourceInfo` are the trusted serde integration: they consult no limits and revalidate nothing. `source_bundle_id` authenticates only sorted raw sources and import edges; it does not independently identify the derived sidecar. A `SourceInfo` validity claim therefore means that the complete presented sidecar matched rederivation during construction. `ContractEnvelope` owns namespaced extensions.

## Required verification

- Compile-fail or API tests showing validated fields are immutable.
- Adversarial tests for mismatched Contract/SourceInfo pairs.
- Adversarial tests showing every derived provenance collection must match
  compiler rederivation from the embedded source bundle.
- Reindexing tests that prove sidecar references are remapped atomically.
- Extension-envelope tests proving core unknown fields still fail closed.
