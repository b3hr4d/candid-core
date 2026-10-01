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

## 0.3.0-beta.1 — 2026-10-01

Pairs with `candid-core` 0.1.0-beta.3.

`@candid-core/schema` 0.3 betas break the 0.2 API (principal values are canonical text, collapsing opts are boxed, every `vec nat8` is a `Uint8Array`, generated modules use a new binding layout and may omit declarations, and the `./actor`, `./transport-icp`, `./forms` and `./labels` subpaths are gone); there is no compatibility layer.

The first beta of the 0.3 line, and the first prerelease of this package. It is
published under the npm `beta` dist-tag, so `latest` stays on 0.2.0 and a plain
`npm install @candid-core/schema` does not select it; ask for it by its exact
version, `npm install --save-exact @candid-core/schema@0.3.0-beta.1`. A
`^0.2.0` range does not admit it either: under npm's pre-1.0 caret rules a
minor is a breaking release. The package stays 0.x through the 4.0 release
of ic-reactor, the call layer built on it; 1.0 follows a stability window after
that, not with it.

The package becomes the Candid layer only — schemas, validation, the codec and
the Contract loader — and its public shape changes where a call layer built on
top of it needs it to. Every break is listed below and marked **BREAKING**,
ordered for a reader upgrading from 0.2.0: what stops resolving, then what
changes shape, then what changes behaviour. No byte a 0.2.0 encoder wrote is
read differently, and no Contract document or identity moved; one encoder
change (the structural type table) makes some byte strings shorter, never
different in meaning.

**The CLI pairs with this exact version.** `@candid-core/cli` 0.2.0-beta.1
declares this package as a peer at exactly `0.3.0-beta.1`: the modules its
generator emits need the `Principal` export and the boxed options introduced
here, which 0.2.0 does not have, and the generator and this package's
`schemaFromContract` agree on blobs and on omitted declarations only when both
come from the same release. While the two packages move in lockstep, every
beta of this package is paired with a new CLI beta that raises that exact peer.

The Contract documents this package loads are unchanged, so the
`candid-core compile --envelope` on-ramp in the README works with the published
`candid-core` 0.1.0-beta.3.

### The package is the Candid layer only

The call stack — transports, identity, actors — and form metadata move
downstream to ic-reactor 4, which builds them on top of these four subpaths. On its own this change
moved no wire encoding, validation verdict, issue code or inferred type: the
modules that remain shipped the same code as 0.2.0, with only documentation
comments edited, until the changes in the sections after this one.

