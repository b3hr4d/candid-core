# Changelog

What changed between released versions of `@candid-core/schema`. This file
ships inside the published tarball, so the record travels with the artifact and
survives registry mirrors and offline installs.

The package versions independently of the [candid-core] crate whose Contract
model and generator produce its bindings, so **every entry names the
`candid-core` version it pairs with**. The release procedure that produces an
entry here is [docs/releasing.md] in that repository.

`@candid-core/schema` is pre-1.0. Until 1.0 any release may change the builder
API, the inferred domain types, the codec's wire behaviour, and the codes and
`$`-rooted paths validation reports. Pin an exact version.

## 0.1.2 — prepared, not yet published

Pairs with `candid-core` 0.1.0-beta.2.

Additive, and mostly documentation: the editor hover, the npm page, and a
small introspection surface on the root export. Nothing that existed before
behaves differently — no inferred type, no wire encoding, and no issue code
changed, and the only executable difference in the modules that shipped in
0.1.1 is one unused import dropped from `codec.js`.

### Reading a schema back

- **`resolveSchema` and `serviceMethods` are exported from the root entry.**
  Both were private walks before: the actor factory carried one copy of the
  rec-chain resolution and the form-model builder another, so anything that
  introspects a service — a wire debugger, a devtools panel, a hook generator
  — had to re-derive an undocumented discipline against the node interfaces.
  `resolveSchema` follows `rec` indirections to the node underneath, bounded
  at 256 hops and throwing `TypeError` on a chain that never terminates, on an
  object that is not a schema, and on a `kind` this package does not define —
  `Schema` requires only that `kind` be *a* string, so the node handed back is
  checked against the kinds the return type covers rather than asserted to be
  one of them. `serviceMethods` returns the per-method
  table — `name`, `mode`, `args`, `results` — as a `ReadonlyMap` keyed in
  declaration order, resolving each method, since schemas built from a
  Contract document at runtime wrap every method in a lazy `rec` thunk.
  Both live on the root export rather than a new subpath, so reading a method
  table costs a consumer no dependency on the codec.
- **The actor factory and the form-model builder now call them**, which is
  what makes the table a consumer reads and the table an actor dispatches on
  the same table by construction. Every message either one throws is
  unchanged.
- **`SchemaNode`, `ResolvedNode`, and `ServiceMethod` are exported** as the
  types those two need: the discriminated union of every node kind, that
  union without the `rec` case that `resolveSchema` has already removed, and
  one method's signature. Every member of the union has its domain type
  erased, composites included, because `Schema<in out T>` is invariant and a
  record of *specific* fields is otherwise not assignable to a record of the
  general field map.
- **`formModel`'s laziness is documented.** A `rec` schema becomes a `lazy`
  node, and generated declarations are all `rec` — so the root of a model is
  `lazy`, and so is every reference to another named declaration inside it.
  The `while (node.control === "lazy") node = node.expand()` idiom is now in
  the hover, with the reason the nodes are not expanded for you: a form
  cannot eagerly expand a recursive type.

### Editor hover

- **Every builder is documented.** 2 of the 28 members of the `c` object
  carried JSDoc in 0.1.1; all 28 do now, and the comments flow into
  `dist/*.d.ts` at build time, which is where a consumer's editor reads them.
  `schema.d.ts` grows from 149 lines to 528 as a result.
- **Shipped doc comments stand on their own.** 0.1.1's declarations cited
  three internal issue numbers across four comments — links a consumer's
  editor cannot follow. Those are gone, and the packaged-artifact gate now
  refuses a tarball whose declarations contain any of them.
- **The agent-adapter sketch is out of `dist/actor.js`.** It was a `//`
  comment, so declaration emit stripped it: it never reached hover, and it
  dropped `effectiveCanisterId`, which would have mis-routed the one call
  shape that needs it. The README now carries a complete adapter instead.

### The npm page

- **The README is an on-ramp rather than a summary.** It opens with the install
  command; states the type-only peer dependency's two failure modes by name —
  the `TS2307` that points into `node_modules` without naming the fix, and the
  `skipLibCheck: true` build that goes green while `Principal` degrades to
  `any`; carries the complete `@icp-sdk/core` v6 `Transport` adapter; and
  documents the `.did` → schemas route that exists today
  (`cargo install candid-core`, `candid-core compile`, then the contract and
  its field-label table into `schemaFromContract`).
- **A support matrix**, measured rather than inferred: TypeScript ≥ 5.0 (a
  *parse* error below it — `c.tuple`'s `const` type parameter);
  `node16`/`nodenext`/`bundler` resolution only, `node10` cannot resolve the
  package at all; ESM-only, on Node ≥ 16 (the build targets ES2020 and does not
  down-level, so `?.` and `??` reach `dist/`); and, from CommonJS, Node ≥
  20.19/22.12 for `require()` and TypeScript ≥ 5.8 with `"module": "nodenext"`
  for the types. No `engines` field: both floors are narrow, and enforcing
  either in install metadata would warn for the consumers it does not apply to.
- **This changelog ships**, listed in the manifest's `files`. The packaged
  artifact gate refuses a tarball whose changelog does not document the version
  being packed together with its `candid-core` pairing, so the claim above is
  checkable from npm rather than promised by it.
- **`homepage` points at the package directory.** npm previously derived the
  homepage link from `repository` and landed consumers on the repository root
  README — Rust-crate material that never mentioned this package. That README
  now carries a TypeScript section too.
- The README's TypeScript is compiled by the packaged-consumer gate, against
  the packed artifact, so an example here cannot drift from the package it
  documents.

## 0.1.1 — 2026-08-03

Pairs with `candid-core` 0.1.0-beta.2.

An audit-fix release. Every change is a fail-closed correction found by review
of the 0.1.0 surface; the module list, exports map, and peer metadata are
unchanged.

- **A blob's declared length is checked against the remaining input before
  any allocation is charged for it**, and the wire `vec nat8` length is
  preflighted so the `Uint8Array` alias agrees with the general vector path in
  every corner (`dist/codec.js`).
- **A named declaration targeting the actor-root class node is refused**, with
  the exemption pinned against a second class node (`dist/contract.js`). A
  class is legal only as the actor root, so naming one was a document the
  builder should never have accepted.
- **Variant arms are classified structurally**, so the inferred
  `{ tag, value }` union matches what the validator and codec actually do at
  runtime (`dist/schema.d.ts`).
- **The `AnyFieldSchema` bound is carried through the codec, actor, and form
  entry points**, admitting the `empty` leaf through every composite bound
  without weakening the gate that keeps `Schema<in out T>` invariant.

## 0.1.0 — 2026-07-31

Pairs with `candid-core` 0.1.0-beta.2.

The first real release: the schema runtime extracted into its own package with
a manifest, a build, a packaged-consumer smoke, and publish machinery. Seven
subpath exports — the `c` builders and `Infer`, `./validate`, `./contract`,
`./codec`, `./actor`, `./forms`, `./labels` — ESM-only, published with npm
provenance from a protected environment.

`0.0.0-bootstrap` (2026-07-31) precedes it and is not a usable release: it
exists only because npm cannot attach a trusted publisher to a name that does
not yet exist on the registry. It is tagged `bootstrap`, never `latest`.

[candid-core]: https://github.com/b3hr4d/candid-core
[docs/releasing.md]: https://github.com/b3hr4d/candid-core/blob/main/docs/releasing.md
