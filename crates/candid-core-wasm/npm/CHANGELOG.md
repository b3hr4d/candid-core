# Changelog

What changed between released versions of `@candid-core/cli`. This file
ships inside the published tarball, so the record travels with the artifact.

**Every entry names the exact revisions it embeds** — the `candid-core`
compiler crate and the `candid-core-ts` generator are compiled into the wasm
artifact from one repository commit, and the pairing is the artifact's
provenance: the parity gates prove the emitted module and contract are
byte-identical to that commit's Rust-native outputs over the golden
fixtures.

`@candid-core/cli` is pre-1.0. Until 1.0 any release may change the CLI
grammar, the library API, and the request/response shapes. Pin an exact
version.

## Unreleased

- **BREAKING**: the embedded `candid-core-ts` generator no longer refuses an
  `opt` whose inner type is another `opt`, `null`, or `reserved` (its
  `TsGenError::UnrepresentableOption` is removed). Such declarations now
  generate, with the present value boxed: `opt opt nat` emits
  `{ some: bigint | null } | null` with builder `c.opt(c.opt(c.nat))`.
  Interfaces that generated before produce byte-identical modules.
- **Release ordering**: the boxed aliases need the `@candid-core/schema`
  release that introduces boxed options; the published 0.2.0 cannot type
  them. The release that ships this generator must raise the
  `@candid-core/schema` peer from `^0.2.0` to that release.

### Generated modules bind `$`-prefixed locals

The embedded generator changes the layout of every module it emits; the
exported names, every type, every field and field order, and every
builder are unchanged. A module imports the schema runtime as a namespace and
binds each declaration as a local whose name starts with `$`, exported under
its Candid name:

```ts
import * as $ from "@candid-core/schema";

type $Tokens = { e8s: bigint };
const $Tokens: $.Schema<$Tokens> = $.c.rec(() => $.c.record({ e8s: $.c.nat64 }));
export { $Tokens as Tokens };
```

- **BREAKING**: the text of every generated module changes. `import
  { Tokens, actor, type Actor } from "./ledger"` resolves exactly as before,
  so code that imports the generated names is unaffected; code that parses or
  patches the generated source, or relied on its module-scope names
  (`export type X`, `export const X`, the `c` and `Schema` bindings, the
  separate `import type { PrincipalValue }` line), must follow the new layout.
- **BREAKING**: declarations the generator used to refuse with
  `ts_generation_refused` now generate: names that collided with the module's
  own bindings (`c`, `Schema`, `PrincipalValue`), with the global types its
  lowerings use (`Array`, `Record`, `Uint8Array`, `Promise`), or that are
  TypeScript reserved words (`delete`, `string`). Import them by name,
  renaming where needed (`import { delete as del } from "./service"`). A
  declaration named `default` becomes the module's default export.
- Declarations named `actor` or `Actor` are still refused with
  `ts_generation_refused`: those are the module's own export names.
- **Release ordering**: none. The layout needs only the `c`, `Schema` and
  `PrincipalValue` exports `@candid-core/schema` 0.2.0 already has; the
  goldens without collapsing options type-check against 0.2.0 unchanged.

### Generated modules type principals as `$.Principal`

- **BREAKING**: the embedded generator emits `$.Principal` wherever it
  emitted `$.PrincipalValue`: principal fields, func reference aliases
  (`{ principal: $.Principal; method: string }`), service aliases, and the
  `actor` schema (`$.Schema<$.Principal>`). Nothing else in any generated
  module changes. The paired `@candid-core/schema` release replaces
  `PrincipalValue` (`{ toText(): string }`) with `Principal`, canonical
  principal text as a branded string: a decoded principal is that text, and
  encoding accepts only it, so an SDK `Principal` converts once with
  `principal(sdkPrincipal)`.
- A declaration named `Principal` still generates, bound as `$Principal` and
  exported as `Principal`.
- **Release ordering**: the emitted modules need the `Principal` export,
  which `@candid-core/schema` 0.2.0 does not have. The release that ships
  this generator must raise the `@candid-core/schema` peer to the release
  that introduces `Principal`.

### A declared primitive names only itself

