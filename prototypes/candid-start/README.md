# candid-start

A **prototype** full-stack TypeScript framework in the TanStack Start mold —
file-of-routes, server functions, server-side rendering, typed client calls —
where the entire application lives in **one Internet Computer canister**, and
every type that crosses a boundary is a candid-core schema.

**Status: prototype, deliberately unpublishable.** `candid-start` is a working
name; npm names are as permanent as crates.io names, and naming is gated on
the issue #106 decision, exactly like the schema core it builds on. The
package is `private: true`, ships nothing, and claims nothing measured — no
performance numbers appear anywhere in it, per the repository's
measured-not-estimated rule.

## The idea

TanStack Start splits an app between a server runtime and a browser bundle,
with server functions as typed RPC between them. A canister *is* both halves:
it serves HTTP (`http_request` for reads, `http_request_update` for writes)
and it exposes typed candid methods. So the mapping is direct — and one thing
becomes possible that has no web-platform equivalent: **the framework derives
the canister's public Candid interface from the application code**, and
candid-core turns that interface into a canonical, identity-addressed
Contract.

| TanStack Start | candid-start |
| --- | --- |
| `createServerFn` | `query({...})` / `update({...})` — a real candid method with schema-validated input and output |
| Route + `loader` | `defineRoute` — the loader is auto-registered as a `<name>_loader` query method |
| SSR document request | `http_request` (query): match route → run loader → render → HTML + hydration payload |
| Mutating server call | `http_request_update`, reached through the IC gateway's upgrade dance |
| Typed client RPC | wire codec + `/__rpc/<method>` — or a plain candid call from any agent or canister |
| `zod` validators | the issue #38 schema core: `c.*` builders generated from `.did` by candid-core-ts |
| Type inference | `Infer<typeof Schema>`, plus `RouteParams<"/notes/:id">` → `{ id: string }` |

## The loop with candid-core

The prototype closes a loop no web framework has:

```
notes.did ──candid-core──► canonical Contract ──candid-core-ts──► schemas.ts
                                                                     │
             app code: server fns + routes typed by those schemas ◄──┘
                                    │
                            createApp(...)
                                    │
                    emitted service .did  (didText())
                                    │
                     candid-core compile ──► interface_id
```

The final `interface_id` is a semantic identity over the canonical type graph
— declaration names excluded — so the emitter can spell types inline and
still land on the same identity as any named spelling. The integration suite
proves that with the real compiler: an inline-emitted interface and a
hand-named equivalent produce identical `interface_id`s. The practical
consequence is **deploy-time drift detection**: compare the `interface_id` of
the build you are about to deploy with the one running, and *any* change to
the wire interface — breaking or not — shows up as a string inequality.
Whether a flagged change is actually breaking is a separate judgment the id
does not make; it tells you the interface moved, not how.

## What runs today

Everything below is exercised by the test suites (76 unit tests, 3
cargo-backed integration tests) and by `npm run dev`:

- **SSR**: GET `/` and `/notes/:id` render through route loaders into
  escape-safe HTML with a hydration payload in a JSON `<script>` (with
  `<`, `>`, `&`, U+2028/9 escaped inside the JSON string domain).
- **Server functions**: `list_notes`/`get_note` (query),
  `add_note`/`publish_note` (update) — validated in both directions, exposed
  three ways: direct server-side call, HTTP RPC, candid dispatch.
- **The upgrade dance**: an update RPC on the query path answers
  `upgrade: true`; the dev server (playing the IC gateway) re-dispatches
  through `httpRequestUpdate`, exactly as mainnet would.
- **Interface derivation**: `didText()` emits the full service; the
  integration test compiles it with the `candid-core` CLI and pins the
  identity properties.
- **Generated schemas**: `example/notes/schemas.ts` is real
  `candid-core-ts` output (via the crate's dev-only `generate` example);
  the integration suite regenerates and byte-compares it, so generated code
  cannot drift from `notes.did` unnoticed.

```sh
npm ci
npm run check              # tsc --strict, both surfaces (server + DOM client)
npm test                   # unit suites, no cargo needed
npm run test:integration   # regeneration freshness + CLI compile + identity
npm run dev                # http://127.0.0.1:3000/
npm run generate           # notes.did → schemas.ts through the Rust generator
```

Node ≥ 22.18: the framework runs as native TypeScript under Node's type
stripping (`erasableSyntaxOnly` is on, so the compiler proves strippability),
with zero runtime dependencies.

## Module tour

```
src/
  schema.ts      the one file that spells the schema-core path (re-export)
  walk.ts        structural view over schema objects; the one coupling point
  validate.ts    bounded, path-addressed validation + construction checks
  wire.ts        schema-driven JSON transport codec (interim until #103)
  did.ts         schemas → deterministic candid service text
  server-fn.ts   query()/update() — server functions as candid methods
  router.ts      typed paths, loaders, matching
  html.ts        h()/renderToString, escape-safe by construction
  canister.ts    createApp: http_request / http_request_update / candid
  dev-server.ts  node:http gateway simulation (upgrade dance included)
  client.ts      browser rpc client + hydrate() sketch (type-checked, DOM)
example/notes/   notes.did → schemas.ts → app.ts → serve.ts
```

## Standing on the #38 program (and staying out of its way)

The umbrella program's recorded decisions are consumed, not relitigated:
modern domain shapes (`T | null` opts failing closed on `opt opt`/`opt
null`/`opt reserved`, `{ tag, value }` variants, `Uint8Array`, `bigint`),
label `_id_` convention, exact pinning, the invariant `Schema<in out T>`
equality gate. Three pieces of this prototype are deliberate stand-ins for
open slices, and the slice wins on any divergence:

- `validate.ts` previews **#102**'s `validate` half (same constraints:
  bigint enforcement, range checks, bounded depth/work, canonical issue
  order). It does not implement `schemaFromContract`, and #102 remains open
  and unowned by this prototype.
