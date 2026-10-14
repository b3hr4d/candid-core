// `decode` and `decodeArgs` type their values by the schemas they decoded
// under (issue #267). The decoder checks the bytes against `Schema<T>` before
// it returns `ok: true`, so `T` is the schema's own domain, not a cast. An
// erased schema (`AnySchema` is `Schema<any>`) stays `unknown`: a naive
// signature would hand the call layers that hold erased schemas an `any`.
//
// The `Equal` probes and `@ts-expect-error` lines are compile-time assertions
// wired into the tsc harness: if a type drifts, `tsc --noEmit` turns red. The
// runtime tests below show the typed values are the decoded ones.

import { test } from "node:test";
import assert from "node:assert/strict";

import { c, type AnyFieldSchema, type AnySchema, type Infer } from "../schema.ts";
import {
  decode,
  decodeArgs,
  encode,
  encodeArgs,
  type CodecIssue,
  type DecodeResult,
} from "../codec.ts";

type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

const Account = c.record({ owner: c.principal, balance: c.nat });
type Account = Infer<typeof Account>;
const Memo = c.opt(c.blob());
type Memo = Infer<typeof Memo>;

const erased: AnySchema = Account;
const func = c.func([Account], [Account, c.text], "query");

/** The `ok` value of a `decode` result type. */
type Value<R> = Extract<R, { ok: true }> extends { value: infer V } ? V : never;
/** The `ok` values of a `decodeArgs` result type. */
type Values<R> = Extract<R, { ok: true }> extends { values: infer V } ? V : never;

// Types only: these calls are never made at run time.
declare const bytes: Uint8Array;
declare const mutableList: AnyFieldSchema[];
declare const probe: {
  account: ReturnType<typeof decode<Account>>;
};

// A concrete schema: the value is `Infer<typeof schema>`.
function typesOnly() {
  const account = decode(Account, bytes);
  const literal = decodeArgs([Account, Memo], bytes);
  const none = decodeArgs([], bytes);
  const fromErased = decode(erased, bytes);
  const fromResults = decodeArgs(func.results, bytes);
  const fromMutable = decodeArgs(mutableList, bytes);
  const mixed = decodeArgs([Account, erased], bytes);
  const never = decode(c.empty, bytes);
  return { account, literal, none, fromErased, fromResults, fromMutable, mixed, never };
}
type Probes = ReturnType<typeof typesOnly>;

export const decodeTypesByItsSchema: Equal<Value<Probes["account"]>, Account> = true;
export const instantiatedDecodeAgrees: Equal<Value<(typeof probe)["account"]>, Account> = true;
// An array literal gives the value tuple, with no `as const`.
export const decodeArgsTypesTheTuple: Equal<
  Values<Probes["literal"]>,
  readonly [Account, Memo]
> = true;
export const decodeArgsTypesNoArgs: Equal<Values<Probes["none"]>, readonly []> = true;
// Erased input stays `unknown`, never `any`: one erased schema, the
// `readonly AnyFieldSchema[]` a `FuncSchema` holds, and one erased element.
export const erasedDecodesToUnknown: Equal<Value<Probes["fromErased"]>, unknown> = true;
export const erasedListDecodesToUnknowns: Equal<
  Values<Probes["fromResults"]>,
  readonly unknown[]
> = true;
// A mutable erased list still gives a readonly result.
export const mutableListDecodesToReadonly: Equal<
  Values<Probes["fromMutable"]>,
  readonly unknown[]
> = true;
export const erasedElementDecodesToUnknown: Equal<
  Values<Probes["mixed"]>,
  readonly [Account, unknown]
> = true;
// `empty` decodes to nothing at all.
export const emptyDecodesToNever: Equal<Value<Probes["never"]>, never> = true;

// The exported name keeps its old meaning by default.
export const decodeResultDefault: Equal<
  DecodeResult,
  | { readonly ok: true; readonly values: readonly unknown[] }
  | { readonly ok: false; readonly issues: readonly CodecIssue[] }
> = true;

export function wrongAssignmentsAreRefused(): void {
  const one = decode(Account, bytes);
  if (one.ok) {
    const balance: bigint = one.value.balance;
    void balance;
    // @ts-expect-error a decoded Account is not a string
    const wrong: string = one.value;
    void wrong;
    // @ts-expect-error a decoded Account has no `owner` of type bigint
    const owner: bigint = one.value.owner;
    void owner;
  }
  const many = decodeArgs([Account, c.text], bytes);
  if (many.ok) {
    const first: Account = many.values[0];
    const second: string = many.values[1];
    void [first, second];
    // @ts-expect-error the second value is text, not an Account
    const swapped: Account = many.values[1];
    void swapped;
    // @ts-expect-error the tuple has two elements
    void many.values[2];
  }
  const loose = decode(erased, bytes);
  if (loose.ok) {
    // @ts-expect-error an erased schema decodes to unknown, which is not a string
    const text: string = loose.value;
    void text;
  }
  const results = decodeArgs(func.results, bytes);
  if (results.ok) {
    // @ts-expect-error an erased list decodes to unknowns
    const text: string = results.values[0];
    void text;
  }
}

test("the typed value is the decoded value", () => {
  const account = { owner: "aaaaa-aa", balance: 5n };
  const encoded = encode(Account, account);
  if (!encoded.ok) throw new Error("encode failed");
  const decoded = decode(Account, encoded.bytes);
  if (!decoded.ok) throw new Error("decode failed");
  const balance: bigint = decoded.value.balance;
  assert.strictEqual(balance, 5n);
  assert.deepStrictEqual(decoded.value, account);

  const args = encodeArgs([Account, Memo], [account, new Uint8Array([1, 2])]);
  if (!args.ok) throw new Error("encodeArgs failed");
  const values = decodeArgs([Account, Memo], args.bytes);
  if (!values.ok) throw new Error("decodeArgs failed");
  const [first, memo]: readonly [Account, Memo] = values.values;
  assert.deepStrictEqual(first, account);
  assert.deepStrictEqual(memo, new Uint8Array([1, 2]));
});

test("bytes that do not fit the schema are refused, not typed", () => {
  const text = encode(c.text, "not an account");
  if (!text.ok) throw new Error("encode failed");
  assert.strictEqual(decode(Account, text.bytes).ok, false);
  assert.strictEqual(decodeArgs([Account], text.bytes).ok, false);
});
