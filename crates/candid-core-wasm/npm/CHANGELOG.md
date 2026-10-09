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

Not yet released; the version is decided at release time. Embeds the
`candid-core` crate and the `candid-core-ts` generator from the release's
commit, as every entry does.

### Added

- **`candid-core-cli project <in.did> --methods <a,b,...> -o <out.did>`**
  writes a `.did` holding only the methods named and every declaration they
  reach, as one self-contained file, and prints the interface identities of
  the input and of the projection. The output is deterministic: the same
  input and the same set of names give the same bytes, in any order. It
  keeps declaration names, doc comments and argument names, so `gen` on the
  projection emits the same declarations, docs and modes as on the full
  interface, with an `Actor` that lists only the projected methods. A
  service class's init arguments are dropped. An unknown method name fails
  with `unknown_method` (its `notes` list the service's methods), an empty
  list (`--methods ""`) with `empty_method_list`, a source with no service
  with `no_service`; each exits 1 and writes nothing. `-o` is required and
  may not name the input file. `--json` prints one document instead of the
  report.
- **`candid-core-cli check <written.did> --against <live.did>`** exits 0
  when the live interface is still a Candid subtype of the written one, and
  1 otherwise: every written method must exist in the live service with the
  same mode, its arguments contravariant and its results covariant, and
  methods only the live service has are ignored. Each finding names its
  method and a stable code (`method_missing`, `mode_changed`,
  `method_incompatible`, the warning `special_opt_rule`, and
  `resource_limit_exceeded`) and, inside a type, the path where the check
  failed. The report prints the live interface identity, so an unchanged
  interface can be told from one that changed compatibly.
- **`projectDid(sources, methods)` and `checkCompatible(written, live)`**,
  the library functions behind the two commands, exported beside
  `didToContract` and `didToModule`, with their types (`ProjectionSuccess`,
  `CompatibilityReport`, `CompatibilityDiagnostic`, `CheckFailure`,
  `Identities`). Neither fetches anything; the package still has no runtime
  dependencies.

### Changed

- **The usage text lists the three commands.** A usage error still exits 64
  with the usage on stderr and nothing on stdout; its text now has three
  lines, one per command.

## 0.2.0 — 2026-10-06

Embeds `candid-core` 0.1.0-beta.3 and the `candid-core-ts` generator from the
same repository commit the release is dispatched from; the release record
names the exact SHA. That commit's `candid-core` source is ahead of the
0.1.0-beta.3 archive on crates.io by four changes not yet in a crate release:
a `.did` source may begin with one UTF-8 byte order mark, which beta.3 refuses
with "Unknown token"; a run of more than 256 consecutive comments between two
tokens is refused with `resource_limit_exceeded` (below), which beta.3
compiles up to a stack-dependent length and then aborts on; a type on a cycle
that passes only through `opt` is refused with `did_type_check_error`
(below), which beta.3 compiles; and the crate's Contract JSON loaders refuse
a struct written as a JSON array, which beta.3's loaders accept. No input of
this package reaches the last one: `gen`, `didToContract` and `didToModule`
read `.did` sources and never load a Contract document. Every input both
accept compiles to the same Contract and identities, and envelopes still name
0.1.0-beta.3 as their producer.

`@candid-core/schema` 0.3 breaks the 0.2 API (principal values are canonical text, collapsing opts are boxed, every `vec nat8` is a `Uint8Array`, generated modules use a new binding layout and may omit declarations, and the `./actor`, `./transport-icp`, `./forms` and `./labels` subpaths are gone); there is no compatibility layer.

The first stable release of the 0.2 line. It is published under the npm
`latest` dist-tag, so a plain `npx @candid-core/cli` runs it; pin it in a
project, `npm install --save-dev --save-exact @candid-core/cli`. The `beta`
dist-tag stays on 0.2.0-beta.1.

**It pairs with exactly one schema release.** The optional peer is exactly
`@candid-core/schema` `0.3.0` (`^0.2.0` in 0.1.0, exactly `0.3.0-beta.1` in
0.2.0-beta.1). The modules this generator emits need that release's
`Principal` export and boxed options, which 0.2.0 does not have, and the
generator and that release's `schemaFromContract` agree on blobs and on
omitted declarations only when both come from the same release. npm refuses
to install a mismatched pair (measured with npm 11.3.0: `ERESOLVE could not
resolve`, exit status 1); the 0.2.0-beta.1 entry below says npm only warns,
which was wrong for a published peer. Install the pair at exact versions.

**Upgrading from 0.1.0.** 0.2.0 is 0.2.0-beta.1 plus the changes in this
entry, so the upgrade is this entry and the 0.2.0-beta.1 entry below, read
together. Every break in either is marked **BREAKING**: from 0.2.0-beta.1, the
`$` binding layout, `$.Principal`, boxed collapsing opts, a declared primitive
naming only itself and every `vec nat8` typed `Uint8Array`, and omission in
place of refusal (`gen`'s exit status and `didToModule`'s result); from this
entry, the refusals of a long comment run and of a type on a cycle through
`opt` alone. JSDoc from `.did` doc comments and
`gen`'s several entries, `--json` and `--check` are additive. The library's
three functions keep their names and signatures, and `lib/index.d.ts` and the
command grammar are unchanged since 0.2.0-beta.1.

### A long run of comments is refused with a resource diagnostic

The embedded compiler now refuses a `.did` file with more than 256 consecutive
comments between two tokens. The upstream Candid tokenizer it embeds spends one
stack frame on each comment it skips, so the compiler counts the run first,
without recursing, and refuses an over-long one before that tokenizer runs.
Before, a long enough run exhausted the WebAssembly stack: measured with the
published tarballs (Node 24.2, macOS arm64), 0.2.0-beta.1's `gen` compiled a
file with 2 728 consecutive `//` lines and failed at 2 729 with
`internal_error` "Maximum call stack size exceeded", and 0.1.0's compiled
2 825 and crashed at 2 826 with an uncaught `RangeError`; the threshold moves
with the toolchain and the engine.

- **BREAKING (input)**: this refuses files that `gen` accepted before: any
  `.did` in the bundle with a run of 257 to about 2 700 or 2 800 comments,
  such as a license header written as `//` lines. `gen`, `didToContract` and
  `didToModule` report `resource_limit_exceeded` with resource
  `source_nesting`, limit 256 and `observed` 257, through the existing
  diagnostic path (the `{ ok: false, diagnostics }` document, or `--json`)
  with exit status 1. A longer run, which failed with `internal_error` before
  (an uncaught `RangeError` in 0.1.0), gets the same diagnostic. The CLI
  compiles at the compiler's default limits and has no flag to raise this one:
  write such a header as a single `/* */` block, which counts as one comment,
  or shorten it. Line and block comments count alike;
  blank lines between comments do not end a run, any token does, a nested
  block comment counts once, and comment markers inside a string are not
  comments.
- **Nothing else moves.** A file within the limit generates the same module and
  the same envelope, with the same identities and documentation, as before.

### The embedded generator renders on an explicit stack

- The embedded `candid-core-ts` generator keeps its pending work on an explicit
  stack instead of recursing once per type level, so generation uses constant
  call-stack depth in the Contract's nesting. It also refuses a Contract with a
  cycle that passes through no declared node, which a TypeScript type alias
  cannot spell. No `.did` source produces such a Contract (a recursive Candid
  type needs a name), so no input `gen` accepts is affected: every module it
  writes is byte-identical to what 0.2.0-beta.1 writes for the same input, and
  `didToModule`, which takes `.did` sources, returns the same result.

### A type on a cycle through `opt` alone is refused

The embedded compiler now refuses a type that lies on a cycle passing only
through `opt`: `type T = opt T;`, `type T = opt opt T;`,
`type A = opt B; type B = opt A;`, or the same through an alias. Decoding a
value that is not an `opt` at such a type unwraps `opt` without end, so no
decoder can answer for it; a generated schema for it ended in a depth refusal.

- **BREAKING (input)**: this refuses files that `gen` accepted before:
  `gen`, `didToContract` and `didToModule` report `did_type_check_error`,
  naming the type ("type T lies on a cycle that passes only through opt;
  …"), through the existing diagnostic path (the `{ ok: false,
  diagnostics }` document, or `--json`) with exit status 1, and write nothing
  for that entry. The upstream Candid checker accepts these files; the
  refusal is candid-core's own. 0.1.0 refused them too, with
  `ts_generation_refused` at generation, because each such type wraps `opt`
  in `opt`; 0.2.0-beta.1 generates them, boxed.
- **A cycle with any other constructor on it still compiles**: `type L = opt
  record { head : nat; tail : L }`, `type T = opt vec T`, `type T = record {
  a : opt T }`, `type T = opt variant { a : T }`.
- **Nothing else moves.** Every other file generates the same module and the
  same envelope, with the same identities and documentation, as before.

## 0.2.0-beta.1 — 2026-10-02

Embeds `candid-core` 0.1.0-beta.3 and the `candid-core-ts` generator from the
same repository commit the release is dispatched from; the release record
names the exact SHA. That commit's `candid-core` source is ahead of the
0.1.0-beta.3 archive on crates.io by one change not yet in a crate release: a
`.did` source may begin with one UTF-8 byte order mark, which beta.3 refuses
with "Unknown token". Every input beta.3 accepts compiles to the same Contract
and identities, and envelopes still name 0.1.0-beta.3 as their producer.

`@candid-core/schema` 0.3 betas break the 0.2 API (principal values are canonical text, collapsing opts are boxed, every `vec nat8` is a `Uint8Array`, generated modules use a new binding layout and may omit declarations, and the `./actor`, `./transport-icp`, `./forms` and `./labels` subpaths are gone); there is no compatibility layer.

The first beta of the 0.2 line, and the first prerelease of this package. It is
published under the npm `beta` dist-tag, so `latest` stays on 0.1.0; ask for it
by its exact version, `npm install --save-exact @candid-core/cli@0.2.0-beta.1`,
or run it as `npx @candid-core/cli@0.2.0-beta.1`.

**It pairs with exactly one schema release.** The optional peer moves from
`@candid-core/schema` `^0.2.0` to exactly `0.3.0-beta.1`. The modules this
generator emits need that release's `Principal` export and boxed options, which
0.2.0 does not have, and the generator and that release's `schemaFromContract`
agree on blobs and on omitted declarations only when both come from the same
release. While the two packages move in lockstep, every `@candid-core/schema`
beta is paired with a new beta of this package that raises the exact peer.
Because the peer is optional, npm does not refuse a mismatched pair: it warns
`ERESOLVE overriding peer dependency` and installs anyway, and the result does
not type-check. Install the pair at exact versions.

Every break below is marked **BREAKING**, ordered for a reader upgrading from
0.1.0: what the emitted module looks like, then what it emits for interfaces it
used to refuse, then the command line. The library's three functions keep
their names and signatures.

### Generated modules bind `$`-prefixed locals

The embedded generator changes the layout of every module it emits. A module
imports the schema runtime as a namespace and binds each declaration as a local
whose name starts with `$`, exported under its Candid name:

```ts
import * as $ from "@candid-core/schema";

type $Tokens = { e8s: bigint };
const $Tokens: $.Schema<$Tokens> = $.c.rec(() => $.c.record({ e8s: $.c.nat64 }));
export { $Tokens as Tokens };
```

- **BREAKING**: the text of every generated module changes. `import
  { Tokens, actor, type Actor } from "./ledger"` resolves exactly as before,
  so code that imports the generated names is unaffected by the layout;
  code that parses or patches the generated source, or relied on its
  module-scope names (`export type X`, `export const X`, the `c` and `Schema`
  bindings, the separate `import type { PrincipalValue }` line), must follow
  the new layout.
- **BREAKING**: declarations the generator used to refuse with
  `ts_generation_refused` now generate: names that collided with the module's
  own bindings (`c`, `Schema`, `PrincipalValue`), with the global types its
  lowerings use (`Array`, `Record`, `Uint8Array`, `Promise`), or that are
  TypeScript reserved words (`delete`, `string`). Import them by name,
  renaming where needed (`import { delete as del } from "./service"`). A
  declaration named `default` becomes the module's default export.
- Declarations named `actor` or `Actor` stay reserved: those are the
  module's own export names. They are omitted from the module rather than
  refusing it; see "Unrepresentable declarations are omitted, not refused".

### Generated modules type principals as `$.Principal`

- **BREAKING**: the embedded generator emits `$.Principal` wherever it
  emitted `$.PrincipalValue`: principal fields, func reference aliases
  (`{ principal: $.Principal; method: string }`), service aliases, and the
  `actor` schema (`$.Schema<$.Principal>`). The paired `@candid-core/schema`
  release replaces `PrincipalValue` (`{ toText(): string }`) with
  `Principal`, canonical principal text as a branded string: a decoded
  principal is that text, and encoding accepts only it, so an SDK `Principal`
  converts once with `principal(sdkPrincipal)`.
- A declaration named `Principal` still generates, bound as `$Principal` and
  exported as `Principal`.

### Options whose inner type admits `null` are boxed

- **BREAKING**: the embedded `candid-core-ts` generator no longer refuses an
  `opt` whose inner type is another `opt`, `null`, or `reserved` (its
  `TsGenError::UnrepresentableOption` is removed). Such declarations now
  generate, with the present value boxed: `opt opt nat` emits
  `{ some: bigint | null } | null` with builder `$.c.opt($.c.opt($.c.nat))`.
  Interfaces without such an `opt` are unaffected by this change.

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
  bytes do not change. The paired `@candid-core/schema` release makes
  `schemaFromContract` load every `vec nat8` as a blob too, so a generated
  module and a loaded schema agree.
- **Trade-off**: a declared alias no longer survives as the *spelling* of a
  field's type (`amount : Tokens` reads `amount: bigint`); the type is the
  same. Two structurally equal *record* declarations still collapse to the
  first name, as before.

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
- **Additive**, with one naming change: a method written `a : (x : nat) -> ()`
  now takes `x` as its parameter name where it took `arg0`, and gains an
  `@param x` block, whether or not the file has a single comment. Types,
  values and wire bytes do not move either way.

### Unrepresentable declarations are omitted, not refused

The embedded generator leaves out a declaration it cannot represent, with
everything that references it, instead of refusing the whole interface. One
exotic declaration no longer costs the module. The paired
`@candid-core/schema` release makes `schemaFromContract` leave out the same
declarations and methods, with the same reasons, so a generated module and a
loaded contract agree.

- **BREAKING (exit status)**: `candid-core-cli gen` used to print a
  `ts_generation_refused` diagnostics document and exit 1 for an interface
  with a record field or variant arm named like the `_N_` numeric-id
  rendering (`record { _0_ : nat }`), a variant arm whose payload is a
  declared `opt` of an uninhabited type (`type W = opt empty; type V =
  variant { a : W }`), or a declaration named `actor` or `Actor`. It now
  writes the module without those declarations and exits 0, printing one
  `warning: omitted …` line per omission on stderr. A script that relied on
  the non-zero exit to reject such an interface must read the warnings, or
  `omitted`, instead. Stdout's report is unchanged.
- **BREAKING**: `didToModule` returns `{ ok: true, module, omitted }` for
  those inputs where it returned `{ ok: false, diagnostics }`. No Candid
  source reaches `ts_generation_refused` any more; the code stays, reserved
  for an invalid contract graph.
- **The closure**: an omitted declaration takes with it every declaration
  that references it through any edge — a field, an alias, `vec`/`opt`/record
  nesting, a `func` type's arguments or results, a `service` type's methods —
  up to the containing declaration. A record holding a
  `service { f : (Bad) -> () }` is omitted whole, not stripped of `f`: a
  service value's wire type is its full method table, and a peer would see a
  different type. Only the actor drops individual methods, from both the
  `actor` schema and the `Actor` type, since calling a method never encodes
  the actor's own service type. The actor itself is never omitted, and a class
  actor whose init arguments reference an omitted declaration keeps every
  method (init arguments are not generated).
- **Everything kept is unchanged**: the modules for the other golden
  interfaces are unchanged by omission, and a test pins that, for an interface
  with omissions, every emitted declaration, `Actor` signature and actor
  method is byte-identical to the module generated with the omitted
  declarations deleted from the source.
- **`ModuleSuccess.omitted`** (new, always present, empty when nothing is
  omitted): an array of `{ kind, name, reason, via? }`, declarations first,
  then methods, each sorted by name. `kind` is `"declaration"` or
  `"method"`. `reason` is one of `reserved_field_name`,
  `ambiguous_variant_arm`, `reserved_export_name`, `invalid_declaration_name`
  (Contract documents only; Candid source cannot produce one) or
  `references_omitted`; `via` is present only for `references_omitted` and
  names the omitted declaration referenced. This is a serialized shape: the
  set of `reason` codes is closed, and a new code is a breaking change. The
  type is exported by name, as `Omission`.
- **The module header** lists the same entries in the same order, one line
  each, directly under the first line:
  `// Omitted: type Holder (references_omitted via Bad)`. A module that omits
  nothing has no such line.

### `gen` takes several entries, and gains `--json` and `--check`

- `candid-core-cli gen <a.did> [<b.did> …] [-o <dir>] [--json] [--check]`.
  Each entry generates its own `<stem>.ts` and `<stem>.envelope.json` into the
  one `-o` directory, byte-for-byte what a run of that entry alone writes. An
  entry's bundle is the `.did` files beneath its own directory, with the bundle
  limits applied per entry; entries sharing a directory share one read of it.
  Two entries with the same stem (compared case-insensitively) are a usage
  error (exit 64) before any work. Both rules may still change before 1.0.
- A failing entry no longer ends the run: every entry is attempted, and the
  exit code is 1 if any failed, 0 otherwise. Omissions do not fail an entry.
  For one entry the behaviour is unchanged: the same `{ ok: false,
  diagnostics }` document on stdout and exit 1.
- **`--json`** prints exactly one JSON document on stdout and nothing else
  (stderr stays empty): `{ schemaVersion, ok, check, entries, drift }`, with
  per entry its `entry`, `status` (`written`, `unchanged`, `drifted` or
  `failed`), `module` and `envelope` paths, `omitted`, and `diagnostics`. The
  shape is documented in the README and typed as `CliReport` in
  `lib/index.d.ts`; `schemaVersion` is `1`. The content-addressed identities
  stay in the human report only.
- **`--check`** generates in memory, compares byte-for-byte with the files on
  disk, writes nothing (not even the output directory) and exits 1 listing
  each missing or drifted file; it combines with `--json` (`drift` lists the
  paths).
- The CLI never reads stdin, and a test now pins that with a stdin that is an
  open pipe nobody writes to.
- **Behaviour change, not breaking**: an output file that already holds the
  generated bytes is no longer rewritten, and its line reads
  `unchanged <path>` where it read `wrote <path>`. Bytes, names and exit codes
  are unchanged; scripts matching `wrote ` on a re-run should match
  `unchanged ` too, or use `--json`.
- Every path the CLI reports, in the document and in the human output, is the
  entry or directory as passed (or `-o` joined with a file name), never a
  resolved absolute path, so the same invocation prints the same bytes from any
  working directory.
- The library (`didToModule`, `didToContract`, `init`) keeps its signatures;
  `ModuleSuccess` gains `omitted` (above), and `lib/index.d.ts` adds the
  `Omission`, `CliReport`, `CliEntryReport` and `CliEntryStatus` types.

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
