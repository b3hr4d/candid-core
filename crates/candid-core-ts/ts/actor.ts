// The typed actor surface over a transport-only agent — issue #104. An actor
// is one async method per service method; every argument and reply crosses
// the boundary as Candid bytes produced and consumed by the TS-native codec,
// so the transport never sees a schema.
//
// # The transport is a two-method byte pipe (decision recorded on #104)
//
// `query` is the non-replicated read path (`query` and `composite_query`
// methods); `call` is the replicated certified path (`update` — and
// `oneway`, which is dispatched and awaited exactly like an update: the
// reference agent has no separate oneway path, and pretending otherwise here
// would invent semantics the platform does not offer). Everything an agent
// does beyond moving bytes — identity, ingress expiry, retries, polling,
// certificate verification, reject classification — stays on the agent's
// side of the pipe.
//
// The complete `@icp-sdk/core` v6 adapter lives in README.md, under "Calling
// a canister" — issue #148. A sketch used to sit here instead, and being
// here was its problem: a `//` comment never reaches declaration emit, so it
// was invisible in `.d.ts` hover while shipping inside `dist/actor.js`, where
// no consumer looks. It also destructured `{ canisterId, methodName }` and
// dropped `effectiveCanisterId` on the floor, so the one call shape that
// needs the field — management-canister routing, which is the only reason
// `CallTarget` carries it — would have routed to the wrong effective target.
//
// # Errors
//
// The codec and validator never throw; the actor layer is where failures
// become rejected promises, because that is what an async method signature
// means. A codec failure rejects with [`ActorError`] carrying the issues;
// transport failures propagate untouched.
//
// # Typing
//
// `createActor<A>` takes the actor interface as an explicit type parameter —
// the generated modules emit it (`export type Actor = { … }`), and the
// invariance gate on the goldens is what proves the emitted interface
// matches the schema the factory walks. `c.rec` erases method structure
// from the schema's *type* (by design: schemas carry values, not calls), so
// the interface cannot be re-derived from `typeof` — it travels explicitly.

import type { AnyFieldSchema, AnySchema, FuncValue, Schema } from "./schema.ts";
import { encodeArgs, decodeArgs, type CodecIssue } from "./codec.ts";
import { validate } from "./validate.ts";

/** Where a call goes; `effectiveCanisterId` only matters for system routing. */
export interface CallTarget {
  readonly canisterId: string;
  readonly methodName: string;
  readonly effectiveCanisterId?: string;
}

/** Candid-encoded bytes in, Candid-encoded reply bytes out. */
export interface Transport {
  /** Non-replicated read path: `query` and `composite_query` methods. */
  query(target: CallTarget, arg: Uint8Array): Promise<Uint8Array>;
  /** Replicated certified path: `update` and `oneway` methods. */
  call(target: CallTarget, arg: Uint8Array): Promise<Uint8Array>;
}

/** A codec failure surfaced through an actor call. */
export class ActorError extends Error {
  readonly issues: readonly CodecIssue[];

  constructor(stage: "encode" | "decode", method: string, issues: readonly CodecIssue[]) {
    super(
      `${stage} failed for method ${JSON.stringify(method)}: ${
        issues[0]?.message ?? "unknown issue"
      } (${issues[0]?.path ?? "$"})`,
    );
    this.issues = issues;
  }
}

interface ResolvedFunc {
  readonly args: readonly AnyFieldSchema[];
  readonly results: readonly AnyFieldSchema[];
  readonly mode: "update" | "query" | "composite_query" | "oneway";
}

/** Resolve rec chains structurally; throws on a non-schema (programmer error). */
function resolveNode(schema: AnyFieldSchema): { kind: string } & Record<string, unknown> {
  let node: unknown = schema;
  for (let hops = 0; hops < 256; hops += 1) {
    if (
      typeof node !== "object" ||
      node === null ||
      typeof (node as { kind?: unknown }).kind !== "string"
    ) {
      throw new TypeError("not a schema object");
    }
    if ((node as { kind: string }).kind !== "rec") {
      return node as { kind: string } & Record<string, unknown>;
    }
    node = (node as { body(): unknown }).body();
  }
  throw new TypeError("rec chain exceeds the depth limit");
}

