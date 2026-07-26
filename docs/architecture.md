# Candid Core — architecture (slice 1)

The implemented [foundation ADRs](adrs/README.md) define the identity, versioning, validation, source-resolution, resource-limit, and HostValue boundaries.

This project turns Candid DID source into a small, validated, versioned **Contract** graph.  A Contract describes Candid's wire-level type semantics; it is not a UI schema, a value codec, or generated application code.

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

The Rust boundary is the only component permitted to parse DID text or apply Candid type rules, and only when the `compiler` feature is enabled (it is by default).  TypeScript (and any other host) consumes already validated Contract JSON and must not grow a second handwritten Candid parser, type checker, or codec.

`candid_parser` is authoritative for the source program's meaning.  The builder only projects its checked semantic result into the Contract arena.  It does not reimplement alias resolution, field hashing, recursive-type handling, function/service references, service-class constructors, or method-mode validation.

### Feature layering

The diagram above is also the dependency layering, and it is enforced by Cargo features rather than by convention. Everything above the "Contract builder" line is the `compiler` feature; everything below it is the base a `default-features = false` consumer gets. All features are enabled by default, so this changes nothing for an existing dependency.

```text
                       cap-std                candid + candid_parser
                          │                            │
filesystem-compiler ──────┘                            │
  WorkspaceResolver, compile_did_file,                 │
  materialization + check_file, CLI                    │
        │                                              │
        └── implies ──> compiler ──────────────────────┘
                          compile_did*, compile_with_resolver,
                          Compilation, SourceId/SourceResolver/
                          MemoryResolver, SourceInfo
                                    │
                                    ▼
  host-value ──> ic_principal    (base)  serde, serde_json, sha2, hex
    HostValue, validate_host_value,        Contract, ContractDraft, RawContract,
    Contract::bind_type/bind_method        validation, canonicalization,
                                           identities, Limits/RuntimeContext,
                                           Diagnostic, ContractEnvelope
```

Three consequences are load-bearing:

