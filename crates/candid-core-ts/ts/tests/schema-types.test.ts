// Issue #126: `c.empty` (`Schema<never>`) is admitted by every composite
// position through the `AnyFieldSchema` bound, and the widening must not
// weaken the invariant equality gate. The `@ts-expect-error` lines are
// compile-time assertions wired into the tsc harness: each marks an
// assignment the gate must refuse, so if the gate ever weakens, the
// suppressed error disappears and tsc reports the unused directive —
// turning the harness red. The runtime tests pin what `validate` says about
// the same shapes, so type admission and runtime behavior move together.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  c,
  isBoxedOpt,
  resolveSchema,
  type AnySchema,
  type Infer,
  type Schema,
  type SchemaNode,
  type WithMode,
} from "../schema.ts";
import { validate, type ValidateResult } from "../validate.ts";
import { encode, encodeArgs, decodeArgs } from "../codec.ts";
import { formModel } from "../forms.ts";
import * as schemaModule from "../schema.ts";
import * as validateModule from "../validate.ts";
import * as codecModule from "../codec.ts";
import * as formsModule from "../forms.ts";

import type * as ledger from "../../tests/goldens/ledger.ts";

// Every composite position admits the empty leaf (the #126 matrix: record
// field, variant arm, tuple element, func arg/result; vec and opt always
// did). These are positive compile-time probes — the file failing to
// type-check is the regression signal.
export const emptyInRecord = c.record({ f: c.empty, g: c.nat });
export const emptyInVariant = c.variant({ a: c.empty, b: c.nat });
export const emptyInTuple = c.tuple([c.empty, c.nat]);
export const emptyInFunc = c.func([c.empty], [c.empty], "update");
export const emptyInVec = c.vec(c.empty);
export const emptyInOpt = c.opt(c.empty);

// The generated-module shape compiles for the admitted positions, under the
// invariant annotation the equality gate rests on.
type EmptyField = { f: never; g: bigint };
export const EmptyField: Schema<EmptyField> = c.rec(() => c.record({ f: c.empty, g: c.nat }));
type EmptyTuple = [never, bigint];
export const EmptyTuple: Schema<EmptyTuple> = c.rec(() => c.tuple([c.empty, c.nat]));

// Issue #127: `VariantInfer` classifies structurally, matching the emitter,
// validator, and codec — every divergence-table row compiles under the
// invariant annotation with the classification the runtime enforces.
type EmptyArmV = { tag: "a"; value: never } | { tag: "b"; value: bigint };
export const EmptyArmV: Schema<EmptyArmV> = c.rec(() => c.variant({ a: c.empty, b: c.nat }));
type OptEmptyArmV = { tag: "a"; value: never | null } | { tag: "b"; value: bigint };
export const OptEmptyArmV: Schema<OptEmptyArmV> = c.rec(() =>
  c.variant({ a: c.opt(c.empty), b: c.nat }),
);
// An `opt null` arm carries `value` like every opt arm, and its payload is
// boxed — `{ some: null } | null` — so the arm's two inhabitants stay apart.
type OptNullArmV = { tag: "a"; value: { some: null } | null };
export const OptNullArmV: Schema<OptNullArmV> = c.rec(() => c.variant({ a: c.opt(c.null) }));
type NullArmV = { tag: "ok" } | { tag: "busy"; value: number };
export const NullArmV: Schema<NullArmV> = c.rec(() => c.variant({ ok: c.null, busy: c.nat8 }));

// Wrong classifications must stay compile errors — each probe marks an
// alias the gate must refuse.
type TagOnlyEmpty = { tag: "a" } | { tag: "b"; value: bigint };
// @ts-expect-error an empty payload carries value: never
export const tagOnlyEmpty: Schema<TagOnlyEmpty> = c.rec(() => c.variant({ a: c.empty, b: c.nat }));
type TagOnlyOptEmpty = { tag: "a" } | { tag: "b"; value: bigint };
// @ts-expect-error an opt payload always carries value
export const tagOnlyOptEmpty: Schema<TagOnlyOptEmpty> = c.rec(() =>
  c.variant({ a: c.opt(c.empty), b: c.nat }),
);
type ValuedNullArm = { tag: "ok"; value: null };
// @ts-expect-error a null payload is a bare tag
export const valuedNullArm: Schema<ValuedNullArm> = c.rec(() => c.variant({ ok: c.null }));

