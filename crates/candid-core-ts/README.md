# candid-core-ts

TypeScript code generation over the [`candid-core`](../../README.md) Contract
graph. This is the separate-crate half of the
[issue #38](https://github.com/b3hr4d/candid-core/issues/38) decision: code
generation consumes the published Contract model and never influences it —
`candid-core`'s public API, packaged archive, canonical bytes, and identities
are unaffected by anything in this directory.

**Unpublishable by construction, for now.** `publish = false` stands until the
registry name is decided deliberately; crate names on crates.io are as
permanent as versions, and `candid-core-ts` is a working name, not that
decision.

**Base surface only.** The dependency on `candid-core` declares
`default-features = false`: a generator needs no Candid parser, no filesystem
capability, and no host-value ABI. If a change here ever needs a feature, that
is a design boundary being crossed and belongs in review, not in a lockfile
diff.

**Covered by its own CI job.** The root is a non-virtual workspace, so
`cargo test`/`cargo clippy` at the repository root select the root package
only; the `Generator crate` job in `Verify` checks the featureless library,
lints and tests both feature configurations, and type-checks the goldens.

**The first generator slice is in.** `generate_module` covers all eighteen
primitives, `opt`, `vec`, `record` (tuple-shaped records become TypeScript
tuples), `variant`, and named declaration references, recursion included.
`func`, `service`, and `class` are deferred: top-level declarations of those
kinds are skipped with a header note, and a deferred construct nested inside a
supported type fails closed rather than emitting `unknown`. Field names are
caller-supplied through `TsNames` — the semantic Contract stores only label
IDs — with a `compiler`-feature bridge from a compilation's provenance sidecar.

**The output is a clean domain model, by owner decision on issue #38.**
`opt T` renders `T | null`; variants render as discriminated
`{ tag, value }` unions with `value` omitted for `null` payloads; anonymous
`vec nat8` renders `Uint8Array`; `Principal` imports from
`@icp-sdk/core/principal` by default. This deliberately diverges from the
shapes the agent-js runtime produces (`[] | [T]` opts, single-key variant
objects — what `@icp-sdk/bindgen` emits, verified against its 0.4.0 output):
compatibility is a non-goal for now, and consuming these types against a live
agent needs a boundary conversion, recorded on the issue as future work. One
consequence is enforced rather than papered over: an `opt` whose inner type
can itself be `null` in TypeScript — `opt opt`, `opt null`, `opt reserved` —
fails closed, because `T | null` cannot distinguish `None` from `Some(None)`.

**The generated artifact is a runtime schema, not just types.** Each
declaration emits an invariantly-annotated builder alongside its alias —
`export const X: Schema<X> = c.rec(() => …)` — targeting the schema core in
`ts/schema.ts` (imported as `@candid-core/schema`, a placeholder until the npm
package and its permanent name exist). Because `Schema<in out T>` is invariant,
the annotation makes `tsc` itself prove on every golden that the builder infers
exactly the reviewed alias. The builders carry the structure the Zod-style
runtime recorded on issue #38 will walk: validation, form metadata, and the
TS-native Candid codec are later slices over these same objects.

**The schemas do something at runtime (issue #102).** `ts/validate.ts`
validates JavaScript values against any schema — every combinator, bounded
depth and traversal budgets, fail closed, no exceptions for control flow —
with path-addressed issues in candid-core's own diagnostic shape
(`{code, path, message, resource_limit?}`, stable snake_case codes,
`$`-rooted paths). `ts/contract.ts` builds the same schemas dynamically from
a canonical Contract JSON document, applying every generator mapping decision
(anonymous `vec nat8` → blob, tuple-shaped records, the collapsing-`opt`
rejection, deferred `func`/`service`/`class`) with label text from the same
caller-supplied name-table shape `TsNames` takes. The golden cross-check test
proves the two paths agree: for every fixture, the dynamically built schema
must return the identical `validate` result the generated builder returns,
sample by sample. The suites run on Node's built-in test runner with native
type stripping — no test framework, no `@types/node`, no npm dependency
beyond the pinned TypeScript.

**Golden tests carry the mapping decisions.** Each fixture under
`tests/fixtures/` must generate byte-identical output to its checked-in golden
in `tests/goldens/`; regenerate deliberately with `UPDATE_GOLDENS=1` and review
the diff. The goldens are additionally compiled by the exact TypeScript pinned
in `ts/package-lock.json` under `strict` (`npm ci && npx tsc --noEmit` in
`ts/`), with the `Principal` import resolved to a local type stub so the check
is hermetic.

**The codec speaks the wire format (issue #103).** `ts/codec.ts` encodes
domain values to Candid binary and decodes Candid binary to domain values,
schema-directed, with the spec's coercion relation on decode (an expected
`opt` absorbs content mismatches to `null`; unknown variant tags and missing
non-optional record fields are hard errors; extra wire fields and trailing
arguments are skipped under the same budgets as decoded values). Bounded and
fail-closed throughout: input size, type-table entries, depth, elements, and
single-numeric byte length are all capped, non-minimal LEB128 is rejected —
the spec defines deserialisation as the inverse of serialisation, whose image
contains only minimal encodings; we adopt that strict-inverse reading as an
interpretation — and nothing throws on any input. Wire field ids derive from
schema keys (`_N_` parse-back, else the Candid label hash), which is why
`schemaFromContract` hash-enforces its name table. Principal text↔bytes is
self-contained (base32 + CRC-32); `float32` encoding is exact-or-refuse.

**Exactly what the conformance suite demonstrates — and no more.** The
checked-in vectors under `tests/goldens/wire/` are generated by the upstream
`candid` crate (exact-pinned, dev-only) from textual values over the golden
fixtures; the TypeScript decoder must interpret every vector to the pinned
domain value, and the TypeScript encoder's own bytes are checked in and
decoded back through the `candid` crate by `tests/wire_vectors.rs` — the
reference implementation checks our encoding, not only the reverse. A
seeded-PRNG property harness round-trips generated values over every fixture
schema, and adversarial suites pin truncation, overlong LEB128, hostile type
tables, zero-width element bombs, and depth exhaustion. Not demonstrated:
opaque reference values (form tag `0`) and external reference sequences are
refused, `func`/`service` values have no schema counterpart and only skip,
and no claim is made about agent envelope formats or any bytes beyond the
`DIDL` message itself. No performance claims (issue #39's rule).

Scope and non-goals are recorded on issue #38 and restated in `src/lib.rs`:
`@icp-sdk/bindgen` is a differential oracle rather than the specification, and
no performance claims are made before the generator exists and issue #39's
baseline machinery measures it.