- **BREAKING**: `./actor` is removed — `createActor`, `callFunc`, `ActorError`,
  and the `Transport`, `CallTarget`, and `ActorOptions` types. Calling a
  canister, and invoking a decoded func reference (what `callFunc` did — the
  ledger's archived-blocks callback, for one), is the downstream call layer's
  job now; a func value stays the inert `{ principal, method }` pair. What a
  call layer builds on is unchanged: `serviceMethods` reads a service's
  method table, `encodeArgs`/`decodeArgs` move its bytes, and generated
  modules still emit the `actor` service schema and the `Actor` call
  interface type.
- **BREAKING**: `./transport-icp` is removed — `httpTransport` and
  `HttpTransportOptions`. Its convenience path built an `HttpAgent` with no
  identity: a second call path, anonymous by default, beside whatever
  identity-aware transport an application actually uses.
- **BREAKING**: `./forms` is no longer exported — `formModel`, `formNodeAt`,
  and the `FormNode`, `FormCommon`, `FormArm`, and `FormControl` types. The
  code stays in the repository, type-checked and tested, but is not built
  into the tarball.
- **BREAKING**: `./labels` is no longer exported — `candidLabelHash`,
  `numericKeyId`, `isNumericShapedName`, `fieldIdOfKey`, `utf8BytesStrict`,
  and `utf8Decode`. The module still ships, because the codec and the
  Contract loader import it, but it is not an entry point.
- Importing any of the four now fails at resolution —
  `ERR_PACKAGE_PATH_NOT_EXPORTED` in Node, `TS2307` in TypeScript — and the
  packaged-consumer gate asserts both.
- **BREAKING**: the optional `@icp-sdk/core >= 6` peer is dropped; only
  `./transport-icp` ever used it. The package now declares no runtime
  dependency and no peer of any kind. SDK `Principal` values convert to this
  package's principal type with `principal()`; see "Principals are canonical
  text" below.
- The Node support matrix loses its `>= 20.19` row, which applied only to
  `./transport-icp`; the ESM floor of every remaining subpath is Node 16.

### Principals are canonical text

A principal's domain value is now its canonical text, as a branded string.
The `{ toText }` closure it replaces could not be serialized
(`JSON.stringify` gave `{}`), cloned (`structuredClone` threw
`DataCloneError`, so a decoded reply could not cross `postMessage`, a
persisted query cache, or a server/client boundary), or compared (two decodes
of one principal were not `===`). Wire bytes are unchanged for every
principal that encoded before; Contract JSON and identities are untouched.

- **BREAKING**: the root export `PrincipalValue` (`{ toText(): string }`) is
  removed, and so is its alias `DecodedPrincipal` in `./codec`. No
  deprecation alias remains: one would keep the object shape alive. In their
  place the root exports:
  - `Principal` — `string & { readonly [brand]: true }`: canonical principal
    text, branded in the type system only;
  - `principal(input)` — the one conversion point. It takes a string or an
    object with `toText()` (an `@icp-sdk/core` `Principal`, for one) and
    returns the text as a `Principal`, or throws `TypeError`. It refuses
    rather than repairs: upper case, a missing, misplaced, leading, trailing
    or doubled dash, a checksum mismatch, non-zero padding bits in the last
    character, an id over 29 bytes, and the empty string all throw, as does
    an input that is neither a string nor an object with `toText()`.
    `"aaaaa-aa"` (the management canister) and `"2vxsx-fae"` (the anonymous
    principal) are canonical;
  - `isPrincipal(value)` — the matching guard: a string whose text is
    canonical.
- **BREAKING**: `c.principal` is `PrimitiveSchema<Principal>`,
  `FuncValue.principal` is `Principal`, and `ServiceSchema` extends
  `Schema<Principal>`, so `Infer` yields `Principal` in all three places.
- **BREAKING**: decoding returns the canonical text string — for the
  primitive, a func reference's `principal`, and a service reference —
  instead of a `{ toText }` object. Code that called `.toText()` on a decoded
  value now holds the text itself; code that needs an SDK instance converts
  with the SDK's `Principal.fromText(value)`. `JSON.stringify` of a decoded
  principal is now its text, the portable form candid-core's host-value ABI
  specifies.
- **BREAKING**: validation checks the text. A principal must be a string
  holding canonical text; an object with `toText()`, an SDK `Principal`
  instance included, and a non-canonical string fail with `invalid_type`.
  Before, `validate` accepted any object with a `toText` function, even one
  returning garbage that `encode` then refused.
- **BREAKING**: encoding is strict and agrees with validation case for case:
  it accepts exactly the strings `validate` accepts and refuses everything
  else with the same code (`invalid_type`) at the same path. It no longer
  calls `toText()`, so SDK values go through `principal()` once, at the
  caller's boundary. `invalid_principal` is now reported only by decoding (an
  id over 29 bytes, an opaque reference).
- Equal principal text encodes to equal bytes whatever built the schema —
  generated, loaded, hand-built or `rec`-wrapped — in the primitive, func and
  service positions alike: the structural-table guarantee below, completed
  for principals now that the value is the text itself.
- A string longer than 63 characters — the longest canonical text — is
  refused before any work proportional to its length.
- The principal text form moves out of the codec into an internal module,
  `dist/principal-text.js`, shared by the root entry, the validator and the
  codec; it is not an entry point. `./codec` still exports
  `principalTextFromBytes` and `principalBytesFromText`, unchanged. The root
  entry now loads that one dependency-free module at runtime, where it
  imported nothing before; the validator is still not in its runtime graph.
- The internal forms model documents the `principal` control's value as the
  `Principal` string.
- **Generated modules**: `@candid-core/cli`'s generator types principals as
  `$.Principal` where it emitted `$.PrincipalValue` — principal fields, func
  references (`{ principal: $.Principal; method: string }`), service
  references, and the `actor` schema. A declaration named `Principal` still
  generates. With `TsOptions::principal_import` set to another module, the
  generator imports `Principal` from it, and that module must re-export this
  package's `Principal`: the brand makes the type nominal, so the generated
  `Schema<…>` annotations compile against no other type.