* **The base validates Candid method IDs without a Candid source engine.** Validation used to call `candid_parser::candid::idl_hash`; it now calls a normative eight-line implementation of the same specified function (`src/name_hash.rs`), pinned against the upstream reference by `tests/candid_name_hash.rs` and by unit tests, in every feature configuration. Canonical bytes, `contract_id`, and `interface_id` are unchanged by construction.
* **Provenance is `compiler` surface.** `SourceInfo`/`RawSourceInfo` and the `Source*` types live behind `compiler` because authenticating a presented sidecar means recompiling its embedded bundle — that is compiler logic, not model logic. The rederivation path reconstructs one virtual merged program in memory and never touches a filesystem, which is why `SourceInfo::try_from_raw` works under `compiler` alone.
* **Imported-bundle compilation is `compiler` surface too** ([issue #21]). `compile_with_resolver` loads the bundle once through the resolver, merges it into one virtual program with the public `candid_parser` merged-program APIs (`IDLMergedProg::new`/`merge`/`decs`/`resolve_actor`), type-checks it with `check_prog`, and lowers it — the same backend provenance rederivation uses, so the two cannot drift. `filesystem-compiler` is what a *native file* caller needs: `compile_did_file` reads through `WorkspaceResolver` and keeps `candid_parser::check_file` over a materialized copy of the bundle as its authority. `src/compile/differential.rs` compares the two backends on the same logical bundles and requires byte-identical Contracts and provenance for valid input and identical stable codes and phases for invalid input.

[issue #21]: https://github.com/b3hr4d/candid-core/issues/21

`ic_principal` is a direct dependency rather than a re-export borrowed from `candid_parser::Principal`. `candid::Principal` *is* `ic_principal::Principal` — a plain `pub use` — so accepted and rejected principal text, the error variants, and their rendered messages are unchanged; taking it directly is what keeps a host that only validates values out of the parser stack.

`tests/fixtures/packaging/verify_feature_graph.py` checks these claims against `cargo metadata` for each feature set and for `wasm32-unknown-unknown`, following only normal and build edges (dev-dependencies never reach a downstream consumer).

Two limits of the mechanism are worth stating plainly. Cargo **unifies** features across a build graph: if any other crate in a build depends on `candid-core` with defaults, the full surface is compiled once for everyone in that build, so feature selection bounds what a dependency graph *must* contain rather than what a mixed graph produces. And feature selection does not change the published `.crate` archive — every source file ships regardless — which is separate release-hardening work.

## Contract v1

At a useful level of detail, the wire Contract JSON has this shape (arrays and object keys are deterministic in its canonical representation):

```json
{
  "format": "candid-core",
  "format_version": 1,
  "semantics_profile": "candid-1",
  "canonicalization_profile": "candid-core-canon-1",
  "identities": {
    "contract": "candid-core:contract:v1:sha256:<64 lowercase hex>",
    "interface": "candid-core:interface:v1:sha256:<64 lowercase hex>"
  },
  "producer": { "name": "candid-core", "version": "..." },
  "types": [
    { "kind": "record", "fields": [
      { "id": 477006482, "type": 0 }
    ] }
  ],
  "declarations": [{ "name": "Account", "type": 0 }],
  "actor": { "kind": "service", "service": 4 }
}
```

`TypeRef` values are zero-based indexes into `types`.  Every edge is direct: `opt` and `vec` have an inner ref; record and variant fields have a ref; function arguments/results have refs; service methods have function refs; and a class has constructor argument refs plus its returned service ref. This makes recursive and mutually recursive types ordinary graph cycles, not special string references.

The type arena includes exactly these semantic node families:

| Family | Nodes / contents |
| --- | --- |
| Primitive | `{ "kind": "primitive", "primitive": "nat" }` (and every other Candid primitive) |
| Containers | `{ "kind": "opt" | "vec", "inner": TypeRef }` |
| Aggregates | `record` and `variant` fields `{ id: u32, type: TypeRef }` |
| Calls | `func` argument/result refs and one valid Candid mode |
| Actors | `service` methods `{ name, id, function }`; `class` constructor argument refs and service ref |

All primitives are represented as values of `primitive`: `null`, `bool`, `nat`, `int`, `nat8`…`nat64`, `int8`…`int64`, `float32`, `float64`, `text`, `reserved`, `empty`, and `principal`.

`actor` is omitted when the DID declares no actor: the property is absent from canonical Contract JSON and from the `contract_id` identity payload alike, and decoding rejects an explicit `"actor": null` instead of treating it as a second spelling of absence. When present, it is either `{ "kind": "service", "service": TypeRef }` or `{ "kind": "class", "class": TypeRef }`. An empty actor is distinct from no actor: it selects a service node whose `methods` array is empty. A service class retains its initialization argument types even though it produces a service.

`declarations` is a provenance-oriented name table over semantic node refs. It preserves useful named declaration spellings, but a declaration name is not the identity of a type.  A structural type reachable through two aliases is still represented by its graph position and edges.

`interface_id` hashes only the canonical actor-reachable graph. `contract_id` hashes the complete canonical Contract, including declaration names and retained declaration-only types. Both use domain-separated SHA-256 over JCS bytes under the named canonicalization profile. `source_bundle_id` independently hashes logical source URIs, bytes, and import edges.

### What each identity claims

Those three are not one family, and the distinction is load-bearing. `contract_id` and `interface_id` are **semantic Contract identities**: each hashes a canonical projection of *meaning*, so inputs that mean the same thing collide on purpose. `source_bundle_id` is a **raw-source bundle content identity**: it does identify raw source-file content, covering the source bytes and import edges, so reformatting a source or editing a comment inside one does move it, while data *derived* from those sources never enters it. What none of the three identifies is a complete serialized `Contract`, `ContractEnvelope`, or `Compilation` document. A fourth, `artifact_id`, is a **detached exact-octet identity** for a declared artifact kind. No unkeyed content ID authenticates itself — these are content addresses over different projections, and signing one commits to exactly what its row lists and nothing more.

| ID | Kind of identity | Exactly what is covered | Equality means | Excluded |
| --- | --- | --- | --- | --- |
| `interface_id`<br>`candid-core:interface:v1` | Semantic Contract | The canonical type graph reachable from the actor, plus both profile markers | The same actor wire interface under the same profiles | Declaration names, actor-unreachable declarations, `producer`, extensions, `SourceInfo`, source text, formatting. Absent entirely for a declaration-only Contract |
| `contract_id`<br>`candid-core:contract:v1` | Semantic Contract | The complete canonical Contract payload: format markers, both profiles, every retained type node, declarations and their names, and the actor when present | The same complete semantic Contract | `producer`, `identities` itself, envelope extensions, `SourceInfo`, source text, comments, formatting, packaging. Two files with different producers, extensions, or sidecars share it |
| `source_bundle_id`<br>`candid-core:source-bundle:v1` | Raw-source bundle content | The canonical list of raw logical sources (`{name, source}`) and their import edges — comment and documentation text inside a source is source bytes, so it is covered | The same raw source bytes and import edges | Everything *derived* from those sources: derived declaration/method/label provenance and derived documentation fields, so it identifies the bundle rather than the complete `SourceInfo`. A presented sidecar is validated by rederivation at construction time, which is a separate operation |
| `artifact_id`<br>`candid-core:artifact:*` | Detached exact-octet, per declared `ArtifactKind` | The exact octet sequence handed to `artifact_id_with_limits` | The same kind and the same bytes | Nothing that is in the bytes; everything that is not. It makes no validity, authenticity, or provenance claim |

`artifact_id` is detached: it is returned to the caller, never stored inside the artifact it hashes, and never computed implicitly by a decode, so no serialized shape and no error precedence changes because it exists. It is base surface, hashes `<domain> || 0x00 || <bytes>` with no canonicalization step, and is metered by its own `max_artifact_identity_work` counter after `max_input_bytes` has gated the slice. Coverage is exactly the bytes passed to the call — persisting them first is a use case, never a prerequisite — so what travels with them depends on which kind is named. `ArtifactKind` selects a domain and neither parses nor validates, so the following describes a *valid serialized document of the declared kind*: a `Contract` document carries the Contract alone — `producer` included, which no semantic identity covers — an envelope document carries extensions and no `SourceInfo`, a compilation document carries a `SourceInfo` sidecar and no extensions, and package or application version is covered only when it is literally present in the supplied artifact bytes. Arbitrary bytes hash just as well under any kind, and the resulting ID claims nothing about their validity. See [artifact identity v1](artifact-identity-v1.md) for the normative construction and [ADR 0007](adrs/0007-artifact-identity.md) for the decision.

## Provenance is a sidecar

Optional `SourceInfo` is separate from Contract v1. It carries a bundle of raw DID sources (including imports and comments), parsed declaration/actor/field/ method documentation, function argument names, and named, numeric, or positional label spellings. It is useful for editors and diagnostics but is not sent to encoders/transports and is bound to `contract_id` rather than embedded in core identity.

`SourceInfo` is itself versioned and contains `contract_id` and `source_bundle_id`. `sources` contains `{ name, source }` for the entry DID and every resolved import. Its declaration entries carry `{ source, name, type, docs }`; field-label, method, and function-argument entries carry a source origin plus an AST-shaped `path`, so distinct source occurrences remain distinguishable even when they lower to one semantic node. This lets a future view distinguish tuple syntax from an explicit numeric record label without adding either concept to Contract.

`source_bundle_id` identifies only the canonical list of raw sources and import edges. It deliberately does not hash derived provenance. External `RawSourceInfo` construction instead treats that bundle as authoritative, recompiles it through the same parser/type-checker/lowering pipeline under the caller's operation budget, and accepts the sidecar only when the rederived Contract identity and every provenance collection match exactly. Consequently, a validated `SourceInfo` has its derived fields validated by rederivation for that construction operation; the sidecar has no independent persisted identity for signing or cache lookup.

The public upstream Candid AST does not expose stable spans for every semantic node. v1 therefore preserves raw source plus AST-shaped occurrence paths in the sidecar and preserves byte spans on parser diagnostics. It intentionally does not introduce a second handwritten Candid parser just to manufacture node spans.

This separation is deliberate:

- Contract owns semantic identity and all information necessary to describe Candid wire types.
- SourceInfo owns explainability, source presentation, and label spelling.
- Future views own conveniences such as blob detection (`vec nat8`), tuple detection (positional records), and conventional `Result` recognition.
- Future UI, form, validation-policy, widget, workflow, and transport layers depend on Contract; Contract never depends on them.

## Diagnostics

Loading DID produces either a valid Contract (and optional SourceInfo) or structured diagnostics.  Parser and semantic errors remain distinguishable so a host can render an actionable editor error without guessing Candid rules itself.

One serializable item type, `Diagnostic`, backs every failure domain in the crate. The outer error types stay domain-specific for Rust ergonomics — `CompileError` for compilation (`compiler`), `ContractValidationError` for Contract/provenance validation, `HostValueValidationError` for HostValue validation (`host-value`) — but their items are all the same algebra; `ContractViolation` and `HostValueViolation` are compatibility aliases for `Diagnostic`. `Diagnostic` itself, and every field it can carry — `phase`, `severity`, `span`, `related`, `resource_limit` — is base surface, so a `default-features = false` consumer sees the identical serialized item shape. An item carries:

- `code` and `message` — always present. Codes and structured `path` values are the stable, machine-matchable surface; human-readable `message` text is not a stable interface and may be reworded.
- `phase` and `severity` — present on every compile-domain diagnostic, never on validation violations. Serialized spellings are unchanged (`parse`, `type_check`, `load`, `lower`; `error`).
- `path` — the semantic/value path (`$.…`), present on every validation violation and on compile diagnostics converted from structured violations.
- `span` — an optional source location (see below).
- `related` — ordered secondary locations. Every upstream report label is retained, under a policy set by which text the report indexes. For reports against original text, the first label's location becomes the exact `span` and every later label becomes an ordered `related` entry carrying its exact span. For reports produced against rewritten (pretty-printed, materialized) text, no rewritten offsets are published: when the diagnostic's message is derived from the report itself (parse errors), the primary label's message is embedded in that message and later labels become ordered `related` entries without spans; when the message comes from the error's own rendering, every label message — the primary included — is retained as an ordered `related` entry without a span.
- `notes` — ordered free-text notes (for example expected-token lists).
- `resource_limit` — the exact `{resource, limit, observed}` triple for resource failures. `limit` and `observed` are **fixed-width `u64`** on the wire: internal `usize` counters widen exactly (the crate refuses to compile on targets whose `usize` exceeds 64 bits), so the same failure serializes to the same numeric text on every platform. Every conversion between domains preserves this triple verbatim; no path may reduce a resource failure to message text. Structured failures convert item-by-item (`ContractValidationError` → compile diagnostics during lowering, compile diagnostics → violations during provenance rederivation) keeping code, path, span, related locations, notes, and resource metadata; a compile diagnostic that carries no path converts to the violation-domain root `$`, and the generic `contract_lowering_error` code is reserved for genuinely unstructured invariant failures.

Every optional field is omitted from JSON when absent, which keeps each domain's pre-existing serialized shape byte-compatible: compile diagnostics still serialize as `{code, phase, severity, message, span?, notes?, resource_limit?}` and violations as `{code, path, message, resource_limit?}`; the new fields appear only where the data genuinely exists. All diagnostic item types derive `Deserialize` with unknown keys rejected; fields that were previously mandatory in one domain (`phase`, `severity`, `path`) are optional in the shared item, so deserialization is strictly more permissive than before, never less.

A source location (`SourceSpan`) comes in two forms. An **exact** span carries `start_byte`/`end_byte` offsets — fixed-width `u64` on the wire, widened exactly from the parser's byte offsets — valid for the named source's original text — parse errors report these, byte-precise, against the logical source ID (`memory:/…`, `workspace:/…`). A **source-scoped** location names a logical source with no offsets.

Both compilation backends produce only these two forms, by different routes. The in-memory backend (`compile_with_resolver`) never sees a rewritten offset at all: it merges the sources the resolver supplied, and the merge and actor-resolution reports upstream produces carry no labels, so there is nothing positional to publish. Where it *can* name the failing source — an `import service` whose target declares no main service — it attaches that logical source as a source-scoped location, because it knows which unit failed without reading the message. The native backend (`compile_did_file`, `filesystem-compiler`) type-checks by materializing pretty-printed sources into a private temp directory under numeric names, and errors crossing that boundary would otherwise leak rewritten offsets and `N.did` file names; it therefore maps every materialized identity back to its logical source ID — anchored to complete upstream message templates, never to a bare quoted `N.did`, so a user's own text field label is never read as a source identity — and withholds byte offsets it cannot prove correct for the original text. No diagnostic from either backend ever exposes a temp directory, a numeric materialized name, or a rewritten offset presented as an original one.

The CLI envelopes are unchanged: compile and operational failures appear under `"diagnostics"`, Contract and HostValue validation failures under their existing `"violations"` envelopes, with the same top-level keys as before. `tests/diagnostics_contract.rs` pins the exact serialized shapes, the logical-source mapping, related-location ordering, and resource-metadata preservation across every conversion chain.

Malformed Contract JSON is rejected by Contract JSON decoding and graph validation rather than being silently repaired. Validated `Contract`, `Compilation`, and `ContractEnvelope` values are reachable only through policy-taking constructors and bounded parse entry points such as `Contract::from_json_with_context`, `Compilation::from_slice_with_context`, and `ContractEnvelope::from_slice_with_limits`. None of these types implements `Deserialize`: a trait impl has no argument position for a resource policy, so it could only ever decode under limits the library chose. A host therefore does not get an unchecked Contract by taking a normal JSON deserialization path.

Bounded parsing enforces `max_input_bytes` before the document is decoded, then shares one budget between decode and validation, so a nested parse charges the counters the decode gate already observed. The byte gate bounds peak allocation against a caller-chosen ceiling; it does not reject element-by-element during decode. Decode-time element charging is a named follow-up.

The no-argument conveniences (`Contract::from_json`, `try_from_raw`, `validate`, `canonicalize`, `to_json_pretty`, `ContractDraft::build`) remain, and run the same bounded path under `Limits::default` — the versioned `LimitsProfile::InteractiveV1`. That is the ADR 0005 position: conveniences use the default policy, and the context-aware entry points expose it. What changed is that a policy is now always expressible — every one of them has a `_with_limits` or `_with_context` sibling, which a trait impl could never offer.

Trusted serde integration is the separate, unbounded path. Decoding a raw DTO (`RawContract`, `RawSourceInfo`, `ContractDraft`) is not a trust boundary and carries no allocation bound: a caller must gate the byte length itself or use a bounded parse API; a decoded draft only becomes a Contract through its limit-taking `build` entry points. `Serialize` likewise consults no limits and performs no revalidation; it is for already-validated values. The limits-aware render is `to_json_pretty_with_context`, which charges its rendered length against `max_canonicalization_work` in addition to the structural limits construction consumed, so raising only the limit that gated construction is not always sufficient.

## Portable limits configuration

The serialized form of `Limits` is a versioned portable configuration, not a bare field map:

```json
{"version": 1, "profile": "interactive_v1", "overrides": {}}
```

That document is exactly how `Limits::default()` serializes. `version` pins the schema (currently `1`), `profile` names a frozen set of default numbers (`LimitsProfile::InteractiveV1` is the only released profile; its values never change — new tunings become new profile names), and `overrides` carries only explicitly overridden fields as fixed-width `u64` values, so one document configures identical policy on every supported host (the crate builds on 32- and 64-bit targets; the `InteractiveV1` default values exceed a 16-bit `usize`, and `usize` may not exceed 64 bits so the `u64` widening stays exact). `deadline_unix_ms` keeps its pre-existing `u64` Unix-milliseconds representation and appears as an override when set. This wire contract is normative; the pinned shapes live in `tests/api_portability.rs`.

Rejection is fail-closed and structured: unknown top-level fields, unknown override fields, explicit `null` overrides, unsupported versions, and unknown profiles are all decode errors, and an override the host platform cannot represent as `usize` is rejected — never truncated or wrapped — with a stable `{code, path, message}` error (`unsupported_limits_version` at `$.version`, `unsupported_limits_profile` at `$.profile`, `limit_override_unrepresentable` at `$.overrides.<field>`; programmatically inspectable via `TryFrom<LimitsConfig> for Limits`). Zero is a defined value for every limit, not an invalid configuration: a zero byte/count/work limit rejects any input that consumes the resource, and `max_diagnostics = 0` retains the single out-of-band sentinel violation.

The host-only runtime bookkeeping stays outside this portable contract: `RuntimeContext` serializes as `{"limits": <portable config>}` and never serializes its `CancellationToken`; the monotonic deadline snapshot, budget counters, and the `usize`-carrying `HostValueJsonError` variants are Rust API surface, not wire values. Internally, accounting and allocation indices remain `usize`; portable `u64` values narrow through checked conversions exactly once, at the configuration boundary.

## Invariants and ownership rules

- A Contract is self-contained: every `TypeRef` is in bounds and every actor, field, argument, result, method, and class edge has the required target kind.
- Interface identity is graph-based and excludes declaration names, comments, and source spans. Contract identity includes declaration names; source identity includes logical source URIs, bytes, and import edges.
- Record and variant fields retain authoritative Candid `u32` field IDs only. The semantic engine, not host code, determines named-label hashes; SourceInfo retains label spelling.
- Field IDs are unique. Service method names are unique and each method ID equals Candid's hash of its name; distinct method names may legitimately share a 32-bit hash, so their text remains authoritative. Method targets are `func` nodes and class result targets are `service` nodes. A `class` node is valid only as the top-level class actor root; it is not a first-class Candid type edge. Canonicalization minimizes semantic equivalents, orders fields and methods deterministically, and re-indexes the graph.
- A function has exactly one valid Candid mode: `update`, `query`, `composite_query`, or `oneway`; an `oneway` function has no results. No arbitrary strings or combinations are accepted.
- The graph may contain cycles.  Validation tracks visited node identities and never requires a recursive type to be expanded into a tree.
- Format, semantics, and canonicalization profiles are independently declared. Unknown versions or profiles fail closed.
- Every arena node is reachable from an actor or declaration root (unless the arena itself is empty).
- The producer owns construction and identity calculation: `ContractDraft` carries no identity fields, and building it calculates the identities its Contract ships with. Consumers may validate and traverse immutable Contract JSON, but must not infer missing semantics.

## Explicit non-goals for this slice

This slice implements the lossless tagged HostValue ABI and graph-directed validation behind the `host-value` feature, but not defaults, coercions, forms, widgets, UI metadata, workflow projections, transport adapters, agent calls, code generation, or Candid binary encoding/decoding. It also does not introduce `blob`, `tuple`, or `Result` nodes: those remain derived semantic views over the canonical graph.

## Next slice

Implement the HostValue \<-> Candid binary bridge. It will accept a validated Contract plus a contract-bound type or method selector, reuse the implemented HostValue validator, delegate binary encode/decode to the authoritative Candid runtime, and return structured diagnostics. It must consume Contract only; it must not parse DID source or add UI policy.