- `wire.ts` is the transport until **#103**'s conformance-gated TS-native
  Candid binary codec; only `toWire`/`fromWire` call sites change when it
  lands. Known interim limits: float NaN payload bits are not preserved,
  and `reserved` round-trips as `null`.
- The `candid` dispatch surface and emitted method specs preview **#104**'s
  typed-actor shape over a transport-only agent.
- The npm story is **#106**'s to decide; nothing here publishes.

One repository-visible change rides along: the schema core's harness
`package.json` now declares `"type": "module"`, which is what the core
already is — required for Node to execute it directly, invisible to the
harness's own `npm ci && npx tsc --noEmit` gate.

## Fail-closed inventory

The prototype inherits the repository's posture — refuse rather than guess:

- unknown schema kinds and primitives — every layer fails closed, under a
  layer-specific stable code (`unsupported_schema` in validation/checkSchema,
  `wire_internal` in the codec, `did_unsupported_schema` in emission)
- collapsing options at three layers: schema check, validation, emission
- unrepresentable labels: control characters, lone UTF-16 surrogates,
  out-of-u32 numeric labels, and `true`/`false` (Candid Boolean tokens, which
  candid_parser rejects as a label in every spelling) — refused as both
  labels and method names
- `rec` chains deeper than the codec resolves (`MAX_REC_DEPTH`) refused at
  construction, so no schema passes every gate and then fails on every call
- non-candid method and route names; `__`-prefixed names reserved
- variant tags and record fields treated only as own properties — reads via
  `ownEntry`, writes via a `__proto__`-safe setter (no prototype-chain hits)
- handler output that breaks the advertised schema → 500 `invalid_result`,
  details to the error observer, never to the wire; the RPC client re-runs
  `validate` after decode, so a misbehaving peer cannot inject out-of-domain
  values into typed client code
- RPC body size bound before parsing (413), wire depth bound, bigint decode
  digit cap, validation depth/work/issue bounds, render depth bound (array
  children included)
- strict base64 (canonical padding and trailing bits)
- `renderToString`: no raw-HTML API, attribute/tag name validation, URL
  scheme refusal for `javascript:`/`vbscript:`/non-image `data:`
  (control-character smuggling included), all `on*` event-handler props
  skipped (string or function), void-element child refusal

## Open decisions (maintainer)

1. **Component model.** The built-in `h()`/`renderToString` is enough to
   prove the architecture; React/Solid adapters (and streaming SSR) are the
   real product question.
2. **Client runtime and bundling.** `client.ts` type-checks but nothing
   ships it to a browser; the prototype's demo navigates MPA-style. A
   bundler choice is a supply-chain decision in this repository's terms.
3. **Certification.** SSR responses from `http_request` are uncertified
   query results. Response certification (asset certification v2 style) is
   unaddressed and would shape the rendering pipeline.
4. **Host binding.** `createApp` returns plain functions on gateway-shaped
   requests; wiring them into a real canister runtime (Azle-style JS, or a
   dedicated runtime) is the deployment story.
5. **Loader argument model.** Loaders currently take path params only
   (`record { <param> : text }`); search params and typed param parsing
   (e.g. `nat64` ids at the route boundary) are open.
6. **Naming**, gated on #106.

## Known limitations beyond the open decisions

- The runtime `Principal` is a text-wrapping stand-in: no base32/CRC
  validation until the agent dependency arrives with #104.
- HTTP callers are anonymous; there is no authentication/identity story on
  the RPC path yet (direct candid dispatch accepts a caller for tests).
- Non-recursive named types are inlined at every use site in the emitted
  `.did`; pathological schema DAGs could grow the text exponentially
  (semantic identity is unaffected; the emitter, not the identity, would
  need sharing).
- The example's clock is deterministic fake time; a host binding supplies
  `ic0.time`.
- `errors.ts` codes are stable strings, but the error *messages* are not
  stable surface, matching candid-core's diagnostics discipline.
- **`empty` in composite positions is not ergonomic.** `Schema<never>`
  (`c.empty`) sits outside the `Schema<any>` variance wildcard the framework
  uses, so a bare `empty` arm/field/element/output needs an `as` cast to
  typecheck, and a bare `empty` variant arm is inference-tag-only yet
  runtime-uninhabited (the runtime is deliberately stricter). Widening this
  belongs to the shared, golden-gated schema core, not the prototype. A
  generated schema using bare `empty` inside a composite is therefore out of
  the prototype's ergonomic scope, though `opt empty` (domain `null`) works.
- The interim JSON wire codec has documented seams that the #103 binary codec
  removes: `-0.0` round-trips as `+0.0` (JSON serializes both as `0`), NaN
  payload bits are not preserved, `reserved` carries nothing, and two schemas
  with the same `interface_id` but different structural spelling (`blob` vs
  `vec nat8`, `unit` vs empty tuple) encode differently — a value must be
  decoded under the same schema it was encoded with.
- The Node version is floored (`>=22.18.0`), not pinned: the prototype has no
  CI job of its own, so there is no workflow-side toolchain pin as the schema
  core's harness has. Pinning is a decision for whenever this graduates to CI.
