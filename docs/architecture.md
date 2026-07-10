# Candid Contract Runtime — architecture (slice 1)

The implemented [foundation ADRs](adrs/README.md) define the identity,
versioning, validation, source-resolution, resource-limit, and HostValue
boundaries.

This project turns Candid DID source into a small, validated, versioned
**Contract** graph.  A Contract describes Candid's wire-level type semantics;
it is not a UI schema, a value codec, or generated application code.

## Boundary and dependency direction

```text
DID text
  │
  ▼
Rust adapter ──> candid_parser (the authoritative Candid parser/type checker)
  │                     │
  │                     └── parsing, aliases, recursive types, labels,
  │                         function/service/class semantics, diagnostics
  ▼
Contract builder ──> Contract graph validator ──> canonical JSON Contract v1
                                                        │
                                                        ├── future host bridge
                                                        ├── future renderer/forms
                                                        └── future transports
```

The Rust boundary is the only component permitted to parse DID text or apply
Candid type rules.  TypeScript (and any other host) consumes already validated
Contract JSON and must not grow a second handwritten Candid parser, type
checker, or codec.

`candid_parser` is authoritative for the source program's meaning.  The
builder only projects its checked semantic result into the Contract arena.  It
does not reimplement alias resolution, field hashing, recursive-type handling,
function/service references, service-class constructors, or method-mode
validation.

## Contract v1

At a useful level of detail, the wire Contract JSON has this shape (arrays and
object keys are deterministic in its canonical representation):

```json
{
  "format": "candid-contract",
  "format_version": 1,
  "semantics_profile": "candid-1",
  "canonicalization_profile": "ccr-canon-1",
  "contract_version": 1,
  "identities": {
    "contract": "ccr:contract:v1:sha256:<64 lowercase hex>",
    "interface": "ccr:interface:v1:sha256:<64 lowercase hex>"
  },
  "fingerprint": "sha256:<legacy digest>",
  "producer": { "name": "candid-contract-runtime", "version": "..." },
  "types": [
    { "kind": "record", "fields": [
      { "id": 477006482, "type": 0 }
    ] }
  ],
  "declarations": [{ "name": "Account", "type": 0 }],
  "actor": { "kind": "service", "service": 4 }
}
```

`TypeRef` values are zero-based indexes into `types`.  Every edge is direct:
`opt` and `vec` have an inner ref; record and variant fields have a ref;
function arguments/results have refs; service methods have function refs; and
a class has constructor argument refs plus its returned service ref. This makes
recursive and mutually recursive types ordinary graph cycles, not special
string references.

The type arena includes exactly these semantic node families:

| Family | Nodes / contents |
| --- | --- |
| Primitive | `{ "kind": "primitive", "primitive": "nat" }` (and every other Candid primitive) |
| Containers | `{ "kind": "opt" | "vec", "inner": TypeRef }` |
| Aggregates | `record` and `variant` fields `{ id: u32, type: TypeRef }` |
| Calls | `func` argument/result refs and one valid Candid mode |
| Actors | `service` methods `{ name, id, function }`; `class` constructor argument refs and service ref |

All primitives are represented as values of `primitive`: `null`, `bool`,
`nat`, `int`, `nat8`…`nat64`, `int8`…`int64`, `float32`, `float64`, `text`,
`reserved`, `empty`, and `principal`.

`actor` is absent when the DID declares no actor. When present, it is either
`{ "kind": "service", "service": TypeRef }` or
`{ "kind": "class", "class": TypeRef }`. An empty actor is distinct from
no actor: it selects a service node whose `methods` array is empty. A service
class retains its initialization argument types even though it produces a
service.

`declarations` is a provenance-oriented name table over semantic node refs.
It preserves useful named declaration spellings, but a declaration name is not
the identity of a type.  A structural type reachable through two aliases is
still represented by its graph position and edges.

`interface_id` hashes only the canonical actor-reachable graph. `contract_id`
hashes the complete canonical Contract, including declaration names and
retained declaration-only types. Both use domain-separated SHA-256 over JCS
bytes under the named canonicalization profile. `source_bundle_id`
independently hashes logical source URIs, bytes, and import edges. The old
`fingerprint` is retained only for explicit legacy migration.

## Provenance is a sidecar

