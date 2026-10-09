// Issue #244: each method of a generated `Actor` carries its mode in the type
// system, as its call signature intersected with `WithMode<mode>`, and
// `ModeOf` reads it back. What this file pins, at compile time (the tsc
// harness) and at run time (`npm test`):
//
// - every mode — `query`, `composite_query`, `update`, `oneway` — reads back
//   from the `modes` golden, for a method written inline, one typed by a
//   declared func, and one with a quoted name, and the mode the type carries
//   is the mode the `actor` schema calls the method with;
// - the marks are additive: with them stripped, each `Actor` method is the
//   exact call signature emitted before, `keyof Actor` is unchanged, and a
//   plain async function still implements a method;
// - an `Actor` written by hand, without marks, stays valid and reads as mode
//   unknown — the whole `MethodMode` union;
// - an actor with no methods has an empty `Actor`;
// - the reading a call layer such as ic-reactor would do — refuse an update
//   or oneway method where a read is built, accept a method of unknown mode —
//   works on the types a consumer already passes (`Actor` alone).
//
// The `@ts-expect-error` lines are compile-time assertions: if a refusal
// stops biting, the unused directive turns the harness red.

import { test } from "node:test";
import assert from "node:assert/strict";

import { serviceMethods, type MethodMode, type ModeOf, type WithMode } from "../schema.ts";

import * as ledger from "../../tests/goldens/ledger.ts";
import * as methodless from "../../tests/goldens/methodless.ts";
import * as modes from "../../tests/goldens/modes.ts";

type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

/** The call signature of a method type, without anything intersected with it. */
type CallOf<F> = F extends (...args: infer P) => infer R ? (...args: P) => R : never;

/** Every method of an `Actor` as its bare call signature. */
type Signatures<A> = { [K in keyof A]: CallOf<A[K]> };

/** Every method of an `Actor` mapped to the mode it reads as. */
type Modes<A> = { [K in keyof A]: ModeOf<A[K]> };

// --- Every mode reads back -------------------------------------------------

export const modesReadBack: Equals<
  Modes<modes.Actor>,
  {
    balance: "query";
    notify: "oneway";
    "quoted name": "query";
    read: "query";
    aggregate: "composite_query";
    announce: "oneway";
    transfer: "update";
  }
> = true;

// Typed by `ModeOf`, so each value can only be the one literal the type
// carries; the test below compares it with the mode the schema calls with.
const modesByType: Modes<modes.Actor> = {
  balance: "query",
  notify: "oneway",
  "quoted name": "query",
  read: "query",
  aggregate: "composite_query",
  announce: "oneway",
  transfer: "update",
};

const ledgerModesByType: Modes<ledger.Actor> = {
  fee: "query",
  decimals: "query",
  name: "query",
  balance_of: "query",
  get_transactions: "query",
  transfer: "update",
  total_supply: "query",
  symbol: "query",
};

test("the mode each Actor method carries is the mode its actor schema calls with", () => {
  for (const [actor, byType] of [
    [modes.actor, modesByType],
    [ledger.actor, ledgerModesByType],
  ] as const) {
    const table = serviceMethods(actor);
    assert.deepStrictEqual(
      [...table.keys()].sort(),
      Object.keys(byType).sort(),
      "the type and the schema name the same methods",
    );
    for (const [name, mode] of Object.entries(byType)) {
      assert.strictEqual(table.get(name)?.mode, mode, name);
    }
  }
});

test("an actor with no methods has an empty service and an empty Actor", () => {
  assert.strictEqual(serviceMethods(methodless.actor).size, 0);
});

// --- The marks are additive ------------------------------------------------

