# @candid-core/schema

A Zod-style schema runtime for [Candid], driven by [candid-core]'s canonical
Contract model: schema builders with static inference, structural validation,
a TypeScript-native Candid binary codec, typed actors over a transport-only
agent, and UI-agnostic form metadata.

```sh
npm install @candid-core/schema @icp-sdk/core
```

Install both. `@icp-sdk/core` is a **type-only peer dependency** — nothing
imports it at runtime, but the shipped declarations reference its `Principal`,
so a TypeScript consumer without it either fails to compile or silently loses
the type. [What exactly goes wrong](#the-type-only-peer-dependency) is worth
reading once.

```ts
import { c, type Infer } from "@candid-core/schema";
import { validate } from "@candid-core/schema/validate";
import { encode, decode } from "@candid-core/schema/codec";
import { Principal } from "@icp-sdk/core/principal";

const Account = c.record({ owner: c.principal, balance: c.nat });
type Account = Infer<typeof Account>; // { owner: Principal; balance: bigint }

const value: Account = { owner: Principal.fromText("aaaaa-aa"), balance: 5n };

validate(Account, value); // { ok: true } | { ok: false, issues }
const encoded = encode(Account, value); // { ok: true, bytes } | { ok: false, issues }
if (encoded.ok) {
  decode(Account, encoded.bytes);
}
```

Modules, each a subpath export:

- **`.`** — the schema core: the `c` builders, `Schema<in out T>` (deliberately
  invariant), `Infer`, and the node interfaces walkers narrow on.
- **`./validate`** — bounded, fail-closed structural validation; never throws
  on any value; issues carry stable codes and `$`-rooted paths.
- **`./contract`** — build the same schemas at runtime from a canonical
  Contract JSON document plus a hash-enforced field-name table.
- **`./codec`** — the Candid binary wire format, schema-directed, with the
  spec's coercion relation on decode and explicit resource budgets. Verified
  bidirectionally against the reference implementation's vectors.
- **`./actor`** — `createActor`/`callFunc` over a two-method byte-pipe
  `Transport`; an `@icp-sdk/core` agent adapts in the one short module
  [below](#calling-a-canister), and the agent never sees a schema.
- **`./forms`** — form-generation metadata: per-kind controls, constraints,
  labels, lazy recursion, and validation-issue-to-form-node resolution.
- **`./labels`** — the Candid label hash and the `_N_` rendering convention.

## The type-only peer dependency

The `principal` primitive types against `@icp-sdk/core`'s `Principal`, so the
shipped declaration files import that type. npm's install metadata marks the
peer `optional` because nothing imports it at *runtime* — plain JavaScript
consumers need nothing, and no bytes from it are bundled — but a TypeScript
consumer does need it installed, and neither failure mode says so:

- **Without it, and with `skipLibCheck` off**, compilation fails inside
  `node_modules`, pointing at this package's own declarations rather than at
  anything you wrote:

  ```
  node_modules/@candid-core/schema/dist/schema.d.ts(1,32): error TS2307:
    Cannot find module '@icp-sdk/core/principal' or its corresponding type declarations.
  ```

  The fix is `npm install @icp-sdk/core`. The error never says so.

- **Without it, and with `skipLibCheck: true`** — the common application
  setting — the build is *green* and `Principal` degrades to `any`. Nothing
  warns. `Infer<typeof Account>["owner"]` becomes `any`, every principal-typed
  field stops being checked, and inference downstream of one weakens with it.
  This is the worse of the two outcomes, because it looks like success.

Any package providing that subpath's types satisfies the requirement; it does
not have to be `@icp-sdk/core` itself.

## From a `.did` file

There is no JavaScript-only path from Candid source to schemas yet — compiling
`.did` needs the Rust crate — but the route that exists today produces exactly
the schemas the generated modules carry, and it is two commands.

Install the compiler. `candid-core` is pre-1.0 with only prereleases on
crates.io, so the version has to be explicit: a bare `cargo install
candid-core` fails with `could not find candid-core in registry crates-io with
version *`.

```sh
cargo install candid-core --version 0.1.0-beta.2 --locked
candid-core compile ./service.did > ./service.json
```

`compile` prints one JSON document holding a canonical `contract` and a
`source_info` provenance sidecar. Both matter: a semantic Contract stores
authoritative field-label *ids*, not text, so the names travel separately in
`source_info.field_labels`. Feed the two to `schemaFromContract`.

```ts
import { readFileSync } from "node:fs";
import { schemaFromContract, type FieldNameEntry } from "@candid-core/schema/contract";

interface CompiledLabel {
  container: number;
  id: number;
  label: { kind: "named"; name: string } | { kind: "positional" };
}

const compiled = JSON.parse(readFileSync("./service.json", "utf8")) as {
  contract: unknown;
  source_info?: { field_labels: CompiledLabel[] };
};

const names: FieldNameEntry[] = [];
for (const entry of compiled.source_info?.field_labels ?? []) {
  if (entry.label.kind === "named") {
    names.push([entry.container, entry.id, entry.label.name]);
  }
}

const built = schemaFromContract(compiled.contract, { names });
if (!built.ok) {
  throw new Error(JSON.stringify(built.issues));
}

built.schemas.Account; // one Schema per declaration, in declaration order
built.actor; // the service schema, when the document has one
```

Positional labels — Candid's tuple syntax — carry no name and are skipped;
those fields render by the ecosystem's `_id_` convention, exactly as the
generator renders them. Every entry you do pass is hash-enforced: a name must
be the Candid preimage of its id, so a table that lies fails closed instead of
quietly renaming a field. Passing no table at all is legal and renders every
field as `_id_`, which is also what `compile --no-source-info` leaves you with.

## Calling a canister

`createActor` needs a `Transport`: two methods that move Candid bytes, nothing
more. Identity, ingress expiry, polling, certificate verification, and reject
classification all stay on the agent's side of that pipe, and the agent never
sees a schema. Here is the whole adapter for `@icp-sdk/core` v6.

<!-- Not compiled by the packaged-consumer gate: @icp-sdk/core is a type-only
peer and this repository's lockfile deliberately carries no dependency tree it
never executes. Verified out of tree against @icp-sdk/core 6.1.0 under strict
TypeScript 5.9.3 and 7.0.2 with skipLibCheck off and exactOptionalPropertyTypes
on. Re-verify it by hand when the SDK's agent surface moves. -->

```ts
import { HttpAgent } from "@icp-sdk/core/agent";
import type { CallTarget, Transport } from "@candid-core/schema/actor";

const agent = await HttpAgent.create({ host: "https://icp-api.io" });

/** `effectiveCanisterId` only matters for management-canister routing. */
function fields({ methodName, effectiveCanisterId }: CallTarget, arg: Uint8Array) {
  return effectiveCanisterId === undefined
    ? { methodName, arg }
    : { methodName, arg, effectiveTarget: { canisterId: effectiveCanisterId } };
}

export const transport: Transport = {
  async query(target, arg) {
    const response = await agent.query(target.canisterId, fields(target, arg));
    if (response.status === "rejected") {
      throw new Error(
        `${target.methodName} rejected (${response.reject_code}): ${response.reject_message}`,
      );
    }
    return response.reply.arg;
  },

  async call(target, arg) {
    // One shot: `update` submits, polls to completion, and verifies the
    // certificate before returning the certified reply bytes.
    const { reply } = await agent.update(target.canisterId, fields(target, arg));
    return reply;
  },
};
```

Then hand that transport, a service schema, and the actor interface to
`createActor`. The interface travels explicitly because `c.rec` erases method
structure from a schema's *type* — schemas carry values, not calls — so it
cannot be re-derived from `typeof`. Generated modules emit it as
`export type Actor = { … }`; written by hand it looks like this:

```ts
import { c, type Infer } from "@candid-core/schema";
import { createActor, type Transport } from "@candid-core/schema/actor";

declare const transport: Transport;

const Tokens = c.record({ e8s: c.nat64 });
const Account = c.record({ owner: c.principal, subaccount: c.opt(c.vec(c.nat8)) });

const Ledger = c.service({
  balance_of: c.func([Account], [Tokens], "query"),
  transfer: c.func([Account, Tokens], [c.nat], "update"),
});

type Ledger = {
  balance_of(account: Infer<typeof Account>): Promise<Infer<typeof Tokens>>;
  transfer(to: Infer<typeof Account>, amount: Infer<typeof Tokens>): Promise<bigint>;
};

export const ledger = createActor<Ledger>(Ledger, "ryjl3-tyaaa-aaaaa-aaaba-cai", transport);
```

A codec failure rejects the call promise with an `ActorError` carrying the
issues; transport failures propagate untouched.

## Support matrix

Measured against the published artifact with `@arethetypeswrong/cli` and
direct compiles, not inferred from the manifest.

| | |
| --- | --- |
| TypeScript | **≥ 5.0** |
| `moduleResolution` | `node16`, `nodenext`, `bundler` |
| Module format | **ESM only** — no CommonJS build ships |
| Node, from ESM | any release with ESM support |
| Node, from CommonJS `require()` | **≥ 20.19 / ≥ 22.12**, else `await import()` |
| TypeScript, from a CommonJS project | **≥ 5.8 with `"module": "nodenext"`** |

**TypeScript 5.0** is a hard floor, and it is a *parse* error below it, not a
type error: `c.tuple` is declared with a `const` type parameter — the 5.0
feature that keeps a tuple's element types from widening — so 4.9 stops at
`dist/schema.d.ts(136,11): error TS1139: Type parameter declaration expected.`
before it type-checks anything.

**`node10` resolution cannot see this package at all.** There is no top-level
`main` or `types` field, only an `exports` map, so every import fails with
`TS2307` — TypeScript's own message tells you to move to `node16`, `nodenext`,
or `bundler`.

**From CommonJS**, the two floors are independent. At runtime, `require()` of
an ES module is what Node added in 20.19 and 22.12; below those it throws
`ERR_REQUIRE_ESM`, and `await import("@candid-core/schema")` works on every
version. At the type level, a `"type": "commonjs"` project needs TypeScript
5.8 *and* `"module": "nodenext"` — `"module": "node16"` is pinned to Node 16
semantics and still refuses with `TS1479` on every compiler tested, up to and
including 7.0.

There is deliberately **no `engines` field**. The only hard floor is the
CommonJS-`require()` one above, and enforcing it in install metadata would
warn — or, under `engine-strict`, fail — for the ESM consumers it does not
apply to. It is documented here instead.

## The domain shapes (a deliberate decision)

Types describe the modern domain, not the agent-js runtime shapes: `opt T` is
`T | null` (collapsing opts fail closed at generation), variants are
`{ tag, value }` discriminated unions, anonymous `vec nat8` is `Uint8Array`,
`nat`/`int`/64-bit integers are `bigint`. Compatibility with agent-js value
shapes is an explicit non-goal, recorded on the project's issue tracker.

## Verification

Every published artifact passes a packaged-consumer gate before release: the
tarball `npm pack` produces is extracted into a clean project, compiled under
strict TypeScript with `skipLibCheck` off, and executed — root and every
subpath export, a real encode/validate round-trip. The TypeScript in this file
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
