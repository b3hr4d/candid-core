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

Documentation only, in both senses: the editor hover and the npm page. No
runtime behaviour, no inferred type, no wire encoding, and no issue code
changed — the only executable difference anywhere in `dist/` is one unused
import dropped from `codec.js`.

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