### Options whose inner type admits `null` are boxed

An `opt` whose inner type is another `opt`, `null`, or `reserved` could not
be carried by `T | null`: `None` and `Some(None)` were the same `null`. Such
interfaces used to be refused outright, and a hand-built `c.opt(c.opt(…))`
silently decoded `Some(None)` as `None`. Exactly those opts now carry their
present value as `{ some: v }`; every other opt is unchanged in type, value,
and wire bytes.

- **BREAKING**: `opt opt T`, `opt null`, and `opt reserved` are
  `{ some: T } | null` in both value and type shape, for hand-built,
  Contract-loaded, and generated schemas alike: `Infer<typeof
  c.opt(c.opt(c.nat))>` is `{ some: bigint | null } | null`, validation and
  encoding require `{ some: v }` for a present value (a bare `v` is
  `invalid_type`, and `undefined` is no longer read as present under
  `opt reserved`), and decoding returns `{ some: v }`. The decision is made
  on the resolved inner node, so declared aliases and recursive types
  (`type Chain = opt Chain`) box too, while `opt empty` stays `null`. Wire
  bytes and the decoding coercion rules are unchanged.
- **BREAKING**: `ContractIssueCode` in `./contract` loses
  `unrepresentable_option`; `schemaFromContract` now loads the documents it
  used to refuse with it.
- **BREAKING**: `OptSchema<T>` now extends `Schema<OptDomain<T>>` rather
  than `Schema<T | null>`. The root export gains the `OptDomain<T>` type
  and the runtime predicate `isBoxedOpt(schema)`, which answers whether a
  schema is an opt whose present values are boxed.
- The internal forms model's `optional` control gains `boxed: boolean`; a
  boxed optional's inner node sits at `….some`, which `formNodeAt` resolves.
  (`./forms` is no longer exported; see above.)

### `schemaFromContract` builds a blob for every `vec nat8`

The loader's blob rule matches the generator's. No wire encoding, issue code,
export or type moved; one loaded value domain did.

- **BREAKING**: `schemaFromContract` used to build `c.blob()` for a `vec nat8`
  only when its `nat8` element node was not itself a declaration. A Contract
  that declares an alias of `nat8` (`type Byte = nat8`) therefore loaded every
  `vec nat8` in the interface — `blob` and `vec Byte` alike — as `c.vec(c.nat8)`,
  a `number[]`. Every `vec nat8` now loads as `c.blob()`, a `Uint8Array`, so
  for a Contract with a declared `nat8` alias the loaded schemas accept a
  `Uint8Array` where they accepted a `number[]`, and refuse the `number[]`.
  Contracts with no such alias load exactly as before. The bytes are the same
  either way: `blob` and `vec nat8` share one type-table entry and one wire
  encoding, so an existing message decodes under either shape.