- **BREAKING**: the embedded generator no longer renders a use of a primitive
  by a declaration's name. The Contract arena shares one node per structure, so
  a name recorded for `nat64` was rendered at *every* `nat64` in the interface,
  including ones that never wrote it. A use of a primitive — a field, an array
  element, a variant arm, a method argument — now renders structurally
  (`bigint`, `$.c.nat64`, and `$.Principal` for `principal`); a declaration of
  the primitive is still emitted and exported as itself. `type Memo = nat64;
  type R = record { a : nat64; b : Memo }` now generates `R = { a: bigint; b:
  bigint }` where it generated `{ a: Memo; b: Memo }`, and the ICRC-1
  ledger's `TransferArg.amount`, written `Tokens` beside `type BlockIndex =
  nat`, is `bigint` and no longer `BlockIndex`.
- **BREAKING (value type)**: every `vec nat8` is `Uint8Array` / `$.c.blob()`,
  whatever its element type is called. Before, a single `type Byte = nat8`
  anywhere in the interface turned every `blob` and `vec Byte` into
  `Array<Byte>`, a `number[]`. For an interface with a declared `nat8` alias
  the generated blob types change from `number[]` to `Uint8Array`; the encoded
  bytes do not change. Interfaces with no declared primitive alias generate
  byte-identical modules, apart from the JSDoc below.
- **Trade-off**: a declared alias no longer survives as the *spelling* of a
  field's type (`amount : Tokens` reads `amount: bigint`); the type is the
  same. Two structurally equal *record* declarations still collapse to the
  first name, as before.
- **Release ordering**: the paired `@candid-core/schema` change makes
  `schemaFromContract` load every `vec nat8` as a blob too, so a generated
  module and a loaded schema agree; ship them together.

### `.did` doc comments become JSDoc

- The embedded generator writes each `.did` doc comment as JSDoc: above the
  generated type and const of a declaration, on a record property, on a
  variant arm's `tag`, on a method of the `Actor` type, and on the `actor`
  export. Docs are the `///` run (or plain `//` lines) directly above the
  item; block comments are not docs. The doc text is escaped — `*/` becomes
  `*\/`, an `@` that could start a tag becomes `\@`, a code fence has each
  backtick escaped — so a comment cannot end early, forge a tag or swallow the
  generated ones.
- A method's `@param` tags name the `.did`'s argument names, and the `Actor`
  method's parameters take the same names; an unnamed argument, a reserved
  word, a name that is not identifier-shaped or a collision stays `arg{n}`.
- A record or union with a documented member spans several lines; without
  docs its one-line form is unchanged. Tuple elements carry no docs.
- Which occurrence documents a node the arena has de-duplicated is one rule:
  the declaration (or actor) whose structure is being emitted; occurrences
  inside one declaration that disagree are dropped. No Contract, envelope,
  identity or wire byte changes — docs are provenance — and the
  `org.candid-core.field-names/v1` extension is untouched.
- **Additive**: modules for `.did` files with no comments beside their
  declarations are unchanged by this entry.

## 0.1.0 — 2026-08-27

Embeds `candid-core` 0.1.0-beta.3 and the `candid-core-ts` generator from
the same repository commit the release is dispatched from; the release
record names the exact SHA.

The first version: the JavaScript-only on-ramp.

- **`gen <service.did> [-o <dir>]`** emits the generated
  `@candid-core/schema` module, the one-document `ContractEnvelope` with the
  `org.candid-core.field-names/v1` field-name table, and prints the content-addressed identities. Imports resolve
  from the entry's directory as an in-memory bundle; every generation
  double-runs and the tool refuses to write on any byte mismatch.
- **`didToContract` / `didToModule`** — the same two operations as a
  library, for Node and browsers alike: one wasm-bindgen artifact over
  `candid-core`'s `compiler` feature (`compile_with_resolver` +
  `MemoryResolver`; no filesystem, no eval, no dynamic import) plus the
  `candid-core-ts` generator. Failures are `{ ok: false, diagnostics }` with
  the compiler's diagnostics verbatim; generator refusals surface as
  `ts_generation_refused`, malformed requests as `invalid_request`.
- **Parity is a gate, not a claim**: CI compares the wasm-built outputs
  byte-for-byte against the repository's reviewed goldens and against the
  native `candid-core compile --envelope` fixture, builds the artifact twice
  from clean and requires identical bytes, and runs `didToContract` inside
  headless Chrome feeding `schemaFromContract`, envelope-carried names
  included.
