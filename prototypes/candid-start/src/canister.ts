// The canister facade: one app object exposing the three surfaces a single
// canister serves.
//
//   http_request         (query)   GET  → SSR'd route documents
//                                  POST → RPC for query fns; update fns
//                                         answer with the IC upgrade flag
//   http_request_update  (update)  POST → RPC in update context
//   candid methods                 every server fn and every route loader,
//                                  under its own name, exactly as the
//                                  emitted `.did` advertises
//
// The shapes mirror the IC HTTP gateway interface (method, url, header
// pairs, byte body, and the `upgrade` flag that re-dispatches a query-path
// POST through consensus) so a host binding maps this one-to-one; the dev
// server plays the gateway for local work. Candid *transport* is the
// framework's interim JSON wire (see wire.ts) until the #103 codec lands —
// dispatch shape and validation boundaries do not change when it does.
import type { Principal } from "@icp-sdk/core/principal";
import { CandidStartError } from "./errors.ts";
import { emitServiceDid, type DidMethodSpec } from "./did.ts";
import {
  fragment,
  h,
  jsonScript,
  renderDocument,
  type VNode,
} from "./html.ts";
import { anonymousPrincipal } from "./principal.ts";
import {
  loaderInputSchema,
  matchRoute,
  type AnyRoute,
  type RouteParams,
} from "./router.ts";
import {
  InvokeValidationError,
  query,
  type AnyServerFn,
  type RequestContext,
} from "./server-fn.ts";
import { fromWire, toWire, type WireValue } from "./wire.ts";
import type { ValidationIssue } from "./validate.ts";
import type { AnySchema } from "./walk.ts";

export interface HttpRequest {
  readonly method: string;
  /** Path plus optional query string, as the gateway delivers it. */
  readonly url: string;
  readonly headers: ReadonlyArray<readonly [string, string]>;
  readonly body: Uint8Array;
}

export interface HttpResponse {
  readonly status: number;
  readonly headers: ReadonlyArray<readonly [string, string]>;
  readonly body: Uint8Array;
  /** IC pattern: a query-path response asking to be re-run as an update. */
  readonly upgrade?: boolean;
}

export interface ShellProps {
  /** The rendered route component. */
  readonly body: VNode;
  /** Framework-provided nodes (hydration payload); place inside `<body>`. */
  readonly scripts: VNode;
}

export interface AppConfig {
  readonly routes: readonly AnyRoute[];
  readonly serverFns: readonly AnyServerFn[];
  /** Returns the full `<html>` document node for a rendered route. */
  readonly shell: (props: ShellProps) => VNode;
  /** Bound on RPC request bodies before any parsing; fail closed. */
  readonly maxRequestBytes?: number;
  /** Observer for handler/render failures; the default is silence. */
  readonly onError?: (error: unknown) => void;
}

export interface CandidSurface {
  /** Method specs in emission order — the #104 typed-actor design preview. */
  methods(): readonly DidMethodSpec[];
  /**
   * Dispatch one candid method through the same validation pipeline as the
   * HTTP surface, on wire-encoded values. `arg` must be `undefined` for a
   * zero-argument method.
   */
  invoke(
    name: string,
    arg: WireValue | undefined,
    opts?: { readonly caller?: Principal },
  ): Promise<WireValue>;
}

export interface App {
  httpRequest(req: HttpRequest): Promise<HttpResponse>;
  httpRequestUpdate(req: HttpRequest): Promise<HttpResponse>;
  didText(): string;
  readonly candid: CandidSurface;
  readonly routes: readonly AnyRoute[];
}

export const RPC_PREFIX = "/__rpc/";
export const HYDRATION_SCRIPT_ID = "__candid_start_data__";
const DEFAULT_MAX_REQUEST_BYTES = 2 * 1024 * 1024;

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: false });

