// The issue #105 acceptance gates: every combinator yields a documented
// control, recursion stays lazy and expands on demand, the ledger worked
// example pins labels/ranges/choices, and validation issues address form
// nodes through the shared path grammar.

import { test } from "node:test";
import assert from "node:assert/strict";

import { formModel, formNodeAt, type FormNode } from "../forms.ts";
import { validate } from "../validate.ts";
import { c, type AnySchema, type Schema } from "../schema.ts";

import * as primitives from "../../tests/goldens/primitives.ts";
import * as collections from "../../tests/goldens/collections.ts";
import * as variants from "../../tests/goldens/variants.ts";
import * as recursion from "../../tests/goldens/recursion.ts";
import * as quoting from "../../tests/goldens/quoting.ts";
import * as deferred from "../../tests/goldens/deferred.ts";
import * as ledger from "../../tests/goldens/ledger.ts";

function resolveLazy(node: FormNode): FormNode {
  let current = node;
  while (current.control === "lazy") {
    current = current.expand();
  }
  return current;
}

test("every golden schema yields a form model without gaps", () => {
  const modules: Record<string, Record<string, unknown>> = {
    primitives,
    collections,
    variants,
    recursion,
    quoting,
    deferred,
    ledger,
  };
  let built = 0;
  for (const name of Object.keys(modules)) {
    for (const declaration of Object.keys(modules[name])) {
      const model = formModel(modules[name][declaration] as AnySchema);
      // Every root is a lazy node (declarations are rec-wrapped) that
      // expands to a real control.
      const resolved = resolveLazy(model);
      assert(typeof resolved.control === "string");
      assert.strictEqual(resolved.path, "$");
      built += 1;
    }
  }
  assert(built > 40, `the sweep must cover the corpus (${built})`);
});

test("recursion is lazy and expands level by level", () => {
  // List = opt record { head : nat; tail : List } — three levels deep, each
  // expansion on demand, none eager.
  const root = resolveLazy(formModel(recursion.List as AnySchema));
  assert.strictEqual(root.control, "optional");
  if (root.control !== "optional") {
    return;
  }
  let group = resolveLazy(root.inner());
  for (let level = 0; level < 3; level += 1) {
    assert.strictEqual(group.control, "group");
    if (group.control !== "group") {
      return;
    }
    assert.deepStrictEqual(group.fieldLabels, ["head", "tail"]);
    const tail = resolveLazy(group.fields[1]());
    assert.strictEqual(tail.control, "optional");
    if (tail.control !== "optional") {
      return;
    }
    group = resolveLazy(tail.inner());
  }
  assert.strictEqual(group.control, "group", "level 3 still expands");
});

test("the ledger worked example: labels, ranges, and variant choices", () => {
  // Account: a principal owner and an optional blob subaccount.
  const account = resolveLazy(formModel(ledger.Account as AnySchema));
  assert.strictEqual(account.control, "group");
  if (account.control !== "group") {
    return;
  }
  assert.deepStrictEqual(account.fieldLabels, ["owner", "subaccount"]);
  const owner = resolveLazy(account.fields[0]());
  assert.strictEqual(owner.control, "principal");
  assert.strictEqual(owner.label, "owner");
  assert.strictEqual(owner.path, "$.owner");
  const subaccount = resolveLazy(account.fields[1]());
  assert.strictEqual(subaccount.control, "optional");
  if (subaccount.control === "optional") {
    assert.strictEqual(resolveLazy(subaccount.inner()).control, "bytes");
  }

  // Tokens: a nat64 — bigint-flagged with exact bounds, never a number input.
  const tokens = resolveLazy(formModel(ledger.Tokens as AnySchema));
  assert.strictEqual(tokens.control, "group");
  if (tokens.control === "group") {
    const e8s = resolveLazy(tokens.fields[0]());
    assert.strictEqual(e8s.control, "bigint");
    if (e8s.control === "bigint") {
      assert.strictEqual(e8s.min, 0n);
      assert.strictEqual(e8s.max, 18_446_744_073_709_551_615n);
    }
  }

  // TransferError: a choice whose tag-only arms need no payload editor.
  const transferError = resolveLazy(formModel(ledger.TransferError as AnySchema));
  assert.strictEqual(transferError.control, "choice");
  if (transferError.control !== "choice") {
    return;
  }
  const byTag = new Map(transferError.arms.map((arm) => [arm.tag, arm]));
  assert(byTag.get("too_old")?.tagOnly);
  assert.strictEqual(byTag.get("too_old")?.payload(), undefined);
  const badFee = byTag.get("bad_fee");
  assert(badFee !== undefined && !badFee.tagOnly);
  const badFeePayload = badFee?.payload();
  assert(badFeePayload !== undefined);
  if (badFeePayload !== undefined) {
    const payload = resolveLazy(badFeePayload);
    assert.strictEqual(payload.control, "group");
    assert.strictEqual(payload.path, "$.value");
  }
});

