# ADR 0006: Use a lossless tagged HostValue ABI

- Status: Implemented
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The next runtime slice will bridge host values and Candid binary messages.
Ordinary JSON values cannot faithfully represent arbitrary `nat`/`int`, all
64-bit integers in JavaScript, floating-point bit patterns, principals,
function/service references, option presence, field IDs, or variant tags.
Choosing a convenient JSON shape first would bake lossy coercions into every
future SDK, form, workflow, and agent tool.

## Decision

`candid-codec` will define a closed, lossless semantic `HostValue` algebra:

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

`empty` has no constructible value. Floating values retain IEEE bits so NaN
payloads and signed zero round-trip. Arbitrary integers use canonical decimal
strings at the JSON boundary. Principals use canonical textual form while the
codec validates and converts their bytes. Record fields use authoritative
`u32` IDs; labels remain provenance. Blob, tuple, and conventional Result are
derived adapters, not new HostValue variants.

The portable JSON ABI is explicitly tagged, for example:

```json
{ "kind": "nat", "value": "340282366920938463463374607431768211456" }
{ "kind": "float64", "bits": "7ff8000000000001" }
{ "kind": "variant", "id": 24860, "value": { "kind": "text", "value": "ok" } }
```

The core validator performs no UI defaults, string-to-number coercion, tuple
guessing, or missing-option repair. Ergonomic adapters may produce explicit
conversion diagnostics. Encoding and decoding always receive a validated
Contract plus `{ contract_id, method_name }` or another contract-bound type
selector. Transport invocation remains outside the codec.

## Consequences

- Values round-trip across Rust, WASM, JavaScript, storage, and agents.
- The portable ABI is verbose by design; UI-friendly JSON is a separate view.
- Core validation errors can use stable value paths and expected/actual kinds.
- Derived conveniences cannot silently change wire semantics.

## Implementation

`HostValue` serializes the tagged JSON ABI, including canonical decimal big
integers, IEEE float bits, principal/service/function values, field IDs, and
variant IDs. It intentionally does not implement serde `Deserialize`: callers
must use `HostValue::from_json_with_limits`, which decodes a private raw DTO
and exposes `HostValue` only after local canonical validation. Public Rust
constructors enforce the same scalar canonical forms. This is a deliberate
pre-release API break from direct enum construction. `validate_host_value`
remains iterative, bounded, and contract-selector-directed; it is a separate
step from local JSON validation. No binary encode/decode or lossy
`serde_json::Value` shortcut is exposed yet.

## Required verification

- Round-trip vectors for every primitive and composite kind.
- Big-integer, integer-boundary, NaN payload, infinity, and signed-zero tests.
- Recursive Contract validation with bounded HostValue depth.
- Cross-language Rust/WASM/TypeScript conformance fixtures.
- Negative tests proving convenience coercions are outside the core codec.
