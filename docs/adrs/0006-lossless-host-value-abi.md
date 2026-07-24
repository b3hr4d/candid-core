# ADR 0006: Use a lossless tagged HostValue ABI

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The next runtime slice will bridge host values and Candid binary messages. Ordinary JSON values cannot faithfully represent arbitrary `nat`/`int`, all 64-bit integers in JavaScript, floating-point bit patterns, principals, function/service references, option presence, field IDs, or variant tags. Choosing a convenient JSON shape first would bake lossy coercions into every future SDK, form, workflow, and agent tool.

## Decision

A closed, lossless semantic `HostValue` algebra (implemented in `candid-core` behind the `host-value` feature, which is enabled by default):

```text
null · bool
nat(decimal) · int(decimal)
nat8/16/32/64 · int8/16/32/64
float32(bits) · float64(bits)
text · reserved · principal
opt(none|some) · vec(values)
record([{ id, value }]) · variant({ id, value })
service(principal) · func({ principal, method })
```

`empty` has no constructible value. Floating values retain IEEE bits so NaN payloads and signed zero round-trip. Arbitrary integers use canonical decimal strings at the JSON boundary. Principals use canonical textual form while the codec validates and converts their bytes. Record fields use authoritative `u32` IDs; labels remain provenance. Blob, tuple, and conventional Result are derived adapters, not new HostValue variants.

The portable JSON ABI is explicitly tagged, for example:

```json
{ "kind": "nat", "value": "340282366920938463463374607431768211456" }
{ "kind": "float64", "bits": "7ff8000000000001" }
{ "kind": "variant", "id": 24860, "value": { "kind": "text", "value": "ok" } }
```

The core validator performs no UI defaults, string-to-number coercion, tuple guessing, or missing-option repair. Ergonomic adapters may produce explicit conversion diagnostics. Encoding and decoding always receive a validated Contract plus `{ contract_id, method_name }` or another contract-bound type selector. Transport invocation remains outside the codec.

## Consequences

- Values round-trip across Rust, WASM, JavaScript, storage, and agents.
- The ABI is usable without a Candid source engine: `host-value` adds only
  `ic_principal` (for canonical principal text) to the base graph, so a host
  that validates values but never compiles DID source builds no parser.
- The portable ABI is verbose by design; UI-friendly JSON is a separate view.
- Core validation errors can use stable value paths and expected/actual kinds.
- Derived conveniences cannot silently change wire semantics.

## Implementation

`HostValue` and everything named in this ADR live behind the `host-value`
feature. Principal text is checked with `ic_principal` taken as a direct
dependency; `candid::Principal` is a plain `pub use` of the same type, so the
accepted and rejected principal text, the error variants, and their rendered
messages are unchanged — only the path into the dependency graph is. Packaging
`HostValue` as an independent crate is deliberately **deferred**: issue #24
isolates it as a feature so that decision stays open, and taking it now would
mean a second published package, a second version line, and a cross-crate
`Contract` dependency before the ABI has any downstream users.

`HostValue` serializes the tagged JSON ABI, including canonical decimal big integers, IEEE float bits, principal/service/function values, field IDs, and variant IDs. It intentionally does not implement serde `Deserialize`: callers must use `HostValue::from_json_with_limits`, which decodes a private raw DTO and exposes `HostValue` only after local canonical validation. Public Rust constructors enforce the same scalar canonical forms. This is a deliberate pre-release API break from direct enum construction. No binary encode/decode or lossy `serde_json::Value` shortcut is exposed yet.

`HostValue` is recursive, so every operation on it — decoding, constructing, cloning, comparing, formatting, dropping, and serializing — walks one stack frame per level, and an unbounded value aborts the process rather than returning an error. Two bounds keep those walks finite. `max_value_nesting` bounds lexical JSON container nesting in a constant-stack scan that runs before `serde_json`'s recursive decoder, so hostile input is rejected by a `value_nesting` resource limit; serde_json's fixed 128-frame ceiling is retained unmodified beneath it as a second line of defence. `max_value_depth` and `max_value_elements` bound the semantic value, and the container constructors take a `&Limits` and fail closed, because construction is the only chokepoint that also covers `Drop`, `Clone`, `PartialEq`, and `Debug` — none of which can signal failure.

`validate_host_value` is contract-selector-directed and bounded by `max_value_depth`; it is a separate step from local JSON validation. It descends recursively rather than with an explicit work stack, which is safe only because that depth bound and the construction bound above make an over-deep `HostValue` unobtainable. Converting it to an explicit stack would remove that dependency and is tracked separately.

## Required verification

- Round-trip vectors for every primitive and composite kind.
- Big-integer, integer-boundary, NaN payload, infinity, and signed-zero tests.
- Recursive Contract validation with bounded HostValue depth.
- Cross-language Rust/WASM/TypeScript conformance fixtures.
- A dependency-graph check that a `host-value`-only build contains
  `ic_principal` and no Candid source engine
  (`tests/fixtures/packaging/verify_feature_graph.py`).
- Negative tests proving convenience coercions are outside the core codec.