// The call signatures the ledger golden emitted before the marks, verbatim.
interface LedgerSignatures {
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

export const ledgerSignaturesUnchanged: Equals<Signatures<ledger.Actor>, LedgerSignatures> = true;
export const ledgerKeysUnchanged: Equals<keyof ledger.Actor, keyof LedgerSignatures> = true;

export const modesSignaturesUnchanged: Equals<
  Signatures<modes.Actor>,
  {
    balance: () => Promise<bigint>;
    notify: (arg0: bigint) => Promise<void>;
    "quoted name": () => Promise<bigint>;
    read: () => Promise<bigint>;
    aggregate: (arg0: string) => Promise<Array<bigint>>;
    announce: (arg0: string) => Promise<void>;
    transfer: (arg0: bigint, arg1: string) => Promise<boolean>;
  }
> = true;

// The library helpers a call layer infers with see through the mark.
export const parametersSeeThrough: Equals<
  Parameters<modes.Actor["transfer"]>,
  [arg0: bigint, arg1: string]
> = true;
export const returnTypeSeesThrough: Equals<
  ReturnType<modes.Actor["aggregate"]>,
  Promise<Array<bigint>>
> = true;

// A plain object of plain async functions still implements a generated Actor.
export const implementsModes: modes.Actor = {
  balance: async () => 1n,
  notify: async () => {},
  "quoted name": async () => 2n,
  read: async () => 3n,
  aggregate: async (text) => [BigInt(text.length)],
  announce: async () => {},
  transfer: async (amount, memo) => amount > 0n && memo.length > 0,
};

// A mark of one mode is not a mark of another.
// @ts-expect-error a query method is not an update method
export const queryIsNotUpdate: (() => Promise<bigint>) & WithMode<"update"> =
  null as unknown as modes.Actor["balance"];

// --- An Actor written by hand ----------------------------------------------

type HandWritten = {
  fee: () => Promise<bigint>;
  transfer: (amount: bigint) => Promise<void>;
};

export const handWrittenReadsUnknown: Equals<
  Modes<HandWritten>,
  { fee: MethodMode; transfer: MethodMode }
> = true;

// A hand-written Actor may carry marks too, on some methods and not others.
type PartlyMarked = {
  fee: (() => Promise<bigint>) & WithMode<"query">;
  transfer: (amount: bigint) => Promise<void>;
};

export const partlyMarkedReads: Equals<
  Modes<PartlyMarked>,
  { fee: "query"; transfer: MethodMode }
> = true;

// --- No methods ------------------------------------------------------------

export const methodlessIsEmpty: Equals<keyof methodless.Actor, never> = true;
export const methodlessModes: Equals<Modes<methodless.Actor>, {}> = true;

// --- How a call layer reads it --------------------------------------------

// The shape of ic-reactor's check, from the `Actor` an app already passes:
// a read is built only from a method that is not known to be an update or a
// oneway; a method of unknown mode stays accepted and is checked at run time.
type ReadMethod<A> = {
  [K in keyof A & string]: [ModeOf<A[K]>] extends ["update" | "oneway"] ? never : K;
}[keyof A & string];

declare function queryOptions<A, M extends ReadMethod<A>>(actor: A, method: M): void;

declare const modesActor: modes.Actor;
declare const ledgerActor: ledger.Actor;
declare const handWritten: HandWritten;
declare const methodlessActor: methodless.Actor;

export const readMethodsOfModes: Equals<
  ReadMethod<modes.Actor>,
  "balance" | "quoted name" | "read" | "aggregate"
> = true;
export const readMethodsOfHandWritten: Equals<ReadMethod<HandWritten>, "fee" | "transfer"> = true;
export const readMethodsOfMethodless: Equals<ReadMethod<methodless.Actor>, never> = true;

export function callLayerProbes(): void {
  queryOptions(modesActor, "balance");
  queryOptions(modesActor, "aggregate");
  queryOptions(ledgerActor, "fee");
  queryOptions(handWritten, "transfer");
  // @ts-expect-error an update method is refused where a read is built
  queryOptions(modesActor, "transfer");
  // @ts-expect-error a oneway method is refused where a read is built
  queryOptions(modesActor, "notify");
  // @ts-expect-error the ledger's update method is refused too
  queryOptions(ledgerActor, "transfer");
  // @ts-expect-error an actor with no methods has nothing to read
  queryOptions(methodlessActor, "anything");
}
