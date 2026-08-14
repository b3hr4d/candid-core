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

Additive: the editor hover, the npm page, a small introspection surface on the
root export, result unwrapping on `./validate`, and one-document contract
loading on `./contract`. Nothing that existed before behaves differently — no
inferred type, no wire encoding, and no existing issue code changed
(`./contract` adds one code, `invalid_extension_name`), and every executable
difference in the modules that shipped in 0.1.1 is new API plus one unused
import dropped from `codec.js`.

### One-document contract loading

- **`schemaFromContract` accepts a `ContractEnvelope` document** — the
  `{ contract, extensions }` shape `candid-core compile --envelope` emits,
  recognised by its `contract` key, which no canonical Contract document
  carries — and consumes the field-name table its
  `org.candid-core.field-names/v1` extension holds (the key is exported as
  `FIELD_NAMES_EXTENSION`). One self-describing document now replaces the
  contract-plus-table pair; the two routes build verdict-for-verdict
  identical schemas, proven against the golden cross-check samples.
- **Envelope-carried names are validated exactly like caller-supplied
  ones** — same entry shape, same `_N_` reservation, same hash enforcement,
  same entry cap — with issues path-addressed at
  `$.extensions["org.candid-core.field-names/v1"][…]`. An explicit `names`
  option wins over the envelope's table, which is then not consulted at all.
- **The envelope shell fails closed the way the Rust loader fails it**:
  unknown envelope keys, a non-object `extensions`, a non-array field-names
  value, and an extension name outside the reverse-domain-`/vN` grammar are
  all refused — the last with the new issue code `invalid_extension_name`,
  mirroring candid-core's own code. Contract-side issues inside an envelope
  are re-rooted at `$.contract…`, where the data actually sits. One
  tightening rides along for bare contracts: a `names` option that is not an
  array at runtime now fails closed as `invalid_name_table` instead of being
  silently treated as empty.
- **Extensions stay outside the canonical identities**: `contract_id` and
  `interface_id` are computed over the Contract alone, so an envelope carries
  names without moving any identity — the README's "From a `.did` file"
  section now documents the one-document flow first.

### Unwrapping ok/err results

- **`isResultSchema` and `unwrapResult` are exported from `./validate`.**
  `variant { ok : T; err : E }` is the universal canister result convention,
  and unwrapping one generically has meant probing a decoded value for
  `ok`/`err` keys — which misfires on any record legitimately carrying those
  field names and cannot type the error payload at all. These read the schema
  instead: `isResultSchema` answers for a schema that resolves — through the
  `rec` indirections generated declarations and runtime-loaded edges arrive
  wrapped in, on the same bounded walk `resolveSchema` performs — to a variant
  whose arms are exactly an ok arm and an err arm; `unwrapResult` validates
  the value and returns
  `{ ok: true, value }` or `{ ok: false, error }`, both typed from those arms
  — `ResultOk<S>` and `ResultErr<S>` name the two payload types on their own.
- **Both spellings, as pairs.** `ok`/`err` (Motoko's `Result.Result`) and
  `Ok`/`Err` (Rust's candid derive), in either arm order, with exactly two
  arms. A mixed pair, a third arm, and every other alias are refused: no
  generator emits them, so admitting one would be unwrapping on coincidence.
- **A bare-tag arm unwraps to `null`**, the single value of the Candid `null`
  its arm declares, so a payload is always exactly the arm's own type and
  never widens to `undefined` — which is not a Candid value anywhere in this
  runtime.
- **An err arm is a value, not an exception**, and neither is a malformed one:
  a value that is not of the schema comes back as `{ ok: false, issues }`,
  carrying exactly what `validate` reports for it, and a value that throws
  while being read is an issue too. A schema that is not a result variant is a
  programmer error and throws `TypeError`, as `resolveSchema` and
  `serviceMethods` already do for theirs.
- **On `./validate` rather than the root**, so that reading a result costs no
  new dependency for anyone else: the root entry imports nothing at runtime,
  and the actor factory, the form-model builder, and the Contract loader all
  import *it* — so the validator would have arrived with `formModel` and
  `schemaFromContract` for consumers who never asked for one.

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