test("fixed-width integers carry number-safe ranges; floats carry width", () => {
  const int8 = resolveLazy(formModel(c.int8));
  assert.strictEqual(int8.control, "integer");
  if (int8.control === "integer") {
    assert.strictEqual(int8.min, -128);
    assert.strictEqual(int8.max, 127);
  }
  const float32 = resolveLazy(formModel(c.float32));
  assert.strictEqual(float32.control, "float");
  if (float32.control === "float") {
    assert.strictEqual(float32.bits, 32);
  }
  const nat = resolveLazy(formModel(c.nat));
  assert.strictEqual(nat.control, "bigint");
  if (nat.control === "bigint") {
    assert.strictEqual(nat.min, 0n);
    assert.strictEqual(nat.max, undefined);
  }
});

test("unnamed labels surface the _id_ rendering with the numeric id apart", () => {
  const numbered = resolveLazy(formModel(variants.Numbered as AnySchema));
  assert.strictEqual(numbered.control, "choice");
  if (numbered.control !== "choice") {
    return;
  }
  const tags = numbered.arms.map((arm) => [arm.tag, arm.numericId]);
  assert.deepStrictEqual(tags, [
    ["_0_", 0],
    ["_5_", 5],
  ]);
  // And a record field: Pair is a tuple, so use a synthetic record.
  const synthetic = resolveLazy(formModel(c.record({ _7_: c.bool })));
  assert.strictEqual(synthetic.control, "group");
  if (synthetic.control === "group") {
    const field = resolveLazy(synthetic.fields[0]());
    assert.strictEqual(field.label, "_7_");
    assert.strictEqual(field.numericId, 7);
  }
});

test("func and service values render as reference controls", () => {
  assert.strictEqual(
    resolveLazy(formModel(deferred.Callback as AnySchema)).control,
    "funcReference",
  );
  assert.strictEqual(
    resolveLazy(formModel(deferred.Registry as AnySchema)).control,
    "serviceReference",
  );
});

test("validation issues address form nodes through the shared path grammar", () => {
  const schema = ledger.TransferArg as AnySchema;
  const model = formModel(schema);
  const invalid = {
    to: { owner: { toText: () => "aaaaa-aa" }, subaccount: null },
    fee: null,
    memo: null,
    from_subaccount: null,
    created_at_time: null,
    amount: { e8s: -1n },
  };
  const result = validate(schema as Schema<unknown>, invalid);
  assert(!result.ok);
  if (!result.ok) {
    for (const issue of result.issues) {
      const node = formNodeAt(model, issue.path);
      assert(node !== undefined, `${issue.path} must address a form node`);
      assert.strictEqual(node?.path, issue.path);
    }
    assert.strictEqual(
      formNodeAt(model, result.issues[0].path)?.control,
      "bigint",
      "the out-of-range e8s addresses its bigint control",
    );
  }
  // Quoted keys travel the same grammar.
  const weird = formModel(quoting.Weird as AnySchema);
  const weirdResult = validate(
    quoting.Weird as Schema<unknown>,
    { 'quote"mark': 1, naïve: "x", "has space": 1n } as never,
  );
  assert(!weirdResult.ok);
  if (!weirdResult.ok) {
    const node = formNodeAt(weird, weirdResult.issues[0].path);
    assert.strictEqual(node?.control, "checkbox");
  }
});
