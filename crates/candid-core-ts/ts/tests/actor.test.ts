// The issue #104 integration gate: encode → transport → decode against a
// mocked agent, over the real generated ledger module. The mock decodes each
// request's argument bytes with the same schemas and encodes its reply with
// them, so a byte-level disagreement anywhere in the chain fails loudly. No
// live network, no agent dependency — the transport is a dictionary.
//
// The hand-written `ExpectedLedgerActor` interface is the acceptance
// criterion's "proven against a hand-written expected interface": the
// `Equals` proof below is the same invariance trick the goldens use, so a
// drift in either the emitted `Actor` type or the runtime factory's shape
// turns this file red under tsc.

import { test } from "node:test";
import assert from "node:assert/strict";

import { createActor, callFunc, ActorError, type Transport, type CallTarget } from "../actor.ts";
import { encodeArgs, decodeArgs } from "../codec.ts";
import { c, type AnySchema } from "../schema.ts";
import * as ledger from "../../tests/goldens/ledger.ts";

// ---------------------------------------------------------------------------
// The acceptance proof: emitted Actor type === hand-written interface.
// ---------------------------------------------------------------------------

type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

interface ExpectedLedgerActor {
  fee: () => Promise<ledger.Tokens>;
  decimals: () => Promise<number>;
  name: () => Promise<string>;
  balance_of: (arg0: ledger.Account) => Promise<ledger.Tokens>;
  get_transactions: (arg0: {
    start: bigint;
    length: bigint;
  }) => Promise<ledger.TransactionsResponse>;
  transfer: (arg0: ledger.TransferArg) => Promise<ledger.TransferResult>;
  total_supply: () => Promise<ledger.Tokens>;
  symbol: () => Promise<string>;
}

const actorTypeMatchesHandWrittenInterface: Equals<ledger.Actor, ExpectedLedgerActor> = true;
void actorTypeMatchesHandWrittenInterface;

// ---------------------------------------------------------------------------
// The mock: a dictionary of schema-decoding handlers.
// ---------------------------------------------------------------------------

const principal = { toText: () => "aaaaa-aa" };

interface Call {
  readonly path: "query" | "call";
  readonly target: CallTarget;
}

function mockTransport(
  handlers: Record<string, (args: readonly unknown[]) => readonly unknown[]>,
  argSchemas: Record<string, readonly AnySchema[]>,
  resultSchemas: Record<string, readonly AnySchema[]>,
  log: Call[],
): Transport {
  const respond = async (
    path: "query" | "call",
    target: CallTarget,
    arg: Uint8Array,
  ): Promise<Uint8Array> => {
    log.push({ path, target });
    const handler = handlers[target.methodName];
    assert(handler !== undefined, `no handler for ${target.methodName}`);
    const decoded = decodeArgs(argSchemas[target.methodName] ?? [], arg);
    assert(decoded.ok, `the mock must decode ${target.methodName}'s argument bytes`);
    if (!decoded.ok) {
      throw new Error("unreachable");
    }
    const reply = handler(decoded.values);
    const encoded = encodeArgs(resultSchemas[target.methodName] ?? [], reply);
    assert(encoded.ok, `the mock must encode ${target.methodName}'s reply`);
    if (!encoded.ok) {
      throw new Error("unreachable");
    }
    return encoded.bytes;
  };
  return {
    query: (target, arg) => respond("query", target, arg),
    call: (target, arg) => respond("call", target, arg),
  };
}

// ---------------------------------------------------------------------------
// The ledger flow, archived-transactions callback included.
// ---------------------------------------------------------------------------

