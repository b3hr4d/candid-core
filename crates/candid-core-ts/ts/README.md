# @candid-core/schema

A Zod-style schema runtime for [Candid], driven by [candid-core]'s canonical
Contract model: schema builders with static inference, structural validation,
a TypeScript-native Candid binary codec, and a loader that builds the same
schemas from a Contract document at runtime. It is the Candid layer only —
calling a canister (transports, identity, actors) belongs to the layer you
build on top of it ([below](#services-and-func-references)).

```sh
npm install @candid-core/schema
```

That is the whole install: no runtime dependencies and no peers, principal
typing included ([the decoded-value contract](#decoded-principal-values)
below).

```ts
import { c, type Infer } from "@candid-core/schema";
import { validate } from "@candid-core/schema/validate";
import { encode, decode } from "@candid-core/schema/codec";

const Account = c.record({ owner: c.principal, balance: c.nat });
type Account = Infer<typeof Account>; // { owner: PrincipalValue; balance: bigint }

const value: Account = { owner: { toText: () => "aaaaa-aa" }, balance: 5n };

validate(Account, value); // { ok: true } | { ok: false, issues }
const encoded = encode(Account, value); // { ok: true, bytes } | { ok: false, issues }
if (encoded.ok) {
  decode(Account, encoded.bytes);
}
```

Modules, each a subpath export:

- **`.`** — the schema core: the `c` builders, `Schema<in out T>` (deliberately
  invariant), `Infer`, and the node interfaces walkers narrow on — plus
  `resolveSchema` and `serviceMethods` for reading one back, since a schema
  reached by name is a `rec` indirection and a service's methods are a table.
- **`./validate`** — bounded, fail-closed structural validation; never throws
  on any value; issues carry stable codes and `$`-rooted paths — plus
  `isResultSchema` and `unwrapResult`, the schema-directed read of the
  `variant { ok; err }` convention [below](#unwrapping-okerr-results).
- **`./contract`** — build the same schemas at runtime from a canonical
  Contract JSON document — or from a one-document `ContractEnvelope` carrying
  its hash-enforced field-name table.
- **`./codec`** — the Candid binary wire format, schema-directed, with the
  spec's coercion relation on decode and explicit resource budgets. Verified
  bidirectionally against the reference implementation's vectors.

Those four are the whole export map. Earlier releases also exported
`./actor`, `./transport-icp`, `./forms`, and `./labels`; they are gone — see
[CHANGELOG.md](./CHANGELOG.md).

## Principals and `@icp-sdk/core`

This package does not depend on `@icp-sdk/core` — not at runtime, not in its
declarations, and not as a peer — so everything compiles and runs with no SDK
installed, under strict TypeScript with `skipLibCheck` off included. SDK
values still work where the schemas take a principal, because principal
typing is structural.

### Decoded principal values

`c.principal` types as `PrincipalValue` — `{ toText(): string }`, exported
from the root — which is the truth in both directions:

- **What you get.** The codec is self-contained and never constructs an SDK
  class: a decoded principal carries exactly `toText()`. It is *not* a
  `Principal` instance — `instanceof` is `false`, and class methods like
  `.toUint8Array()` do not exist on it. Code that wants the class re-wraps:
  `Principal.fromText(value.toText())`.
- **What you can give.** Validation and encoding are structural — any object
  whose `toText()` returns canonical principal text — and the SDK class
  satisfies that shape, so values built with `Principal.fromText(…)` encode
  unchanged.

## From a `.did` file

Two routes compile Candid source into the document `schemaFromContract`
consumes, and both produce exactly the schemas the generated modules carry.

With no Rust toolchain, [`@candid-core/cli`](https://www.npmjs.com/package/@candid-core/cli)
runs the same compiler and generator as WebAssembly. It writes the generated
module and a `ContractEnvelope` document, `./generated/service.envelope.json`,
which is byte-identical to the `compile --envelope` output below:

```sh
npx @candid-core/cli@0.1.0 gen ./service.did -o ./generated
```

With the Rust crate, it is two commands. Install the compiler. `candid-core`
is pre-1.0 with only prereleases on crates.io, so the version has to be
explicit: a bare `cargo install candid-core` fails with `could not find
candid-core in registry crates-io with version *`.

```sh
cargo install candid-core --version 0.1.0-beta.3 --locked
candid-core compile ./service.did --envelope > ./service.json
```

`compile --envelope` prints one self-describing document: a `ContractEnvelope`
holding the canonical `contract` plus an `extensions` map whose
`org.candid-core.field-names/v1` entry carries the field-name table. A
semantic Contract stores authoritative field-label *ids*, not text, so names
travel side-band — and envelope extensions live outside the canonical
identities by design, so carrying them never moves a `contract_id`.
`schemaFromContract` consumes the document whole:

```ts
import { readFileSync } from "node:fs";
import { schemaFromContract } from "@candid-core/schema/contract";

const built = schemaFromContract(JSON.parse(readFileSync("./service.json", "utf8")));
if (!built.ok) {
  throw new Error(JSON.stringify(built.issues));
}

built.schemas.Account; // one Schema per declaration, in declaration order
built.actor; // the service schema, when the document has one
```

The two-file flow also works: plain `compile` (no `--envelope`) prints
`{ ok, contract, source_info }`, and the named `[container, id, name]` triples in
`source_info.field_labels` — entries whose `label.kind` is `"named"` — pass as
the `names` option alongside the bare `contract`:

```ts
import { schemaFromContract, type FieldNameEntry } from "@candid-core/schema/contract";

declare const compiled: { contract: unknown };
declare const names: FieldNameEntry[];

schemaFromContract(compiled.contract, { names });
```

Both routes yield verdict-for-verdict identical schemas, and an explicit
`names` option always wins over envelope-carried names — the envelope's table
is then not consulted at all.

Positional and numeric labels carry no name and are skipped; those fields
render by the ecosystem's `_id_` convention, exactly as the generator renders
them. Every name — envelope-carried or caller-supplied alike — is
hash-enforced: it must be the Candid preimage of its id, so a table that lies
fails closed instead of quietly renaming a field. Passing no table at all is
legal and renders every field as `_id_`, which is also what
`compile --no-source-info` leaves you with.

## Services and func references

A `service` schema describes a canister interface; it does not call one.
`serviceMethods` reads its method table — name, mode, argument and result
schemas — and the codec turns each call's arguments and reply into Candid
bytes, which is everything a call layer needs from this package:

```ts
import { c, serviceMethods } from "@candid-core/schema";
import { encodeArgs, decodeArgs } from "@candid-core/schema/codec";

const Account = c.record({ owner: c.principal, subaccount: c.opt(c.vec(c.nat8)) });
const Ledger = c.service({
  balance_of: c.func([Account], [c.nat], "query"),
});

const method = serviceMethods(Ledger).get("balance_of");
if (method !== undefined) {
  const request = encodeArgs(method.args, [
    { owner: { toText: () => "aaaaa-aa" }, subaccount: null },
  ]);
  // `request.bytes` is the argument a transport sends on the `method.mode`
  // path; the reply's bytes decode with `decodeArgs(method.results, reply)`.
  void request;
  void decodeArgs;
}
```

Transports, identity, certificate verification, retries, and invoking a
decoded func reference are the call layer's job, not this package's: a func
*value* stays the inert `{ principal, method }` pair. Generated modules still
emit the typed call interface as `export type Actor = { … }` beside the
`actor` service schema, because `c.rec` erases method structure from a
schema's *type* — schemas carry values, not calls — so a call layer cannot
re-derive it from `typeof`.

## Unwrapping ok/err results

`variant { ok : T; err : E }` is the universal canister result convention, and
the usual way to unwrap one generically is to probe the decoded *value* for
`ok`/`err` keys — a guess that misfires on any record legitimately carrying
those field names, and one that cannot type the error payload. Whether a reply
*is* a result, and what each arm carries, is a schema fact:

```ts
import { c, type Infer } from "@candid-core/schema";
import { isResultSchema, unwrapResult } from "@candid-core/schema/validate";

const TransferError = c.variant({
  bad_fee: c.record({ expected_fee: c.nat }),
  too_old: c.null,
});
const TransferResult = c.variant({ ok: c.nat, err: TransferError });

isResultSchema(TransferResult); // true
isResultSchema(c.record({ ok: c.bool, err: c.opt(c.text) })); // false: a record is not a variant

declare const reply: unknown; // whatever the call decoded to

const outcome = unwrapResult(TransferResult, reply);
if (outcome.issues) {
  // Not a value of this schema at all: the issues `validate` would report.
  throw new Error(outcome.issues[0].message);
} else if (outcome.ok) {
  const blockIndex: bigint = outcome.value;
  void blockIndex;
} else {
  const failure: Infer<typeof TransferError> = outcome.error;
  void failure;
}
```

Both spellings are recognised, as *pairs*: `ok`/`err`, which Motoko's
`Result.Result` produces, and `Ok`/`Err`, which Rust's candid derive produces
— in either arm order, and with exactly those two arms. A variant that adds a
third arm is not a result, because mapping it onto two states would drop one.
A bare-tag arm (`variant { ok; err : text }`) unwraps to `null`, the single
value of the Candid `null` it declares.

An `err` arm is a value, not an exception: nothing throws for one, and a
malformed value comes back as `{ ok: false, issues }` rather than as an
exception either. A schema that is not a result variant *is* a programmer
error, and throws `TypeError`.

## Support matrix

Measured against the packed tarball — the artifact `npm pack` produces,
extracted and consumed as a dependency — with `@arethetypeswrong/cli` and
direct compiles, not inferred from the manifest.

| | |
| --- | --- |
| TypeScript | **≥ 5.0** |
| `moduleResolution` | `node16`, `nodenext`, `bundler` |
| Module format | **ESM only** — no CommonJS build ships |
| Node, from ESM | **≥ 16** (16.20, 18.20, 20.19, 22.12, 25.9 exercised) |
| Node, from CommonJS `require()` | **≥ 20.19 / ≥ 22.12**, else `await import()` |
| TypeScript, from a CommonJS project | **≥ 5.8 with `"module": "nodenext"`** |

**TypeScript 5.0** is a hard floor, and it is a *parse* error below it, not a
type error: `c.tuple` is declared with a `const` type parameter — the 5.0
feature that keeps a tuple's element types from widening — so 4.9 stops at
`error TS1139: Type parameter declaration expected.` in `dist/schema.d.ts`
before it type-checks anything.

**`node10` resolution cannot see this package at all.** There is no top-level
`main` or `types` field, only an `exports` map, so every import fails with
`TS2307` — TypeScript's own message tells you to move to `node16`, `nodenext`,
or `bundler`.

**Supporting ESM is not by itself enough for Node.** The build targets ES2020
and does not down-level, so optional chaining and nullish coalescing reach
`dist/` verbatim — they appear in four of the five modules — and those are
V8 8.0 syntax, which no Node before 14 can parse. The floor above is the
oldest release this package is actually run on rather than the oldest that
might work: 16.20.2 is exercised and passes every subpath, and nothing older
is claimed.

**From CommonJS**, the two floors are independent. At runtime, `require()` of
an ES module is what Node added in 20.19 and 22.12; below those it throws
`ERR_REQUIRE_ESM`, while `await import("@candid-core/schema")` succeeded on
every version tested. At the type level, a `"type": "commonjs"` project needs
TypeScript 5.8 *and* `"module": "nodenext"` — `"module": "node16"` is pinned
to Node 16 semantics and still refuses with `TS1479` on every compiler tested,
up to and including 7.0.

There is deliberately **no `engines` field**. Both floors are narrow — an
ES2020-capable Node for ESM, a `require(esm)`-capable one for CommonJS — and
enforcing either in install metadata would warn, or fail under
`engine-strict`, for consumers it does not apply to. They are documented here
instead.

## The domain shapes (a deliberate decision)

Types describe the modern domain, not the agent-js runtime shapes: `opt T` is
`T | null` (collapsing opts fail closed at generation), variants are
`{ tag, value }` discriminated unions, anonymous `vec nat8` is `Uint8Array`,
`nat`/`int`/64-bit integers are `bigint`, and principals are the structural
[`PrincipalValue`](#decoded-principal-values) the codec actually delivers.
Compatibility with agent-js value shapes is an explicit non-goal, recorded on
the project's issue tracker.

## Verification

Every release runs a packaged-consumer gate on the commit it publishes from,
before anything is uploaded: the tarball `npm pack` produces at that commit is
extracted into a clean project, compiled under strict TypeScript with
`skipLibCheck` off, and executed — root and every subpath export, a real
encode/validate round-trip. The publish step then builds and uploads from that
same commit. The TypeScript in this file
is compiled by that same gate, against the packed artifact, so an example here
cannot drift away from the package it documents. The codec is additionally
verified in both directions against the reference implementation's own wire
vectors.

## Provenance

Generated bindings, the Contract model, and the conformance gates live in the
[candid-core] repository; this package versions independently (pre-1.0).
[CHANGELOG.md](./CHANGELOG.md) ships in the tarball and records, for every
release, the `candid-core` generator version it pairs with.

[Candid]: https://github.com/dfinity/candid
[candid-core]: https://github.com/b3hr4d/candid-core