function resolveFunc(schema: AnyFieldSchema, what: string): ResolvedFunc {
  const node = resolveNode(schema);
  if (node.kind !== "func") {
    throw new TypeError(`${what} is not a func schema`);
  }
  return node as unknown as ResolvedFunc;
}

/**
 * The reply convention, mirroring the ecosystem's: zero results resolve to
 * `undefined`, one result to the value, several to a tuple array.
 */
function collapseResults(values: readonly unknown[]): unknown {
  if (values.length === 0) {
    return undefined;
  }
  if (values.length === 1) {
    return values[0];
  }
  return values;
}

async function dispatch(
  func: ResolvedFunc,
  target: CallTarget,
  transport: Transport,
  args: readonly unknown[],
): Promise<unknown> {
  const encoded = encodeArgs(func.args, args);
  if (!encoded.ok) {
    throw new ActorError("encode", target.methodName, encoded.issues);
  }
  const viaQuery = func.mode === "query" || func.mode === "composite_query";
  const reply = viaQuery
    ? await transport.query(target, encoded.bytes)
    : await transport.call(target, encoded.bytes);
  const decoded = decodeArgs(func.results, reply);
  if (!decoded.ok) {
    throw new ActorError("decode", target.methodName, decoded.issues);
  }
  return collapseResults(decoded.values);
}

/** Per-actor call settings, applied to every method it dispatches. */
export interface ActorOptions {
  readonly effectiveCanisterId?: string;
}

/**
 * A typed call object over a service schema: one async method per service
 * method, mode-dispatched onto the transport. `A` is the generated actor
 * interface (or a hand-written one); the schema is the runtime truth the
 * methods are built from, and the golden gates are what keep the two equal.
 *
 * Throws `TypeError` immediately on a schema that is not a service — a
 * programmer error, unlike the data errors that reject call promises.
 */
export function createActor<A = Record<string, (...args: never[]) => Promise<unknown>>>(
  service: AnyFieldSchema,
  canisterId: string,
  transport: Transport,
  options: ActorOptions = {},
): A {
  const node = resolveNode(service);
  if (node.kind !== "service") {
    throw new TypeError("createActor needs a service schema");
  }
  const methods = node.methods as { readonly [name: string]: AnySchema };
  const actor: Record<string, unknown> = Object.create(null);
  for (const name of Object.keys(methods)) {
    const func = resolveFunc(methods[name], `method ${JSON.stringify(name)}`);
    const target: CallTarget = {
      canisterId,
      methodName: name,
      ...(options.effectiveCanisterId === undefined
        ? {}
        : { effectiveCanisterId: options.effectiveCanisterId }),
    };
    actor[name] = (...args: unknown[]) => dispatch(func, target, transport, args);
  }
  return actor as A;
}

/**
 * Invoke a func *value* — the `{ principal, method }` reference a message
 * carried — against its own service. This is how the ledger's archived
 * transaction callbacks are paged: decode the response, then call the
 * reference it contains.
 */
export function callFunc(
  schema: AnyFieldSchema,
  value: FuncValue,
  args: readonly unknown[],
  transport: Transport,
): Promise<unknown> {
  const func = resolveFunc(schema, "the func value's schema");
  // The reference is data from a decoded message; guard it like every other
  // data path rather than letting a malformed value pick the call target.
  const shape = validate(schema as Schema<unknown>, value);
  if (!shape.ok) {
    return Promise.reject(
      new ActorError(
        "encode",
        typeof value?.method === "string" ? value.method : "<func value>",
        shape.issues.map((issue) => ({ ...issue })),
      ),
    );
  }
  const target: CallTarget = {
    canisterId: value.principal.toText(),
    methodName: value.method,
  };
  return dispatch(func, target, transport, args);
}