export function createApp(config: AppConfig): App {
  const maxRequestBytes = config.maxRequestBytes ?? DEFAULT_MAX_REQUEST_BYTES;
  const onError = config.onError ?? (() => {});

  // Loaders become real query server functions, so SSR, client navigation,
  // and direct candid calls all run the identical validated path.
  const methods = new Map<string, AnyServerFn>();
  const loaderNames = new Map<string, string>();

  const register = (fn: AnyServerFn): void => {
    if (methods.has(fn.name)) {
      throw new CandidStartError(
        "app_duplicate_method",
        `method name ${JSON.stringify(fn.name)} is registered twice`,
      );
    }
    methods.set(fn.name, fn);
  };

  for (const fn of config.serverFns) {
    register(fn);
  }

  const routeNames = new Set<string>();
  for (const route of config.routes) {
    if (routeNames.has(route.name)) {
      throw new CandidStartError(
        "app_duplicate_route",
        `route name ${JSON.stringify(route.name)} is registered twice`,
      );
    }
    routeNames.add(route.name);
    const loaderName = `${route.name}_loader`;
    const input = loaderInputSchema(route);
    const loaderFn =
      input === null
        ? query({
            name: loaderName,
            output: route.data as AnySchema,
            handler: (ctx) =>
              route.loader({ params: {} as RouteParams<string>, ctx }),
          })
        : query({
            name: loaderName,
            input: input as AnySchema,
            output: route.data as AnySchema,
            handler: (params, ctx) =>
              route.loader({ params: params as RouteParams<string>, ctx }),
          });
    register(loaderFn);
    loaderNames.set(route.name, loaderName);
  }

  const methodSpecs = (): readonly DidMethodSpec[] =>
    [...methods.values()].map((fn) => ({
      name: fn.name,
      mode: fn.mode,
      input: fn.input,
      output: fn.output,
    }));

  // Emitting at construction proves every registered schema is expressible;
  // a schema the emitter refuses fails the app at startup, not a request.
  const didDocument = emitServiceDid(methodSpecs());

  const invokeWire = async (
    fn: AnyServerFn,
    arg: WireValue | undefined,
    ctx: RequestContext,
  ): Promise<WireValue> => {
    let domainInput: unknown;
    if (fn.input === null) {
      if (arg !== undefined) {
        throw new InvokeValidationError("invalid_argument", [
          {
            code: "unexpected_argument",
            path: "$",
            message: `${fn.name} takes no argument`,
          },
        ]);
      }
      domainInput = undefined;
    } else {
      const decoded = fromWire(fn.input, arg);
      if (!decoded.ok) {
        throw new InvokeValidationError("invalid_argument", decoded.issues);
      }
      domainInput = decoded.value;
    }
    const result = await fn.invoke(domainInput, ctx);
    return toWire(fn.output, result);
  };

  const rpcResponse = (
    status: number,
    payload: Record<string, unknown>,
  ): HttpResponse => ({
    status,
    headers: [["content-type", "application/json; charset=utf-8"]],
    body: encoder.encode(JSON.stringify(payload)),
  });

  const rpcFailure = (
    status: number,
    code: string,
    issues?: readonly ValidationIssue[],
  ): HttpResponse =>
    rpcResponse(status, {
      ok: false,
      code,
      ...(issues === undefined ? {} : { issues }),
    });

  const htmlResponse = (status: number, html: string): HttpResponse => ({
    status,
    headers: [["content-type", "text/html; charset=utf-8"]],
    body: encoder.encode(html),
  });

  const notFoundPage = (): HttpResponse =>
    htmlResponse(
      404,
      renderDocument(
        h(
          "html",
          null,
          h("head", null, h("title", null, "Not found")),
          h("body", null, h("h1", null, "404"), h("p", null, "No such route.")),
        ),
      ),
    );

  const errorPage = (): HttpResponse =>
    htmlResponse(
      500,
      renderDocument(
        h(
          "html",
          null,
          h("head", null, h("title", null, "Error")),
          h(
            "body",
            null,
            h("h1", null, "500"),
            h("p", null, "The canister failed to render this route."),
          ),
        ),
      ),
    );

  const executeRpc = async (
    name: string,
    req: HttpRequest,
    mode: "query" | "update",
  ): Promise<HttpResponse> => {
    const fn = methods.get(name);
    if (fn === undefined) {
      return rpcFailure(404, "unknown_method");
    }
    if (mode === "query" && fn.mode === "update") {
      // The IC pattern: the query path cannot mutate, so the response asks
      // the gateway to re-run the request through consensus.
      return { ...rpcResponse(200, { ok: false, code: "upgrade" }), upgrade: true };
    }
    if (req.body.byteLength > maxRequestBytes) {
      return rpcFailure(413, "request_too_large");
    }
    let envelope: unknown;
    try {
      envelope = JSON.parse(decoder.decode(req.body));
    } catch {
      return rpcFailure(400, "rpc_malformed_json");
    }
    if (
      typeof envelope !== "object" ||
      envelope === null ||
      Array.isArray(envelope)
    ) {
      return rpcFailure(400, "rpc_malformed_envelope");
    }
    const arg = Object.prototype.hasOwnProperty.call(envelope, "arg")
      ? ((envelope as Record<string, unknown>)["arg"] as WireValue)
      : undefined;
    try {
      const value = await invokeWire(fn, arg, {
        caller: anonymousPrincipal,
        mode,
      });
      return rpcResponse(200, { ok: true, value });
    } catch (error) {
      if (error instanceof InvokeValidationError) {
        if (error.code === "invalid_argument") {
          return rpcFailure(400, error.code, error.issues);
        }
        // invalid_result: the handler broke the advertised contract. The
        // issues describe server internals, so the body carries the code
        // alone and the details go to the error observer.
        onError(error);
        return rpcFailure(500, error.code);
      }
      onError(error);
      return rpcFailure(500, "internal_error");
    }
  };

  const renderRoute = async (pathname: string): Promise<HttpResponse> => {
    const match = matchRoute(config.routes, pathname);
    if (match === null) {
      return notFoundPage();
    }
    const loaderName = loaderNames.get(match.route.name);
    const loaderFn = loaderName === undefined ? undefined : methods.get(loaderName);
    if (loaderFn === undefined) {
      onError(
        new CandidStartError(
          "app_missing_loader",
          `route ${JSON.stringify(match.route.name)} lost its loader`,
        ),
      );
      return errorPage();
    }
    try {
      const ctx: RequestContext = { caller: anonymousPrincipal, mode: "query" };
      const data = await loaderFn.invoke(
        loaderFn.input === null ? undefined : match.params,
        ctx,
      );
      const body = match.route.component({ params: match.params, data });
      const payload = {
        route: match.route.name,
        params: match.params,
        data: toWire(match.route.data, data),
      };
      const document = config.shell({
        body,
        scripts: fragment(jsonScript(HYDRATION_SCRIPT_ID, payload)),
      });
      return htmlResponse(200, renderDocument(document));
    } catch (error) {
      onError(error);
      return errorPage();
    }
  };

  const parseUrl = (url: string): string => {
    const queryIndex = url.indexOf("?");
    return queryIndex === -1 ? url : url.slice(0, queryIndex);
  };

  const httpRequest = async (req: HttpRequest): Promise<HttpResponse> => {
    const pathname = parseUrl(req.url);
    if (req.method === "GET") {
      if (pathname.startsWith(RPC_PREFIX)) {
        return rpcFailure(405, "rpc_requires_post");
      }
      return renderRoute(pathname);
    }
    if (req.method === "POST" && pathname.startsWith(RPC_PREFIX)) {
      return executeRpc(pathname.slice(RPC_PREFIX.length), req, "query");
    }
    return rpcFailure(405, "method_not_allowed");
  };

  const httpRequestUpdate = async (req: HttpRequest): Promise<HttpResponse> => {
    const pathname = parseUrl(req.url);
    if (req.method === "POST" && pathname.startsWith(RPC_PREFIX)) {
      return executeRpc(pathname.slice(RPC_PREFIX.length), req, "update");
    }
    return rpcFailure(405, "method_not_allowed");
  };

  const candid: CandidSurface = {
    methods: methodSpecs,
    invoke: async (name, arg, opts) => {
      const fn = methods.get(name);
      if (fn === undefined) {
        throw new CandidStartError(
          "unknown_method",
          `no candid method named ${JSON.stringify(name)}`,
        );
      }
      return invokeWire(fn, arg, {
        caller: opts?.caller ?? anonymousPrincipal,
        mode: fn.mode,
      });
    },
  };

  return {
    httpRequest,
    httpRequestUpdate,
    didText: () => didDocument,
    candid,
    routes: config.routes,
  };
}
