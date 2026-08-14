// The `@icp-sdk/core` Transport adapter — issue #154, option (b): a subpath
// export, so the adapter is written once, compiled, and tested here instead
// of re-derived by every consumer against a moving agent API. Importing this
// module is what makes the `@icp-sdk/core` peer a *runtime* requirement; no
// other subpath imports it, so consumers who never touch this file keep the
// type-only peer situation exactly as documented in the README.
//
// # Why the peer range is `>= 6`
//
// `call` returns `agent.update(...).reply` in one shot: on v6 the agent
// submits, polls to completion, and verifies the certificate before
// resolving, so the returned bytes are the certified reply. Older majors
// resolved `update` at submission and left polling to the caller — an
// adapter for them needs its own submit-and-poll loop and is deliberately
// not written here.
//
// # What stays on the agent's side of the pipe
//
// Identity, ingress expiry, retries, polling strategy, and query-signature
// verification are agent concerns (decision recorded on #104). This adapter
// exposes exactly the two knobs a plain consumer needs — `host` and
// `rootKey` — and takes a pre-built agent for everything beyond them,
// rather than re-exporting agent configuration. Composition covers logging:
// wrap the returned `Transport`.

import { HttpAgent, QueryResponseStatus, type Agent } from "@icp-sdk/core/agent";

import type { CallTarget, Transport } from "./actor.ts";

/**
 * Options for {@link httpTransport}. `host` and `rootKey` configure the
 * transport's own `HttpAgent`; `agent` supplies a pre-built one instead —
 * for identity, retries, ingress options, or anything else beyond those two
 * knobs — and must then travel alone.
 */
export interface HttpTransportOptions {
  /**
   * The root key certificates are verified against. Needed for local
   * networks; omit on mainnet, whose root key the agent already knows.
   */
  readonly rootKey?: Uint8Array;
  /** The replica URL. Defaults to the page origin. */
  readonly host?: string;
  /**
   * A pre-built agent, when host and root key are not enough. Exclusive
   * with the other options: configuring a supplied agent from here would
   * silently discard whatever it was built with, so the combination is
   * refused instead.
   */
  readonly agent?: Agent;
}

/**
 * The effective routing target: `effectiveCanisterId` when the call names
 * one (management-canister routing is why `CallTarget` carries it), the
 * canister itself otherwise. The agent normalizes the text to a principal.
 */
function effectiveTargetOf(target: CallTarget): { canisterId: string } {
  return { canisterId: target.effectiveCanisterId ?? target.canisterId };
}

/**
 * A {@link Transport} over `@icp-sdk/core`'s `HttpAgent` (peer `>= 6`).
 *
 * `query` is the non-replicated read path and throws a plain `Error` naming
 * the reject code and message when the replica rejects. `call` is the
 * replicated path; the returned bytes are certificate-verified by the agent
 * before this adapter ever sees them. Codec failures are not this layer's
 * concern — `createActor` rejects with `ActorError` before or after the
 * transport runs.
 *
 * Throws `TypeError` immediately when a pre-built `agent` is combined with
 * `host` or `rootKey` — a programmer error, unlike the data and transport
 * errors that reject call promises.
 */
export function httpTransport(options: HttpTransportOptions = {}): Transport {
  if (
    options.agent !== undefined &&
    (options.host !== undefined || options.rootKey !== undefined)
  ) {
    throw new TypeError("httpTransport takes either a pre-built agent or host/rootKey, not both");
  }
  const agentPromise =
    options.agent !== undefined
      ? Promise.resolve(options.agent)
      : HttpAgent.create({
          ...(options.host !== undefined ? { host: options.host } : {}),
          ...(options.rootKey !== undefined ? { rootKey: options.rootKey } : {}),
        });
  return {
    async query(target, arg) {
      const agent = await agentPromise;
      const response = await agent.query(target.canisterId, {
        methodName: target.methodName,
        arg,
        effectiveTarget: effectiveTargetOf(target),
      });
      if (response.status !== QueryResponseStatus.Replied) {
        throw new Error(
          `${target.methodName} rejected (${response.reject_code}): ${response.reject_message}`,
        );
      }
      return response.reply.arg;
    },
    async call(target, arg) {
      const agent = await agentPromise;
      const result = await agent.update(target.canisterId, {
        methodName: target.methodName,
        arg,
        effectiveTarget: effectiveTargetOf(target),
      });
      // Certificate-verified by the agent: v6's update polls to completion
      // and refuses an unverifiable reply before resolving.
      return result.reply;
    },
  };
}