// The gate keeps full strength around the admitted leaf: every wrong alias
// below must stay a compile error.
type WrongFieldType = { f: null; g: bigint };
// @ts-expect-error the alias says null where the builder infers never
export const wrongFieldType: Schema<WrongFieldType> = c.rec(() =>
  c.record({ f: c.empty, g: c.nat }),
);
type MissingField = { f: never };
// @ts-expect-error the alias omits a field the builder infers
export const missingField: Schema<MissingField> = c.rec(() => c.record({ f: c.empty, g: c.nat }));
type ExtraField = { f: never; g: bigint; h: string };
// @ts-expect-error the alias adds a field the builder does not infer
export const extraField: Schema<ExtraField> = c.rec(() => c.record({ f: c.empty, g: c.nat }));
type WidenedField = { f: unknown; g: bigint };
// @ts-expect-error the alias widens never to unknown
export const widenedField: Schema<WidenedField> = c.rec(() => c.record({ f: c.empty, g: c.nat }));

function firstIssue(result: ValidateResult): { code: string; path: string } {
  assert.strictEqual(result.ok, false);
  if (result.ok) {
    throw new Error("unreachable");
  }
  const issue = result.issues[0];
  return { code: issue.code, path: issue.path };
}

test("a record with an empty field admits no value", () => {
  assert.deepStrictEqual(firstIssue(validate(emptyInRecord, { g: 5n })), {
    code: "missing_field",
    path: "$.f",
  });
  assert.deepStrictEqual(firstIssue(validate(emptyInRecord, { f: 0, g: 5n })), {
    code: "uninhabited_type",
    path: "$.f",
  });
});

test("a tuple with an empty element admits no value", () => {
  assert.deepStrictEqual(firstIssue(validate(emptyInTuple, [0, 5n])), {
    code: "uninhabited_type",
    path: "$[0]",
  });
});

test("an empty variant payload rejects every carried value", () => {
  assert.deepStrictEqual(firstIssue(validate(emptyInVariant, { tag: "a", value: 0 })), {
    code: "uninhabited_type",
    path: "$.value",
  });
});

test("a func value with empty args is still an inert reference", () => {
  const principal = "aaaaa-aa";
  assert.strictEqual(validate(emptyInFunc, { principal, method: "m" }).ok, true);
});

test("vec empty and opt empty keep their inhabited cases", () => {
  assert.strictEqual(validate(emptyInVec, []).ok, true);
  assert.strictEqual(validate(emptyInOpt, null).ok, true);
  assert.deepStrictEqual(firstIssue(validate(emptyInVec, [0])), {
    code: "uninhabited_type",
    path: "$[0]",
  });
});

// The package's own exports must keep composing: a func schema's stored
// args/results feed the codec's argument-sequence entries directly. These
// lines type-checking is the probe — a codec entry left at the narrower
// `AnySchema[]` bound rejects `FuncSchema`'s `AnyFieldSchema[]` storage for
// every func, empty-free ones included.
test("func schemas compose with the codec argument entries", () => {
  const plain = c.func([c.nat], [c.text], "update");
  const encoded = encodeArgs(plain.args, [1n]);
  assert.strictEqual(encoded.ok, true);
  if (encoded.ok) {
    const decoded = decodeArgs(plain.args, encoded.bytes);
    assert.strictEqual(decoded.ok, true);
  }
  // An empty arg admits no value, and the codec agrees at runtime — through
  // the sequence entry and the one-argument wrapper alike.
  const refused = encodeArgs(emptyInFunc.args, [0]);
  assert.strictEqual(refused.ok, false);
  assert.strictEqual(encode(c.empty, 0).ok, false);
});

