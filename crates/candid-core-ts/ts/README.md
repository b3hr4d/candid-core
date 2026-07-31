# @candid-core/schema

A Zod-style schema runtime for [Candid], driven by [candid-core]'s canonical
Contract model: schema builders with static inference, structural validation,
a TypeScript-native Candid binary codec, typed actors over a transport-only
agent, and UI-agnostic form metadata.

```ts
import { c, type Infer } from "@candid-core/schema";
import { validate } from "@candid-core/schema/validate";
import { encode, decode } from "@candid-core/schema/codec";

const Account = c.record({ owner: c.principal, balance: c.nat });
type Account = Infer<typeof Account>; // { owner: Principal; balance: bigint }

validate(Account, value);       // { ok: true } | { ok: false, issues }
encode(Account, value);         // { ok: true, bytes } | { ok: false, issues }
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
  `Transport`; an `@icp-sdk/core` agent adapts in ~10 lines, and the agent
  never sees a schema.
- **`./forms`** — form-generation metadata: per-kind controls, constraints,
  labels, lazy recursion, and validation-issue-to-form-node resolution.
- **`./labels`** — the Candid label hash and the `_N_` rendering convention.

## The domain shapes (a deliberate decision)

Types describe the modern domain, not the agent-js runtime shapes: `opt T` is
`T | null` (collapsing opts fail closed at generation), variants are
`{ tag, value }` discriminated unions, anonymous `vec nat8` is `Uint8Array`,
`nat`/`int`/64-bit integers are `bigint`. Compatibility with agent-js value
shapes is an explicit non-goal, recorded on the project's issue tracker.

`@icp-sdk/core` is a **type-only peer dependency**: the `principal`
primitive types against its `Principal`, so the shipped declaration files
import that type and **a TypeScript consumer must install it** — without it,
compiling this package's types fails with a missing-module error naming
`@icp-sdk/core/principal`. It is marked `optional` in npm's install metadata
because nothing imports it at *runtime*: plain JavaScript consumers need
nothing, and no bytes from it are bundled. Any package providing that
subpath's types satisfies the requirement.

## Verification

Every published artifact passes a packaged-consumer gate before release: the
tarball `npm pack` produces is extracted into a clean project, compiled under
strict TypeScript with `skipLibCheck` off, and executed — root and every
subpath export, a real encode/validate round-trip. The codec is additionally
verified in both directions against the reference implementation's own wire
vectors.

## Provenance

Generated bindings, the Contract model, and the conformance gates live in the
[candid-core] repository; this package versions independently (pre-1.0), and
each release documents the generator version it pairs with.

[Candid]: https://github.com/dfinity/candid
[candid-core]: https://github.com/b3hr4d/candid-core
