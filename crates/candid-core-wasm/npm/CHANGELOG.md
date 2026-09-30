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
