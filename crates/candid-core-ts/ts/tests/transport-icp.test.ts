// The `@icp-sdk/core` transport adapter, tested over a mock `fetch` — the
// issue #154 acceptance paths: the query reject path, the certified reply
// path, and root-key wiring. Nothing here stubs the agent: every test runs
// the real `HttpAgent` from the pinned `@icp-sdk/core@6.1.0` against
// fabricated replica responses, and the certified-path responses carry a
// genuinely BLS-signed certificate over a root key generated in the test —
// so a passing test proves the agent's certificate verification accepted
// the reply *because* the transport wired our root key through, not because
// verification was bypassed.

import { test } from "node:test";
import assert from "node:assert/strict";

import { bls12_381 } from "@noble/curves/bls12-381.js";
import {
  Cbor,
  HttpAgent,
  NodeType,
  domain_sep,
  reconstruct,
  requestIdOf,
  type CallRequest,
  type HashTree,
  type NodeLabel,
  type NodeValue,
} from "@icp-sdk/core/agent";

import { httpTransport } from "../transport-icp.ts";
import type { CallTarget } from "../actor.ts";

const CANISTER = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const MANAGEMENT = "aaaaa-aa";
const HOST = "http://127.0.0.1:4943";
/** `()` — the empty candid argument/reply. */
const EMPTY_ARGS = Uint8Array.from([68, 73, 68, 76, 0, 0]);
/** `(42 : nat)` — a distinguishable certified reply. */
const REPLY = Uint8Array.from([68, 73, 68, 76, 0, 1, 125, 42]);

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);

const hexToBytes = (hex: string): Uint8Array =>
  Uint8Array.from((hex.match(/../g) ?? []).map((byte) => parseInt(byte, 16)));

// The IC root-key DER envelope, exactly as the agent's `extractDER` expects
// it: this constant prefix followed by the 96-byte BLS12-381 G2 public key.
const DER_PREFIX = hexToBytes(
  "308182301d060d2b0601040182dc7c0503010201060c2b0601040182dc7c05030201036100",
);

/** Unsigned LEB128, for the certificate's nanosecond `time` leaf. */
function lebEncode(value: bigint): Uint8Array {
  const out: number[] = [];
  let rest = value;
  do {
    let byte = Number(rest & 0x7fn);
    rest >>= 7n;
    if (rest !== 0n) {
      byte |= 0x80;
    }
    out.push(byte);
  } while (rest !== 0n);
  return Uint8Array.from(out);
}

// The IC hash-tree encoding the certificate module reads, built with the
// SDK's own `HashTree` node types. Labels under a fork stay sorted
// ("reply" < "status", "request_status" < "time"), the order `lookup_path`
// requires. The label and value brands are nominal stamps on `Uint8Array`;
// these two casts are the whole bridge.
const label = (bytes: Uint8Array): NodeLabel => bytes as NodeLabel;
const leafValue = (bytes: Uint8Array): NodeValue => bytes as NodeValue;

/** One BLS root key pair per suite run; the DER form is what consumers pass. */
const rootSecret = bls12_381.utils.randomSecretKey();
const shortSignatures = bls12_381.shortSignatures;
const rootKey = new Uint8Array([
  ...DER_PREFIX,
  ...shortSignatures.getPublicKey(rootSecret).toBytes(),
]);

/** A certificate for `requestId` marked replied, signed by our root key. */
async function certificateFor(requestId: Uint8Array): Promise<Uint8Array> {
  const statusTree: HashTree = [
    NodeType.Fork,
    [NodeType.Labeled, label(utf8("reply")), [NodeType.Leaf, leafValue(REPLY)]],
    [NodeType.Labeled, label(utf8("status")), [NodeType.Leaf, leafValue(utf8("replied"))]],
  ];
  const tree: HashTree = [
    NodeType.Fork,
    [
      NodeType.Labeled,
      label(utf8("request_status")),
      [NodeType.Labeled, label(requestId), statusTree],
    ],
    [
      NodeType.Labeled,
      label(utf8("time")),
      [NodeType.Leaf, leafValue(lebEncode(BigInt(Date.now()) * 1_000_000n))],
    ],
  ];
  const rootHash = await reconstruct(tree);
  const message = new Uint8Array([...domain_sep("ic-state-root"), ...rootHash]);
  const signature = shortSignatures.sign(shortSignatures.hash(message), rootSecret).toBytes();
  return Cbor.encode({ tree, signature });
}

/** The URLs a mock saw, so routing can be asserted. */
function recordingCallFetch(seen: string[]): typeof fetch {
  return async (input, init) => {
    const url = String(input);
    seen.push(url);
    if (!url.includes("/call")) {
      return new Response("unexpected request", { status: 404 });
    }
    const envelope = Cbor.decode<{ content: CallRequest }>(
      new Uint8Array(init?.body as ArrayBuffer),
    );
    const certificate = await certificateFor(requestIdOf(envelope.content));
    return new Response(Cbor.encode({ status: "replied", certificate }), {
      status: 200,
      headers: { "Content-Type": "application/cbor" },
    });
  };
}