// Issue #127's acceptance criterion: `validate` accepts exactly the values
// the generated static types admit, for every divergence-table row.
test("variant classification aligns validate with the static types", () => {
  // empty arm: the type says value: never — the field is demanded, and
  // nothing inhabits it.
  assert.deepStrictEqual(firstIssue(validate(EmptyArmV, { tag: "a" })), {
    code: "missing_field",
    path: "$.value",
  });
  assert.deepStrictEqual(firstIssue(validate(EmptyArmV, { tag: "a", value: 0 })), {
    code: "uninhabited_type",
    path: "$.value",
  });
  // opt empty arm: the type admits exactly { tag, value: null } — the
  // pre-fix static type said bare tag, the inversion this issue fixed.
  // A non-null value is rejected on the uninhabited inner, so "exactly"
  // holds in both directions.
  assert.strictEqual(validate(OptEmptyArmV, { tag: "a", value: null }).ok, true);
  assert.deepStrictEqual(firstIssue(validate(OptEmptyArmV, { tag: "a" })), {
    code: "missing_field",
    path: "$.value",
  });
  assert.deepStrictEqual(firstIssue(validate(OptEmptyArmV, { tag: "a", value: 0 })), {
    code: "uninhabited_type",
    path: "$.value",
  });
  // opt null arm: same alignment, with the boxed Some(null) admitted too.
  assert.strictEqual(validate(OptNullArmV, { tag: "a", value: null }).ok, true);
  assert.strictEqual(validate(OptNullArmV, { tag: "a", value: { some: null } }).ok, true);
  assert.deepStrictEqual(firstIssue(validate(OptNullArmV, { tag: "a" })), {
    code: "missing_field",
    path: "$.value",
  });
  // null arm: a bare tag; a present value stays rejected.
  assert.strictEqual(validate(NullArmV, { tag: "ok" }).ok, true);
  assert.deepStrictEqual(firstIssue(validate(NullArmV, { tag: "ok", value: null })), {
    code: "unexpected_field",
    path: "$.value",
  });
});

// `formModel` admits the empty leaf (the compile probe) and renders it as
// the uninhabited control — a UI must not offer an input — including when
// it arrives through a composite.
test("formModel renders the empty leaf uninhabited", () => {
  assert.strictEqual(formModel(c.empty).control, "uninhabited");
  const record = formModel(emptyInRecord);
  assert.strictEqual(record.control, "group");
  if (record.control === "group") {
    const f = record.fields[record.fieldLabels.indexOf("f")]();
    assert.strictEqual(f.control, "uninhabited");
  }
});

// Issue #149: every builder result assigns to `SchemaNode`, composites
// included. `Schema<in out T>` is invariant, so a union member carrying a
// *specific* domain type would refuse the ordinary builder result whose node
// it is meant to describe — a record of specific fields is not a record of
// the general field map. These are positive compile-time probes in the same
// spirit as the ones above: the file failing to type-check is the signal.
export const nodePrimitive: SchemaNode = c.nat;
export const nodeOpt: SchemaNode = c.opt(c.text);
export const nodeBoxedOpt: SchemaNode = c.opt(c.opt(c.text));
export const nodeVec: SchemaNode = c.vec(c.nat);
export const nodeBlob: SchemaNode = c.blob();
export const nodeUnit: SchemaNode = c.unit();
export const nodeRecord: SchemaNode = c.record({ owner: c.principal, balance: c.nat });
export const nodeTuple: SchemaNode = c.tuple([c.text, c.nat]);
export const nodeVariant: SchemaNode = c.variant({ ok: c.nat, err: c.text });
export const nodeFunc: SchemaNode = c.func([c.principal], [c.nat], "query");
export const nodeService: SchemaNode = c.service({ fee: c.func([], [c.nat], "query") });
export const nodeRec: SchemaNode = c.rec(() => c.record({ head: c.nat }));
// The empty leaf reaches the union inside a composite, whose own domain is
// inhabited.
export const nodeEmptyArm: SchemaNode = c.variant({ a: c.empty, b: c.nat });
// Bare, it does not — `SchemaNode` is a union of erased members, so it sits
// exactly where `AnySchema` does on the #126 corner: `any` is assignable to
// every type but `never`. `AnyFieldSchema` is the bound that re-admits it,
// which is why `resolveSchema` takes that and not this.
// @ts-expect-error a Schema<never> is not an erased node
export const nodeEmpty: SchemaNode = c.empty;

