// Routes: typed paths, server loaders, and components.
//
// The TanStack Start mapping: a route's `loader` runs inside the canister —
// during SSR for a document request, or as an auto-registered *query server
// function* when a client navigates. The `data` schema is the candid twist:
// loader results are schema-validated domain values, so the hydration
// payload and the loader RPC both move through the same validated wire
// codec as every other server function, and the loader appears in the
// canister's `.did` as an honest query method.
//
// Path params are typed from the literal: `"/notes/:id"` gives
// `{ id: string }` at compile time with no code generation.
import type { Schema } from "./schema.ts";
import type { VNode } from "./html.ts";
import type { RequestContext } from "./server-fn.ts";
import { CandidStartError } from "./errors.ts";
import { checkSchema } from "./validate.ts";
import { c } from "./schema.ts";

type Segments<P extends string> = P extends `${infer Head}/${infer Tail}`
  ? Head | Segments<Tail>
  : P;

type ParamName<S extends string> = S extends `:${infer Name}` ? Name : never;

/** `"/notes/:id"` → `{ id: string }`; a paramless path gives `{}`. */
export type RouteParams<P extends string> = {
  [K in ParamName<Segments<P>>]: string;
};

export interface RouteConfig<P extends string, D> {
  /** Candid-identifier-shaped; becomes the `<name>_loader` method name. */
  readonly name: string;
  readonly path: P;
  /** The loader result schema — the route's validated data contract. */
  readonly data: Schema<D>;
  readonly loader: (args: {
    readonly params: RouteParams<P>;
    readonly ctx: RequestContext;
  }) => D | Promise<D>;
  readonly component: (props: {
    readonly params: RouteParams<P>;
    readonly data: D;
  }) => VNode;
}

export interface Route<P extends string = string, D = unknown> {
  readonly name: string;
  readonly path: P;
  readonly segments: readonly RouteSegment[];
  readonly staticCount: number;
  readonly paramNames: readonly string[];
  readonly data: Schema<D>;
  readonly loader: (args: {
    readonly params: RouteParams<P>;
    readonly ctx: RequestContext;
  }) => D | Promise<D>;
  readonly component: (props: {
    readonly params: RouteParams<P>;
    readonly data: D;
  }) => VNode;
}

// The variance wildcard, for heterogeneous route lists; see walk.ts.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnyRoute = Route<string, any>;

export type RouteSegment =
  | { readonly kind: "static"; readonly text: string }
  | { readonly kind: "param"; readonly name: string };

const ROUTE_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const STATIC_SEGMENT = /^[A-Za-z0-9._~-]+$/;
const PARAM_SEGMENT = /^:[A-Za-z_][A-Za-z0-9_]*$/;

export function defineRoute<P extends string, D>(
  config: RouteConfig<P, D>,
): Route<P, D> {
  if (!ROUTE_NAME.test(config.name)) {
    throw new CandidStartError(
      "route_invalid_name",
      `route name ${JSON.stringify(config.name)} is not identifier-shaped`,
    );
  }
  const segments = parsePath(config.path);
  const paramNames: string[] = [];
  for (const segment of segments) {
    if (segment.kind === "param") {
      if (paramNames.includes(segment.name)) {
        throw new CandidStartError(
          "route_duplicate_param",
          `path ${JSON.stringify(config.path)} repeats :${segment.name}`,
        );
      }
      paramNames.push(segment.name);
    }
  }
  const schemaIssues = checkSchema(config.data);
  if (schemaIssues.length > 0) {
    throw new CandidStartError(
      "route_invalid_schema",
      `route ${JSON.stringify(config.name)} data schema: ` +
        schemaIssues.map((issue) => `${issue.code} at ${issue.path}`).join("; "),
    );
  }
  return {
    name: config.name,
    path: config.path,
    segments,
    staticCount: segments.filter((segment) => segment.kind === "static").length,
    paramNames,
    data: config.data,
    loader: config.loader,
    component: config.component,
  };
}

function parsePath(path: string): RouteSegment[] {
  if (path === "/") {
    return [];
  }
  if (!path.startsWith("/") || path.endsWith("/")) {
    throw new CandidStartError(
      "route_invalid_path",
      `path ${JSON.stringify(path)} must start with "/" and not end with one`,
    );
  }
  return path
    .slice(1)
    .split("/")
    .map((raw): RouteSegment => {
      if (PARAM_SEGMENT.test(raw)) {
        return { kind: "param", name: raw.slice(1) };
      }
      if (STATIC_SEGMENT.test(raw)) {
        return { kind: "static", text: raw };
      }
      throw new CandidStartError(
        "route_invalid_path",
        `path segment ${JSON.stringify(raw)} is neither static nor :param`,
      );
    });
}

export interface RouteMatch {
  readonly route: AnyRoute;
  readonly params: Record<string, string>;
}

/**
 * Match a request pathname. Specificity: more static segments win; ties go
 * to registration order. A pathname that fails percent-decoding matches
 * nothing rather than throwing.
 */
export function matchRoute(
  routes: readonly AnyRoute[],
  pathname: string,
): RouteMatch | null {
  const requestSegments = splitPathname(pathname);
  if (requestSegments === null) {
    return null;
  }
  let best: RouteMatch | null = null;
  let bestStatic = -1;
  for (const route of routes) {
    const params = matchSegments(route.segments, requestSegments);
    if (params !== null && route.staticCount > bestStatic) {
      best = { route, params };
      bestStatic = route.staticCount;
    }
  }
  return best;
}

function splitPathname(pathname: string): string[] | null {
  if (!pathname.startsWith("/")) {
    return null;
  }
  const trimmed =
    pathname.length > 1 && pathname.endsWith("/")
      ? pathname.slice(0, -1)
      : pathname;
  if (trimmed === "/") {
    return [];
  }
  const out: string[] = [];
  for (const raw of trimmed.slice(1).split("/")) {
    if (raw === "") {
      return null;
    }
    try {
      out.push(decodeURIComponent(raw));
    } catch {
      return null;
    }
  }
  return out;
}

function matchSegments(
  pattern: readonly RouteSegment[],
  request: readonly string[],
): Record<string, string> | null {
  if (pattern.length !== request.length) {
    return null;
  }
  const params: Record<string, string> = {};
  for (let index = 0; index < pattern.length; index += 1) {
    const expected = pattern[index];
    const actual = request[index];
    if (expected === undefined || actual === undefined) {
      return null;
    }
    if (expected.kind === "static") {
      if (expected.text !== actual) {
        return null;
      }
    } else {
      params[expected.name] = actual;
    }
  }
  return params;
}

/**
 * The loader's candid argument schema: a record of text params in sorted
 * order, or `null` (a zero-argument method) for a paramless route. This is
 * what makes a loader an honest `.did` query method.
 */
export function loaderInputSchema(
  route: AnyRoute,
): Schema<Record<string, string>> | null {
  if (route.paramNames.length === 0) {
    return null;
  }
  const fields: Record<string, Schema<string>> = {};
  for (const name of [...route.paramNames].sort()) {
    fields[name] = c.text;
  }
  return c.record(fields) as unknown as Schema<Record<string, string>>;
}