test("the generated ledger actor round-trips through a mocked transport", async () => {
  const log: Call[] = [];
  const archived = {
    callback: { principal, method: "get_archived" },
    start: 0n,
    length: 1n,
  };
  const response: ledger.TransactionsResponse = {
    log_length: 2n,
    transactions: [],
    archived_transactions: [archived],
  };
  const transport = mockTransport(
    {
      name: () => ["Internet Computer"],
      transfer: (args) => {
        const arg = args[0] as ledger.TransferArg;
        assert.strictEqual(arg.amount.e8s, 7n);
        return [{ tag: "ok", value: 42n }];
      },
      get_transactions: () => [response],
      get_archived: () => [{ transactions: [] }],
    },
    {
      name: [],
      transfer: [ledger.TransferArg],
      get_transactions: [c.record({ start: c.nat, length: c.nat })],
      get_archived: [c.record({ start: c.nat, length: c.nat })],
    },
    {
      name: [c.text],
      transfer: [ledger.TransferResult],
      get_transactions: [ledger.TransactionsResponse],
      get_archived: [ledger.TransactionRange],
    },
    log,
  );

  const actor = createActor<ledger.Actor>(ledger.actor, "aaaaa-aa", transport);

  // A query method rides the query path.
  assert.strictEqual(await actor.name(), "Internet Computer");
  // An update rides the call path, arguments intact through the codec.
  const transferred = await actor.transfer({
    to: { owner: principal, subaccount: null },
    fee: null,
    memo: null,
    from_subaccount: null,
    created_at_time: null,
    amount: { e8s: 7n },
  });
  assert.deepStrictEqual(transferred, { tag: "ok", value: 42n });

  // The real ledger paging pattern: the response carries a func *value*,
  // and invoking that reference is a fresh query against its own service.
  const page = await actor.get_transactions({ start: 0n, length: 10n });
  const callback = page.archived_transactions[0].callback;
  assert.strictEqual(callback.method, "get_archived");
  const archivedPage = await callFunc(
    ledger.ArchiveCallback,
    callback,
    [{ start: 0n, length: 1n }],
    transport,
  );
  assert.deepStrictEqual(archivedPage, { transactions: [] });

  assert.deepStrictEqual(
    log.map((entry) => [entry.path, entry.target.methodName]),
    [
      ["query", "name"],
      ["call", "transfer"],
      ["query", "get_transactions"],
      ["query", "get_archived"],
    ],
    "modes dispatch onto the recorded paths",
  );
  assert(
    log.every((entry) => entry.target.canisterId === "aaaaa-aa"),
    "every call targeted the actor's canister (the callback carries its own principal)",
  );
});

test("oneway dispatches on the call path and resolves to undefined", async () => {
  const log: Call[] = [];
  const transport = mockTransport({ fire: () => [] }, { fire: [] }, { fire: [] }, log);
  const service = c.service({ fire: c.func([], [], "oneway") });
  const actor = createActor<{ fire: () => Promise<void> }>(service, "aaaaa-aa", transport);
  assert.strictEqual(await actor.fire(), undefined);
  assert.deepStrictEqual(
    log.map((entry) => entry.path),
    ["call"],
  );
});

test("actor failures are typed: codec issues reject, misuse throws", async () => {
  const log: Call[] = [];
  const transport = mockTransport({ take: () => [] }, { take: [c.nat] }, { take: [] }, log);
  const service = c.service({ take: c.func([c.nat], [], "update") });
  const actor = createActor<{ take: (value: bigint) => Promise<void> }>(
    service,
    "aaaaa-aa",
    transport,
  );
  // An invalid argument rejects with the codec's issues before any call.
  await assert.rejects(
    // The wrong type on purpose: the runtime must refuse it.
    () => actor.take(5 as unknown as bigint),
    (error: unknown) => {
      if (!(error instanceof ActorError)) {
        return false;
      }
      assert.strictEqual(error.issues[0]?.code, "invalid_type");
      return true;
    },
  );
  assert.strictEqual(log.length, 0, "nothing crossed the transport");
  // A non-service schema is a programmer error and throws synchronously.
  assert.throws(() => createActor(c.nat, "aaaaa-aa", transport), TypeError);
});