// And `resolveSchema` narrows back out of it: the walk a consumer writes
// over a resolved node needs no cast and no unreachable `rec` case.
test("a resolved node narrows without casts", () => {
  const node = resolveSchema(c.rec(() => c.record({ owner: c.principal })));
  assert.strictEqual(node.kind, "record");
  if (node.kind !== "record") {
    return;
  }
  assert.deepStrictEqual(Object.keys(node.fields), ["owner"]);
  assert.strictEqual(resolveSchema(node.fields.owner).kind, "primitive");
});

// The generated call interface, proven against a hand-written one. A module
// generated from a contract with an actor exports the type `Actor` — one
// async method per service method, zero results resolving to `void`, one to
// the value, several to a tuple, each intersected with its mode (#244) — as
// reviewed generator output for whatever call layer a consumer builds on it. `Equals` is the same invariance trick
// the goldens rest on, so a drift in the emitted interface turns this file
// red under tsc.
type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

interface ExpectedLedgerActor {
  fee: (() => Promise<ledger.Tokens>) & WithMode<"query">;
  decimals: (() => Promise<number>) & WithMode<"query">;
  name: (() => Promise<string>) & WithMode<"query">;
  balance_of: ((arg0: ledger.Account) => Promise<ledger.Tokens>) & WithMode<"query">;
  get_transactions: ((arg0: {
    start: bigint;
    length: bigint;
  }) => Promise<ledger.TransactionsResponse>) &
    WithMode<"query">;
  transfer: ((arg0: ledger.TransferArg) => Promise<ledger.TransferResult>) & WithMode<"update">;
  total_supply: (() => Promise<ledger.Tokens>) & WithMode<"query">;
  symbol: (() => Promise<string>) & WithMode<"query">;
}

export const actorTypeMatchesHandWrittenInterface: Equals<ledger.Actor, ExpectedLedgerActor> = true;

// Boxed options: an `opt` whose inner domain admits `null` — another opt,
// `null`, `reserved` — infers `{ some: T } | null`; every other opt infers
// `T | null`, `opt empty` included. `Equal` is exact (mutual assignability
// is not enough for `any`/`never` corners), and each probe below is a
// compile-time assertion: the harness fails to type-check if one drifts.
type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

const optOptNat = c.opt(c.opt(c.nat));
const optNull = c.opt(c.null);
const optReserved = c.opt(c.reserved);
const optEmpty = c.opt(c.empty);
const optNat = c.opt(c.nat);
const optOptEmpty = c.opt(c.opt(c.empty));
const optOptOptNat = c.opt(c.opt(c.opt(c.nat)));
export const boxedProbes: [
  Equal<Infer<typeof optOptNat>, { some: bigint | null } | null>,
  Equal<Infer<typeof optNull>, { some: null } | null>,
  Equal<Infer<typeof optReserved>, { some: unknown } | null>,
  Equal<Infer<typeof optEmpty>, null>,
  Equal<Infer<typeof optNat>, bigint | null>,
  Equal<Infer<typeof optOptEmpty>, { some: null } | null>,
  Equal<Infer<typeof optOptOptNat>, { some: { some: bigint | null } | null } | null>,
] = [true, true, true, true, true, true, true];

// The same agreement under the invariant annotation generated modules use,
// through `c.rec` recursion: `type Chain = opt Chain` and mutual recursion
// where one side boxes and the other does not.
type Chain = { some: Chain } | null;
export const Chain: Schema<Chain> = c.rec(() => c.opt(Chain));
type Ping = { pong: Pong } | null;
type Pong = { some: Ping } | null;
export const Ping: Schema<Ping> = c.rec(() => c.opt(c.record({ pong: Pong })));
export const Pong: Schema<Pong> = c.rec(() => c.opt(Ping));