Optional `SourceInfo` is separate from Contract v1. It carries a bundle of raw
DID sources (including imports and comments), parsed declaration/actor/field/
method documentation, function argument names, and named, numeric, or
positional label spellings.
It is useful for editors and diagnostics but is not sent to
encoders/transports and is bound to `contract_id` rather than embedded in core
identity.

`SourceInfo` is itself versioned and contains `contract_id` and
`source_bundle_id`. `sources` contains `{ name, source }` for
the entry DID and every resolved import. Its declaration entries carry
`{ source, name, type, docs }`; field-label, method, and function-argument
entries carry a source origin plus an AST-shaped `path`, so distinct source
occurrences remain distinguishable even when they lower to one semantic node.
This lets a future view distinguish tuple syntax from an explicit numeric
record label without adding either concept to Contract.

The public upstream Candid AST does not expose stable spans for every semantic
node. v1 therefore preserves raw source plus AST-shaped occurrence paths in
the sidecar and preserves byte spans on parser diagnostics. It intentionally
does not introduce a second handwritten Candid parser just to manufacture
node spans.

This separation is deliberate:

- Contract owns semantic identity and all information necessary to describe
  Candid wire types.
- SourceInfo owns explainability, source presentation, and label spelling.
- Future views own conveniences such as blob detection (`vec nat8`), tuple
  detection (positional records), and conventional `Result` recognition.
- Future UI, form, validation-policy, widget, workflow, and transport layers
  depend on Contract; Contract never depends on them.

## Diagnostics

Loading DID produces either a valid Contract (and optional SourceInfo) or
structured diagnostics.  A diagnostic has a stable category/code, severity,
human-readable message, and optional source range/related locations.  Parser
and semantic errors remain distinguishable so a host can render an actionable
editor error without guessing Candid rules itself.

Malformed Contract JSON is rejected by Contract JSON decoding and graph
validation rather than being silently repaired. Both `Contract::from_json` and
the Rust `Deserialize` boundary validate and canonicalize; a host does not get
an unchecked Contract by taking a normal JSON deserialization path.

## Invariants and ownership rules

- A Contract is self-contained: every `TypeRef` is in bounds and every actor,
  field, argument, result, method, and class edge has the required target kind.
- Interface identity is graph-based and excludes declaration names, comments,
  and source spans. Contract identity includes declaration names; source identity
  includes logical source URIs, bytes, and import edges.
- Record and variant fields retain authoritative Candid `u32` field IDs only.
  The semantic engine, not host code, determines named-label hashes;
  SourceInfo retains label spelling.
- Field IDs are unique. Service method names are unique and each method ID
  equals Candid's hash of its name; distinct method names may legitimately
  share a 32-bit hash, so their text remains authoritative. Method targets are
  `func` nodes and class result targets are `service` nodes. A `class` node is
  valid only as the top-level class actor root; it is not a first-class Candid
  type edge. Canonicalization minimizes semantic equivalents, orders fields
  and methods deterministically, and re-indexes the graph.
- A function has exactly one valid Candid mode: `update`, `query`,
  `composite_query`, or `oneway`; an `oneway` function has no results. No
  arbitrary strings or combinations are accepted.
- The graph may contain cycles.  Validation tracks visited node identities and
  never requires a recursive type to be expanded into a tree.
- Format, semantics, and canonicalization profiles are independently declared.
  Unknown versions or profiles fail closed; legacy JSON requires an explicit
  verified migration.
- Every arena node is reachable from an actor or declaration root (unless the
  arena itself is empty).
- The producer owns construction and identity calculation. Consumers may validate
  and traverse immutable Contract JSON, but must not infer missing semantics.

## Explicit non-goals for this slice

This slice implements the lossless tagged HostValue ABI and graph-directed
validation, but not defaults, coercions, forms, widgets, UI metadata, workflow
projections, transport adapters, agent calls, code generation, or Candid binary
encoding/decoding. It also does not introduce `blob`, `tuple`, or `Result`
nodes: those remain derived semantic views over the canonical graph.

## Next slice

Implement the HostValue \<-> Candid binary bridge. It will accept a validated
Contract plus a contract-bound type or method selector, reuse the implemented
HostValue validator, delegate binary encode/decode to the authoritative Candid
runtime, and return structured diagnostics. It must consume Contract only; it
must not parse DID source or add UI policy.
