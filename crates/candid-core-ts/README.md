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

**Golden tests carry the mapping decisions.** Each fixture under
`tests/fixtures/` must generate byte-identical output to its checked-in golden
in `tests/goldens/`; regenerate deliberately with `UPDATE_GOLDENS=1` and review
the diff. The goldens are additionally compiled by the exact TypeScript pinned
in `ts/package-lock.json` under `strict` (`npm ci && npx tsc --noEmit` in
`ts/`), with the `Principal` import resolved to a local type stub so the check
is hermetic.

Scope and non-goals are recorded on issue #38 and restated in `src/lib.rs`:
`@icp-sdk/bindgen` is a differential oracle rather than the specification, and
no performance claims are made before the generator exists and issue #39's
baseline machinery measures it.