// And the wrong readings stay compile errors.
// @ts-expect-error opt opt collapses to T | null no longer
export const collapsedOptOpt: Schema<bigint | null> = c.rec(() => c.opt(c.opt(c.nat)));
// @ts-expect-error opt empty does not box
export const boxedOptEmpty: Schema<{ some: never } | null> = c.rec(() => c.opt(c.empty));
// @ts-expect-error opt nat does not box
export const boxedOptNat: Schema<{ some: bigint } | null> = c.rec(() => c.opt(c.nat));
type UnboxedChain = { next: UnboxedChain } | null;
// @ts-expect-error a recursive opt of an opt boxes too
export const unboxedChain: Schema<UnboxedChain> = c.rec(() => c.opt(unboxedChain));

test("validate agrees with the boxed static types", () => {
  const cases: readonly [unknown, boolean][] = [
    [null, true],
    [{ some: null }, true],
    [{ some: 5n }, true],
    [5n, false],
    [{ some: 5 }, false],
  ];
  for (const [value, ok] of cases) {
    assert.strictEqual(validate(optOptNat, value).ok, ok, JSON.stringify(String(value)));
  }
  assert.strictEqual(validate(Chain, { some: { some: null } }).ok, true);
  assert.strictEqual(validate(Chain, { some: {} }).ok, false);
  assert.strictEqual(validate(Pong, { some: { pong: { some: null } } }).ok, true);
  assert.strictEqual(validate(Pong, { some: { pong: null } }).ok, true);
  assert.strictEqual(validate(optEmpty, null).ok, true);
  assert.strictEqual(validate(optEmpty, { some: null }).ok, false);
});

// The boxed-option rule has one public statement, `isBoxedOpt`; the walkers
// that decide on an inner node they already resolved under their own budget
// (validate, the codec) keep module-local copies, and the form model calls
// `isBoxedOpt` itself. This pins every walker to `isBoxedOpt` over inner
// kinds on both sides of the rule, through `rec` and aliases: an empty
// object reports `missing_field` at `$.some` exactly when the opt boxes, and
// the form model's `boxed` flag says the same.
test("every walker boxes exactly when isBoxedOpt says so", () => {
  const Inner = c.rec(() => c.opt(c.nat));
  const NullAlias = c.rec(() => c.null);
  const inners: readonly AnySchema[] = [
    c.nat,
    c.text,
    c.opt(c.nat),
    c.opt(c.empty),
    c.null,
    c.reserved,
    c.empty as AnySchema,
    c.record({ a: c.nat }),
    c.unit(),
    c.variant({}) as AnySchema,
    Inner,
    NullAlias,
    c.rec(() => c.rec(() => c.reserved)),
    c.rec(() => c.text),
  ];
  let boxedSeen = 0;
  for (const inner of inners) {
    const schema = c.opt(inner) as AnySchema;
    const boxed = isBoxedOpt(schema);
    boxedSeen += boxed ? 1 : 0;
    const validated = validate(schema as Schema<unknown>, {});
    const encoded = encode(schema as Schema<unknown>, {});
    const atSome = (result: { ok: boolean; issues?: readonly { code: string; path: string }[] }) =>
      !result.ok &&
      result.issues?.[0].code === "missing_field" &&
      result.issues[0].path === "$.some";
    assert.strictEqual(
      atSome(validated),
      boxed,
      `validate, inner ${String(resolveSchema(inner).kind)}`,
    );
    assert.strictEqual(
      atSome(encoded),
      boxed,
      `encode, inner ${String(resolveSchema(inner).kind)}`,
    );
    const form = formModel(schema);
    assert(form.control === "optional");
    if (form.control === "optional") {
      assert.strictEqual(form.boxed, boxed);
    }
  }
  assert.strictEqual(boxedSeen, 7, "both sides of the rule are exercised");
});

// The rule's published surface is `isBoxedOpt` (and the `OptDomain` type):
// the predicate the walkers share is not an export of any subpath.
test("the node-level predicate stays out of every module's exports", () => {
  for (const module of [schemaModule, validateModule, codecModule, formsModule]) {
    assert.strictEqual("admitsNull" in module, false);
  }
  assert.strictEqual(typeof schemaModule.isBoxedOpt, "function");
});
