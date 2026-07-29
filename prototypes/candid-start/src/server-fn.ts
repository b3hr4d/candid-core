// Server functions: the TanStack Start `createServerFn` idea made Candid.
//
// A server function here is not a hidden POST endpoint with an ad-hoc JSON
// shape — it is a first-class candid method on the canister: its input and
// output are schema-core schemas, its mode is the query/update distinction,
// and it appears in the emitted `.did` under its own name. That makes every
// server function callable three ways with one definition: directly on the
// server (SSR and other server functions), over the framework's HTTP RPC
// (the browser), and as a plain candid method (other canisters, agents,
// tooling) — the single-canister full-stack story this prototype exists to
// demonstrate.
//
// Inputs are validated before the handler runs and outputs are validated
// before they are encoded, in both transport directions. A handler never
// sees an out-of-schema value and never leaks one; both failures carry
// path-addressed issues with stable codes.
import type { Principal } from "@icp-sdk/core/principal";
import type { Infer, Schema } from "./schema.ts";
import { CandidStartError } from "./errors.ts";
import { checkMethodName } from "./did.ts";
import { checkSchema, validate, type ValidationIssue } from "./validate.ts";
import type { AnySchema } from "./walk.ts";

export interface RequestContext {
  /** The caller's principal; anonymous when unauthenticated. */
  readonly caller: Principal;
  /** Whether this execution may mutate state (update) or not (query). */
  readonly mode: "query" | "update";
}

export interface ServerFn<I, O> {
  readonly name: string;
  readonly mode: "query" | "update";
  /** `null` for a zero-argument method. */
  readonly input: Schema<I> | null;
  readonly output: Schema<O>;
  /**
   * The server-side call path: validates the input, runs the handler, and
   * validates the output. SSR loaders and other server functions call this
   * directly — the isomorphic half of the TanStack Start model.
   */
  invoke(input: I, ctx: RequestContext): Promise<O>;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnyServerFn = ServerFn<any, any>;

/** Thrown when a value fails schema validation at an invoke boundary. */
export class InvokeValidationError extends CandidStartError {
  readonly issues: readonly ValidationIssue[];

  constructor(code: string, issues: readonly ValidationIssue[]) {
    super(
      code,
      issues
        .map((issue) => `${issue.code} at ${issue.path}: ${issue.message}`)
        .join("; "),
    );
    this.issues = issues;
  }
}

interface WithInputInit<I extends AnySchema, O extends AnySchema> {
  readonly name: string;
  readonly input: I;
  readonly output: O;
  readonly handler: (
    input: Infer<I>,
    ctx: RequestContext,
  ) => Infer<O> | Promise<Infer<O>>;
}

interface NoInputInit<O extends AnySchema> {
  readonly name: string;
  readonly input?: undefined;
  readonly output: O;
  readonly handler: (ctx: RequestContext) => Infer<O> | Promise<Infer<O>>;
}

/** Define a query server function: read-only, callable in query context. */
export function query<O extends AnySchema>(
  init: NoInputInit<O>,
): ServerFn<void, Infer<O>>;
export function query<I extends AnySchema, O extends AnySchema>(
  init: WithInputInit<I, O>,
): ServerFn<Infer<I>, Infer<O>>;
export function query(
  init: NoInputInit<AnySchema> | WithInputInit<AnySchema, AnySchema>,
): AnyServerFn {
  return defineServerFn("query", init);
}

/** Define an update server function: may mutate canister state. */
export function update<O extends AnySchema>(
  init: NoInputInit<O>,
): ServerFn<void, Infer<O>>;
export function update<I extends AnySchema, O extends AnySchema>(
  init: WithInputInit<I, O>,
): ServerFn<Infer<I>, Infer<O>>;
export function update(
  init: NoInputInit<AnySchema> | WithInputInit<AnySchema, AnySchema>,
): AnyServerFn {
  return defineServerFn("update", init);
}

function defineServerFn(
  mode: "query" | "update",
  init: NoInputInit<AnySchema> | WithInputInit<AnySchema, AnySchema>,
): AnyServerFn {
  checkMethodName(init.name);
  if (init.name.startsWith("__")) {
    throw new CandidStartError(
      "server_fn_reserved_name",
      `names starting with "__" are reserved for the framework`,
    );
  }
  const input = init.input ?? null;
  for (const [role, schema] of [
    ["input", input],
    ["output", init.output],
  ] as const) {
    if (schema === null) {
      continue;
    }
    const issues = checkSchema(schema);
    if (issues.length > 0) {
      throw new CandidStartError(
        "server_fn_invalid_schema",
        `${init.name} ${role} schema: ` +
          issues.map((issue) => `${issue.code} at ${issue.path}`).join("; "),
      );
    }
  }

  const invoke = async (value: unknown, ctx: RequestContext): Promise<unknown> => {
    if (mode === "update" && ctx.mode === "query") {
      throw new CandidStartError(
        "server_fn_requires_update",
        `${init.name} is an update function and cannot run in query context`,
      );
    }
    let result: unknown;
    if (input === null) {
      if (value !== undefined) {
        throw new InvokeValidationError("invalid_argument", [
          {
            code: "unexpected_argument",
            path: "$",
            message: `${init.name} takes no argument`,
          },
        ]);
      }
      result = await (init as NoInputInit<AnySchema>).handler(ctx);
    } else {
      const checked = validate(input, value);
      if (!checked.ok) {
        throw new InvokeValidationError("invalid_argument", checked.issues);
      }
      result = await (init as WithInputInit<AnySchema, AnySchema>).handler(
        value,
        ctx,
      );
    }
    const checkedOut = validate(init.output, result);
    if (!checkedOut.ok) {
      // The handler produced an out-of-schema value; failing closed here is
      // what keeps the advertised `.did` honest.
      throw new InvokeValidationError("invalid_result", checkedOut.issues);
    }
    return result;
  };

  return {
    name: init.name,
    mode,
    input,
    output: init.output,
    invoke,
  } as AnyServerFn;
}