- **Why**: one unrelated declaration changed the value domain of every `blob`
  in the interface, breaking the locality a declaration's meaning should have.
  A declared primitive now names only itself, in the loader and in the
  generator (see `@candid-core/cli`'s changelog), and the crosscheck that holds
  the loaded schemas to the generated ones is extended to the new `fidelity`
  fixture. A generated module and a loaded schema from different sides of
  this change disagree about `vec Byte`, which is one reason the CLI peers this
  exact version.

### `schemaFromContract` omits what the generator omits

The loader leaves out exactly the declarations and actor methods a generated
module leaves out for the same Contract, with the same reasons, and says so.
No wire encoding, validation verdict or issue code moved; an invalid document
still fails whole, with `issues`. The golden crosscheck holds the loader's
list and the generator's equal, entry for entry, on the new `omissions`
fixture.

- **BREAKING**: a name-table entry whose name is shaped like the `_N_`
  numeric-id rendering and honestly hashes to its id — a Candid field
  genuinely named `_123_` — no longer fails the document with
  `invalid_name_table`. Such a name never becomes a schema key (as one, it
  would read back as numeric id N and encode the wrong wire id); the
  declaration that would render it is left out instead, with everything that
  references it. An `_N_`-shaped name that does not hash to its id is still a
  lying entry and still refused; an entry that names no rendered field is
  ignored, as for any other name.
- **BREAKING**: declarations that loaded before are now left out of
  `schemas`, together with every declaration and actor method that
  references them, because a generated module cannot represent them: a
  declaration named `actor` or `Actor` (the generated module's own export
  names), a variant whose arm payload is a declared `opt` of an uninhabited
  type, and a declaration whose name is not identifier-shaped
  (`[A-Za-z_$][A-Za-z0-9_$]*`) — which reverses the loader's former
  deliberate divergence from the generator on such names, for parity.
  Omission follows every reference, nested `func` and `service` types
  included, up to the containing declaration; the actor loses only the listed
  methods and is never itself omitted.
- **BREAKING (type)**: the `ok` branch of `SchemaFromContractResult` gains a
  required `omitted: readonly { kind, name, reason, via? }[]` — declarations
  first, then methods, each sorted by name in code-point order, empty when
  nothing is left out. `kind` is `"declaration"` or `"method"`; `reason` is
  one of the closed set `reserved_field_name`, `ambiguous_variant_arm`,
  `reserved_export_name`, `invalid_declaration_name`, `references_omitted`;
  `via` names the omitted declaration a `references_omitted` entry
  references. Reading results is unaffected; code that constructs an `ok`
  result by hand must add the field. The entry types are not exported by name.

### Encoded bytes no longer depend on how a schema was built; bad options throw

Two fail-open behaviours closed. Neither changes a validation verdict, an
issue code, a decoded value, or a generated binding.

- **BREAKING**: `encode` and `encodeArgs` write a *structural* type table.
  The table used to be keyed by schema-object identity, so the same value
  encoded through a generated module and through `schemaFromContract` could
  produce different bytes — the ledger fixture's `TransferArg` wrote an
  11-entry table through the generated schema and a 7-entry one through the
  loaded schema. Anything keying a cache or deduplicating requests on
  argument bytes saw two keys for one call. Equal Candid types now produce
  equal entries, written once: bytes depend on the values and the types, not
  on whether the schema was generated, loaded or hand-built, how it shares or
  duplicates nodes, or in which order a record's schema or value spells its
  keys. `blob` and `vec nat8` share one entry. A schema with no repeated
  structure writes exactly the bytes it wrote before — every checked-in wire
  golden is unchanged — and a schema with repeated anonymous structure now
  writes a smaller table, still valid Candid that every decoder reads; for
  the ledger vectors it is byte-for-byte what the `candid` crate writes.
  Recursive types are canonical per knot: schemas generated from or loaded
  from a Contract (one knot per recursive node, the Contract canonicalizer
  having already minimised the graph) agree, while two separately hand-built
  knots for one recursive type are not merged — cyclic minimisation is a
  non-goal, and the case is pinned. `maxTypeTableEntries` still charges one
  entry per distinct composite schema node the encoder's walk meets, before
  merging, so where it refuses is unchanged. The rewrite is iterative and
  linear in the table's size.
- **BREAKING**: every entry point that takes an options object — `validate`
  and `unwrapResult` in `./validate`, `encode`, `encodeArgs`, `decode` and
  `decodeArgs` in `./codec`, `schemaFromContract` in `./contract` — throws
  `TypeError` on an own key it does not define (a misspelled limit used to
  apply the default silently; `maxIssues` passed to `encode` is now an error
  too), and on a limit that is not a non-negative safe integer: `NaN`, a
  negative, a fraction, a string, `null`, and `Infinity` all throw. A `NaN`
  limit used to switch its bound off entirely (`validate` accepted a
  depth-300 value under `maxDepth: NaN`) and a string one was compared as a
  string. Options are code, not input, so this is a `TypeError` like the
  ones `resolveSchema`, `serviceMethods` and `unwrapResult` already throw,
  not a new issue code; it is raised before anything else is read.
  `undefined` still means "use the default", and `0` is still a valid,
  fail-closed limit. Each option is read from the caller's object exactly
  once, into a frozen snapshot that the whole call — nested calls included —
  reads instead, so a getter or Proxy cannot pass the check with one value
  and run with another; an options object whose getter or Proxy trap throws
  while being read raises a `TypeError` naming the entry point, with the
  original exception as its `cause`. Values, byte strings and Contract
  documents still never make these functions throw. Refusing `Infinity` may
  still be revisited before 1.0: a trusted host that wants no practical bound
  passes a large safe integer.
- Two internal modules ship in `dist/` without an export: `options.js` (the
  shared option check) and `typetable.js` (the structural table). Deep
  imports of them fail like every other internal module.

### Iterative walkers: the configured limits are the only bounds

`validate`, `encode`/`encodeArgs` and `decode`/`decodeArgs` no longer recurse
on the JavaScript call stack once per nesting level: each keeps its work on an
explicit stack of frames. No wire encoding, golden or wire vector moved, and
every issue code, `$`-path, message, `resource_limit` triple and first-issue
precedence is what it was, with the one deliberate exception below.
`DEFAULT_MAX_DEPTH` stays 256 and every other limit keeps its meaning.

- **Deep data within raised limits now works, everywhere, every time.** With
  `maxDepth` and `maxElements` raised, a 100,000-level value — a Motoko-style
  linked list, an ICRC-3 `Value` tree, a nested `vec` — validates, encodes and
  decodes on any engine, independent of its stack size, with the same result
  on every call. In 0.2.0 the recursive walkers overflowed the host stack from
  about 1,500 (encode), 2,000 (validate) and 2,600 (decode) levels on Node, at
  a point that moved with the engine's JIT state.
- **Hostile depth is refused after bounded work.** A reply nested a million
  levels deep is refused with `value_depth` at `maxDepth + 1` after charging
  work proportional to `maxDepth` (256 elements at the default, 10,000 at
  `maxDepth: 10_000`), whatever lies beyond; the suite pins the exact charge.
- **Changed: `encode` charges `maxDepth` for Candid nesting depth, and only
  for that.** The property: any type the candid-core compiler accepts encodes
  through its generated module at the default limits, however it is split
  into declarations. The type-table walk charges one depth unit per
  combinator level at which it opens an entry (the argument's type at 0) and
  nothing for `rec` hops, which are aliases and lazy edges — the count the
  compiler bounds with `max_type_depth` (256), where an alias adds no depth
  either. A chain of `rec` hops resolving one reference is capped on its own
  at `maxDepth` (a generated module needs one or two, a loaded schema one).
  Before, the walk counted rec hops and combinators together and checked the
  sum only at rec hops: a compiler-accepted type reached through many aliases
  (for example 250 aliases each a `vec` of the next) and any
  `schemaFromContract` schema deeper than about 128 levels were refused with
  `value_depth`, while a hand-built static schema with no `rec` in it was not
  checked at all and overflowed the stack. Now the first are accepted, and
  the last — `c.vec` nested 20,000 times, say — is refused with `value_depth`
  at the first composite at Candid depth 257 (`observed` 257, path `$`),
  after 257 entries. Primitives open no entry and are not charged. A schema
  object reused at several positions is written once but charged at each,
  for the depth it spans there, so a composite first met shallow cannot carry
  a deep use past the limit; a back edge closing a recursive knot (including
  one reached again through an alias of the knot) is not charged, as Candid
  does not expand a type inside itself. Visible only in a schema's type
  table: such an encode may now be accepted where it was refused, refused
  where it was accepted (a shallow value in a hand-built schema nested past
  the limit), or refused with a different `observed`, or ahead of a value
  issue, since the type table is walked first. A value nested through every
  level of a deep type still meets `maxDepth` in the value walk, which counts
  rec hops as `validate` and `decode` do; that is unchanged. The worst case
  for a hostile hand-built schema is `maxDepth × (maxDepth + 2)` thunk calls
  per path (a full 256-hop chain at every level), pinned in the suite.
- **Faster deep records.** `encode` no longer copies a record's field bytes
  once per enclosing record to put them in wire order, so deeply nested
  records encode in linear rather than quadratic time. Bytes are unchanged.

### Stack exhaustion is its own resource

One closed type union gains a member, and a host stack overflow is reported
as what it is. No wire encoding, no validation verdict and no other issue code
moved.

- **BREAKING**: `resource_limit_exceeded` issues gain a new `resource` member,
  `"stack"`, in both closed unions that enumerate resources —
  `ResourceLimitInfo["resource"]` in `./validate` (now `"value_depth" |
  "value_elements" | "stack"`) and `CodecResourceLimitInfo["resource"]` in
  `./codec`. This is a **type-level** breaking change: a consumer's exhaustive
  `switch` over either union stops compiling until it handles `"stack"`.
  `ContractIssue["resource_limit"]` reuses the `./validate` type, so its
  declared type widens too, though the contract loader is non-recursive and
  never produces it.
- **A host stack overflow is no longer mislabelled.** When the engine's own call
  stack ran out mid-walk, 0.2.0's `validate` and `encode` reported
  `unreadable_value` ("the value threw while being inspected") and `decode`
  reported `unsupported_schema` ("the schema threw while being traversed").
  Neither was true. All three now report `{ code: "resource_limit_exceeded",
  resource_limit: { resource: "stack", limit, observed } }`, where `limit` is
  the call's effective `maxDepth` and `observed` the deepest depth the walk had
  reached when the engine refused. Tell `stack` from `value_depth` by
  `resource`: `observed` is usually below `limit`, but not always.
- **When this can happen.** With the walkers iterative (above), only when user
  code a walk calls — a getter, a Proxy trap, a `rec` thunk — recurses too
  deeply itself. No depth of value, message or schema produces it.
- **How an overflow is recognised.** The engine's error is a `RangeError`
  reading "Maximum call stack size exceeded" (V8 and JavaScriptCore) or an
  `InternalError` reading "too much recursion" (SpiderMonkey, which does not
  throw a `RangeError`). Detection matches on `name` and `message`, never
  `instanceof`, and answers no for any other `RangeError` (an invalid array
  length, an oversized BigInt), for a plain `Error` quoting the words, and for
  any thrown value it cannot safely read. A getter or `rec` thunk that throws an
  error with exactly this name and message is classified the same way — the
  label can be forged, the fail-closed outcome cannot. The detector is internal
  and adds no export to any subpath.
- **User-thrown errors keep their labels.** A getter that throws an ordinary
  error is still `unreadable_value`, and a `rec` thunk that throws during decode
  is still `unsupported_schema`; the choke points were not widened.

### Generated modules bind `$`-prefixed locals

Nothing in this package changes for this: no export, type, verdict or byte.
The generator in `@candid-core/cli` now emits modules that import this package
as a namespace (`import * as $ from "@candid-core/schema"`) and bind every
declaration as a `$`-prefixed local exported under its Candid name, so a
Candid declaration named `c`, `Schema`, `Array` or `delete` no longer collides
with the module's own bindings. The `Schema` documentation comment that quoted
the old generated line is updated.

### The declarations

- `schema.d.ts` grows from 640 lines to 732, with the documentation of the new
  `Principal`, `principal()`, `isPrincipal()`, `OptDomain` and `isBoxedOpt`.
  Every shipped declaration still stands on its own, with no internal issue
  number, and compiles under strict TypeScript without `skipLibCheck`.

## 0.2.0 — 2026-08-24

Pairs with `candid-core` 0.1.0-beta.3.

The editor hover, the npm page, a small introspection surface on the root
export, result unwrapping on `./validate`, one-document contract loading on
`./contract`, the `@icp-sdk/core` transport adapter as the new
`./transport-icp` subpath — and one deliberate **type-level breaking change**:
decoded principal values now type as the structural `PrincipalValue` instead
of the SDK `Principal` class (details below; the only code it breaks was
already broken at runtime). Runtime behavior is unchanged throughout — no
wire encoding and no validation verdict moved, and no existing issue code
changed (`./contract` adds one code, `invalid_extension_name`).

The minor is what carries that break. Under npm's pre-1.0 caret rules a
`^0.1.1` dependency accepts `0.1.2` and refuses `0.2.0`, so releasing the new
principal typing as a patch would have upgraded every existing dependent into
it unasked.

### Decoded principal values

- **BREAKING**: `c.principal` types as `PrincipalValue` — `{ toText(): string }`,
  exported from the root — and `FuncValue.principal` and service schemas
  moved with it. This is the type surface telling the truth: the codec is
  self-contained and never constructs an SDK class, so a decoded principal
  has always carried exactly `toText()` — `instanceof Principal` was `false`
  and `.toUint8Array()` threw. The compiler used to accept that crashing
  code; now it refuses it. **Type-level breaking change** for consumers who
  called class methods on *decoded* values — code that never worked at
  runtime. Encode-side code is untouched: SDK `Principal` instances satisfy
  the structural shape and encode unchanged, proven by suite against the
  real pinned SDK.
- **The shipped declarations no longer import `@icp-sdk/core` anywhere
  outside `./transport-icp`.** The failure 0.1.1 could hand a consumer — a missing peer
  failing as `TS2307` deep in `node_modules`, or silently degrading
  `Principal` to `any` under `skipLibCheck` — is not mitigated but gone:
  every subpath except the transport compiles and runs with no peer
  installed, asserted by the packaged-consumer gate in both directions.
  `npm install @candid-core/schema` is now the whole install for everything
  but the transport.
- **The generator emits the structural type**: `TsOptions::principal_import`
  now defaults to `@candid-core/schema` and the emitted import is
  `import type { PrincipalValue }`. The reserved-declaration-name set
  tracks whichever binding the generated modules actually import, so it moved
  with the import: `PrincipalValue` is now refused as a declaration name, and `Principal` — no longer a binding
  generated modules reference — is allowed. Goldens regenerated and
  reviewed as a mapping change; the `tsc` invariance gate is untouched and
  stays green.
- **`DecodedPrincipal` in `./codec` is now an alias of `PrincipalValue`** —
  the same shape it always was, stated in one place.

### The icp-sdk transport adapter

- **`httpTransport` is exported from the new `./transport-icp` subpath**
  (decision recorded on the tracker: a subpath, not a new package — no new
  permanent npm name for one small module, and the already-declared optional peer
  carries it). It builds a `Transport` over `@icp-sdk/core`'s `HttpAgent`:
  `query` throws a plain `Error` naming the reject code and message; `call`
  returns the certificate-verified reply bytes v6's one-shot `agent.update`
  resolves with. Options are exactly `host`, `rootKey`, and a pre-built
  `agent` for everything beyond them (identity, retries, ingress options) —
  the pre-built agent travels alone, and combining it with the other options
  throws `TypeError`. No logging hook; wrap the returned `Transport`.
- **Importing the subpath is what makes the peer a runtime requirement**, and
  the range `>= 6` is load-bearing: older majors resolved `update` at
  submission and need their own submit-and-poll adapter. Consumers who never
  import `./transport-icp` need no `@icp-sdk/core` at all — not at runtime and
  not in their types — and the package stays `sideEffects: false`.
- **Tested against the real pinned `@icp-sdk/core@6.1.0` over a mock
  `fetch`**, not against a stubbed agent: the replied and rejected query
  paths, TypeError on the refused option combination, effective-canister-id
  routing (the management-canister shape that needs it), and a certified
  reply whose fabricated certificate is genuinely BLS-signed by a root key
  generated in the test — plus the adversarial half, where the same reply
  under a different root key must be refused, proving the transport's
  root-key wiring is what verification consulted. The README's actor section
  now shows `createActor(…, httpTransport(…))` as the primary example,
  replacing the former inline adapter.

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
  `schema.d.ts` grows from 149 lines to 640 as a result.
- **Shipped doc comments stand on their own.** 0.1.1's declarations cited
  three internal issue numbers across four comments — links a consumer's
  editor cannot follow. Those are gone, and the packaged-artifact gate now
  refuses a tarball whose declarations contain any of them.
- **The agent-adapter sketch is out of `dist/actor.js`.** It was a `//`
  comment, so declaration emit stripped it: it never reached hover, and it
  dropped `effectiveCanisterId`, which would have mis-routed the one call
  shape that needs it. A compiled, tested adapter ships as `./transport-icp`
  instead, and the README imports it rather than asking a reader to paste one.

### The npm page

- **The README is an on-ramp rather than a summary.** It opens with the install
  command — the whole install, since no subpath but `./transport-icp` needs the
  peer; states the decoded-principal contract in both directions, what a decoded
  value carries and what encoding accepts; reaches a canister through the
  shipped `httpTransport` rather than an adapter the reader must paste; and
  documents the `.did` → schemas route that exists today (`cargo install
  candid-core`, then `candid-core compile --envelope` into
  `schemaFromContract`).
- **A support matrix**, measured rather than inferred: TypeScript ≥ 5.0 (a
  *parse* error below it — `c.tuple`'s `const` type parameter);
  `node16`/`nodenext`/`bundler` resolution only, `node10` cannot resolve the
  package at all; ESM-only, on Node ≥ 16 for the seven peer-free subpaths and
  ≥ 20.19 for `./transport-icp`, whose pinned SDK tree declares it (the build
  targets ES2020 and does not down-level, so `?.` and `??` reach `dist/`); and,
  from CommonJS, Node ≥
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