function queryFetch(body: (url: string) => unknown): typeof fetch {
  return async (input) => {
    const url = String(input);
    if (!url.includes("/query")) {
      return new Response("unexpected request", { status: 404 });
    }
    return new Response(Cbor.encode(body(url)), {
      status: 200,
      headers: { "Content-Type": "application/cbor" },
    });
  };
}

/** Run `body` with `globalThis.fetch` replaced, restoring it either way. */
async function withGlobalFetch(mock: typeof fetch, body: () => Promise<void>): Promise<void> {
  const original = globalThis.fetch;
  globalThis.fetch = mock;
  try {
    await body();
  } finally {
    globalThis.fetch = original;
  }
}

test("a replied query returns the reply bytes through the pre-built agent option", async () => {
  const agent = HttpAgent.createSync({
    host: HOST,
    fetch: queryFetch(() => ({ status: "replied", reply: { arg: REPLY } })),
    verifyQuerySignatures: false,
  });
  const transport = httpTransport({ agent });
  const reply = await transport.query({ canisterId: CANISTER, methodName: "echo" }, EMPTY_ARGS);
  assert.deepStrictEqual(reply, REPLY);
});

test("a rejected query throws an error naming the method, code, and message", async () => {
  const agent = HttpAgent.createSync({
    host: HOST,
    fetch: queryFetch(() => ({
      status: "rejected",
      reject_code: 4,
      reject_message: "no such method",
      error_code: "IC0302",
    })),
    verifyQuerySignatures: false,
  });
  const transport = httpTransport({ agent });
  await assert.rejects(
    () => transport.query({ canisterId: CANISTER, methodName: "echo" }, EMPTY_ARGS),
    (error: unknown) =>
      error instanceof Error && error.message === "echo rejected (4): no such method",
  );
});

test("a certified reply reaches the caller only through the wired root key", async () => {
  // The positive half: the transport builds its own agent from host and
  // rootKey, the mock replies with a certificate signed by that key's
  // secret, and the agent's verification accepts it.
  const seen: string[] = [];
  await withGlobalFetch(recordingCallFetch(seen), async () => {
    const transport = httpTransport({ host: HOST, rootKey });
    const reply = await transport.call({ canisterId: CANISTER, methodName: "ping" }, EMPTY_ARGS);
    assert.deepStrictEqual(reply, REPLY);
  });
  assert.strictEqual(seen.length, 1, "one call request reaches the replica");

  // The adversarial half, proving the wiring is load-bearing: the same
  // mock under a *different* root key must be refused by certificate
  // verification — if this passed, the positive half proved nothing.
  const otherKey = new Uint8Array([
    ...DER_PREFIX,
    ...shortSignatures.getPublicKey(bls12_381.utils.randomSecretKey()).toBytes(),
  ]);
  await withGlobalFetch(recordingCallFetch([]), async () => {
    const transport = httpTransport({ host: HOST, rootKey: otherKey });
    await assert.rejects(
      () => transport.call({ canisterId: CANISTER, methodName: "ping" }, EMPTY_ARGS),
      (error: unknown) => error instanceof Error && /signature|verif/i.test(error.message),
    );
  });
});

test("effectiveCanisterId routes the call; its absence routes to the canister", async () => {
  // Management-canister routing is the one call shape that needs the field —
  // and dropping it is the exact footgun the inline #148 adapter fixed.
  const routed: string[] = [];
  await withGlobalFetch(recordingCallFetch(routed), async () => {
    const transport = httpTransport({ host: HOST, rootKey });
    const target: CallTarget = {
      canisterId: MANAGEMENT,
      methodName: "install_code",
      effectiveCanisterId: CANISTER,
    };
    await transport.call(target, EMPTY_ARGS);
  });
  assert.strictEqual(routed.length, 1);
  assert.match(routed[0], new RegExp(`/canister/${CANISTER}/`), "the effective id routes");

  const direct: string[] = [];
  await withGlobalFetch(recordingCallFetch(direct), async () => {
    const transport = httpTransport({ host: HOST, rootKey });
    await transport.call({ canisterId: CANISTER, methodName: "ping" }, EMPTY_ARGS);
  });
  assert.match(direct[0], new RegExp(`/canister/${CANISTER}/`));
});

test("a pre-built agent refuses to travel with host or rootKey", () => {
  const agent = HttpAgent.createSync({ host: HOST, fetch: queryFetch(() => ({})) });
  assert.throws(() => httpTransport({ agent, host: HOST }), TypeError);
  assert.throws(() => httpTransport({ agent, rootKey }), TypeError);
  // Alone, both configurations construct.
  httpTransport({ agent });
  httpTransport({ host: HOST, rootKey });
});
