// The browser half — a typed design preview, not yet a shipped runtime.
//
// What is real and testable today: `createRpcClient` speaks the same wire
// codec and envelope as the canister's RPC surface, so a browser (or any
// fetch-capable host) calls server functions with domain values and full
// static types. What is deliberately still a sketch: `hydrate` swaps
// `innerHTML` on navigation instead of diffing, and no bundling story
// exists — shipping this file to a browser is the missing piece, and a
// bundler choice is left open (README, "Open decisions" item 2). Nothing
// here runs under Node's test suite; the file is type-checked against DOM
// types by `tsconfig.client.json`.
import type { Schema } from "./schema.ts";
import { CandidStartError } from "./errors.ts";
import { renderToString, type VNode } from "./html.ts";
import { loaderInputSchema, matchRoute, type AnyRoute } from "./router.ts";
import { fromWire, toWire, type WireValue } from "./wire.ts";
import { validate } from "./validate.ts";
import { HYDRATION_SCRIPT_ID, RPC_PREFIX } from "./canister.ts";

export interface RpcTarget<I, O> {
  readonly name: string;
  readonly input: Schema<I> | null;
  readonly output: Schema<O>;
}

export interface RpcClient {
  call<I, O>(target: RpcTarget<I, O>, input: I): Promise<O>;
}

export function createRpcClient(baseUrl: string = ""): RpcClient {
  return {
    async call<I, O>(target: RpcTarget<I, O>, input: I): Promise<O> {
      const envelope: Record<string, WireValue> = {};
      if (target.input !== null) {
        const checked = validate(target.input, input);
        if (!checked.ok) {
          throw new CandidStartError(
            "invalid_argument",
            checked.issues
              .map((issue) => `${issue.code} at ${issue.path}`)
              .join("; "),
          );
        }
        envelope["arg"] = toWire(target.input, input);
      }
      const response = await fetch(`${baseUrl}${RPC_PREFIX}${target.name}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(envelope),
      });
      const body: unknown = await response.json();
      if (
        typeof body !== "object" ||
        body === null ||
        (body as { ok?: unknown }).ok !== true
      ) {
        const code =
          typeof body === "object" && body !== null
            ? String((body as { code?: unknown }).code ?? "rpc_failed")
            : "rpc_failed";
        throw new CandidStartError(code, `server function ${target.name} failed`);
      }
      const decoded = fromWire(
        target.output,
        (body as { value?: unknown }).value,
      );
      if (!decoded.ok) {
        throw new CandidStartError(
          "rpc_malformed_result",
          decoded.issues
            .map((issue) => `${issue.code} at ${issue.path}`)
            .join("; "),
        );
      }
      // `fromWire` is structural only; its contract is that the caller then
      // validates. The canister honors that on both directions, so the client
      // must too — otherwise a peer answering with a structurally-decodable
      // but out-of-domain value (a negative `nat`, a fractional `nat8`) would
      // resolve the typed Promise with a value its schema forbids.
      const checked = validate(target.output, decoded.value);
      if (!checked.ok) {
        throw new CandidStartError(
          "rpc_invalid_result",
          checked.issues
            .map((issue) => `${issue.code} at ${issue.path}`)
            .join("; "),
        );
      }
      return decoded.value;
    },
  };
}

export interface HydrationPayload {
  readonly route: string;
  readonly params: Record<string, string>;
  readonly data: unknown;
}

export function readHydrationPayload(): HydrationPayload | null {
  const element = document.getElementById(HYDRATION_SCRIPT_ID);
  if (element === null || element.textContent === null) {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(element.textContent);
  } catch {
    return null;
  }
  // The cast is only sound if the shape actually holds; a tampered or absent
  // payload must read as `null`, not as a typed object with missing fields.
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    typeof (parsed as { route?: unknown }).route !== "string" ||
    typeof (parsed as { params?: unknown }).params !== "object" ||
    (parsed as { params: unknown }).params === null
  ) {
    return null;
  }
  return parsed as HydrationPayload;
}

export interface ClientOptions {
  readonly routes: readonly AnyRoute[];
  /** The element whose content is replaced on navigation. */
  readonly container: Element;
  readonly rpc?: RpcClient;
}

/**
 * Intercept same-origin link clicks and drive navigation through route
 * loaders as RPC query calls — TanStack Start's client navigation, with the
 * loader living in the canister. Rendering is a full `innerHTML` swap of
 * the container; a diffing renderer is future work by design.
 */
export function hydrate(options: ClientOptions): void {
  const rpc = options.rpc ?? createRpcClient();

  const navigate = async (pathname: string, push: boolean): Promise<void> => {
    const match = matchRoute(options.routes, pathname);
    if (match === null) {
      window.location.assign(pathname);
      return;
    }
    const loaderName = `${match.route.name}_loader`;
    const input = loaderInputSchema(match.route);
    const data =
      input === null
        ? await rpc.call(
            {
              name: loaderName,
              input: null,
              output: match.route.data,
            } as RpcTarget<void, unknown>,
            undefined,
          )
        : await rpc.call(
            { name: loaderName, input, output: match.route.data },
            match.params,
          );
    const view: VNode = match.route.component({
      params: match.params,
      data,
    });
    options.container.innerHTML = renderToString(view);
    if (push) {
      history.pushState(null, "", pathname);
    }
  };

  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) {
      return;
    }
    const anchor = target.closest("a");
    if (
      anchor === null ||
      anchor.origin !== window.location.origin ||
      anchor.hasAttribute("download") ||
      anchor.target !== ""
    ) {
      return;
    }
    event.preventDefault();
    void navigate(anchor.pathname, true).catch(() => {
      window.location.assign(anchor.pathname);
    });
  });

  window.addEventListener("popstate", () => {
    void navigate(window.location.pathname, false).catch(() => {
      window.location.reload();
    });
  });
}
