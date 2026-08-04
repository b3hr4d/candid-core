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
import * as proto from "../../tests/goldens/proto.ts";
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
    proto,
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
    // The node's own path must be the issue's path verbatim — the quoted
    // rendering included, or the two grammars have diverged.
    assert.strictEqual(node?.path, weirdResult.issues[0].path);
  }
});

// ---------------------------------------------------------------------------
// Review-cycle regressions (pre-publication adversarial review)
// ---------------------------------------------------------------------------

test("a self-referential rec chain throws instead of hanging", () => {
  const evil = c.rec(() => evil as unknown as Schema<unknown>) as AnySchema;
  assert.throws(() => formNodeAt(formModel(evil), "$"), TypeError);
  assert.throws(() => formNodeAt(formModel(c.record({ x: evil })), "$.x.y"), TypeError);
});

test("variant and func issue paths map onto form nodes", () => {
  const model = formModel(ledger.TransferResult as AnySchema);
  // A deep issue inside a chosen arm resolves through unambiguous arms.
  const bad = validate(ledger.TransferResult as Schema<unknown>, {
    tag: "err",
    value: { tag: "bad_fee", value: { expected_fee: { e8s: "wrong" } } },
  });
  assert(!bad.ok);
  if (!bad.ok) {
    const node = formNodeAt(model, bad.issues[0].path);
    assert.strictEqual(node?.control, "bigint");
    assert.strictEqual(node?.path, bad.issues[0].path);
  }
  // A bad tag addresses the re-pathed choice.
  const badTag = validate(ledger.TransferResult as Schema<unknown>, { tag: "nope" });
  assert(!badTag.ok);
  if (!badTag.ok) {
    const node = formNodeAt(model, badTag.issues[0].path);
    assert.strictEqual(node?.control, "choice");
    assert.strictEqual(node?.path, badTag.issues[0].path);
  }
  // A missing payload addresses the re-pathed choice at $.value.
  const noValue = validate(ledger.TransferResult as Schema<unknown>, { tag: "ok" });
  assert(!noValue.ok);
  if (!noValue.ok) {
    const node = formNodeAt(model, noValue.issues[0].path);
    assert.strictEqual(node?.control, "choice");
    assert.strictEqual(node?.path, noValue.issues[0].path);
  }
  // Func references expose their two editors at the paths validate uses.
  const funcModel = formModel(deferred.Callback as AnySchema);
  const badFunc = validate(deferred.Callback as Schema<unknown>, {
    principal: 42,
    method: "",
  });
  assert(!badFunc.ok);
  if (!badFunc.ok) {
    for (const issue of badFunc.issues) {
      const node = formNodeAt(funcModel, issue.path);
      assert(node !== undefined, `${issue.path} must address a node`);
      assert.strictEqual(node?.path, issue.path);
    }
    assert.strictEqual(formNodeAt(funcModel, "$.principal")?.control, "principal");
    assert.strictEqual(formNodeAt(funcModel, "$.method")?.control, "text");
  }
});

test("a numeric-tagged arm's payload carries the numeric id too", () => {
  const model = resolveLazy(formModel(c.variant({ _5_: c.nat8 })));
  assert.strictEqual(model.control, "choice");
  if (model.control === "choice") {
    const payload = model.arms[0].payload();
    assert(payload !== undefined);
    assert.strictEqual(payload?.numericId, 5);
    // And the positive tagOnly direction: a non-null primitive payload is
    // NOT tag-only.
    assert.strictEqual(model.arms[0].tagOnly, false);
  }
});

test("prototype-hazard keys resolve through the model", () => {
  const model = formModel(proto.Holder as AnySchema);
  const result = validate(proto.Holder as Schema<unknown>, JSON.parse('{"__proto__": true}'));
  assert(!result.ok);
  if (!result.ok) {
    const node = formNodeAt(model, result.issues[0].path);
    assert.strictEqual(node?.control, "integer");
    assert.strictEqual(node?.path, result.issues[0].path);
  }
});

test("every fixed-width range and bigint bound is exactly the domain's", () => {
  const expectations: readonly [AnySchema, number, number][] = [
    [c.nat8, 0, 255],
    [c.nat16, 0, 65_535],
    [c.nat32, 0, 4_294_967_295],
    [c.int8, -128, 127],
    [c.int16, -32_768, 32_767],
    [c.int32, -2_147_483_648, 2_147_483_647],
  ];
  for (const [schema, min, max] of expectations) {
    const node = resolveLazy(formModel(schema));
    assert.strictEqual(node.control, "integer");
    if (node.control === "integer") {
      assert.strictEqual(node.min, min);
      assert.strictEqual(node.max, max);
    }
  }
  const int64 = resolveLazy(formModel(c.int64));
  assert.strictEqual(int64.control, "bigint");
  if (int64.control === "bigint") {
    assert.strictEqual(int64.min, -9_223_372_036_854_775_808n);
    assert.strictEqual(int64.max, 9_223_372_036_854_775_807n);
  }
  const int = resolveLazy(formModel(c.int));
  assert.strictEqual(int.control, "bigint");
  if (int.control === "bigint") {
    assert.strictEqual(int.min, undefined);
    assert.strictEqual(int.max, undefined);
  }
});

test("the path resolver's remaining branches: indices, opts, and dead ends", () => {
  // The opt same-path rule: the inner editor sits at the field's own path.
  const account = formModel(ledger.Account as AnySchema);
  const viaOpt = formNodeAt(account, "$.subaccount");
  assert.strictEqual(viaOpt?.control, "optional");
  assert.strictEqual(viaOpt?.path, "$.subaccount");
  const opt = resolveLazy(account);
  if (opt.control === "group") {
    const subaccount = resolveLazy(opt.fields[1]());
    if (subaccount.control === "optional") {
      assert.strictEqual(resolveLazy(subaccount.inner()).path, "$.subaccount");
    }
  }
  // List and tuple indices.
  const items = formModel(collections.Items as AnySchema);
  assert.strictEqual(formNodeAt(items, "$[2].id")?.control, "integer");
  const pair = formModel(collections.Pair as AnySchema);
  assert.strictEqual(formNodeAt(pair, "$[1]")?.control, "text");
  assert.strictEqual(formNodeAt(pair, "$[7]"), undefined);
  // A wrong-typed opt value reports at the opt's own path and maps to the
  // optional node itself...
  const atOpt = validate(ledger.Account as Schema<unknown>, {
    owner: { toText: () => "aaaaa-aa" },
    subaccount: [1, 2],
  });
  assert(!atOpt.ok);
  if (!atOpt.ok) {
    assert.strictEqual(formNodeAt(account, atOpt.issues[0].path)?.control, "optional");
  }
  // ...while an issue INSIDE a present opt resolves through the wrapper.
  const transaction = formModel(ledger.Transaction as AnySchema);
  const deep = validate(ledger.Transaction as Schema<unknown>, {
    to: { owner: 42, subaccount: null },
    fee: null,
    from: null,
    memo: null,
    timestamp: 1n,
    index: 2n,
    amount: { e8s: 3n },
  });
  assert(!deep.ok);
  if (!deep.ok) {
    assert.strictEqual(deep.issues[0].path, "$.to.owner");
    assert.strictEqual(formNodeAt(transaction, deep.issues[0].path)?.control, "principal");
  }
  // Dead ends stay undefined: unknown keys, malformed paths, wrong roots.
  assert.strictEqual(formNodeAt(account, "$.nope"), undefined);
  assert.strictEqual(formNodeAt(account, "$..broken"), undefined);
  assert.strictEqual(formNodeAt(account, "nope"), undefined);
});

test("PR #122 review: value-named fields and malformed escapes", () => {
  // A single-arm variant whose payload has a field literally named "value":
  // $.value is the payload, $.value.value is that field.
  const single = formModel(c.variant({ a: c.record({ value: c.nat8 }) }));
  const singleResult = validate(c.variant({ a: c.record({ value: c.nat8 }) }), {
    tag: "a",
    value: { value: 999 },
  } as never);
  assert(!singleResult.ok);
  if (!singleResult.ok) {
    assert.strictEqual(singleResult.issues[0].path, "$.value.value");
    const node = formNodeAt(single, singleResult.issues[0].path);
    assert.strictEqual(node?.control, "integer");
    assert.strictEqual(node?.path, "$.value.value");
  }
  // Multi-arm variants still resolve deep nested-variant paths (pinned
  // above) — and a malformed quoted escape is undefined, never a throw.
  const account = formModel(ledger.Account as AnySchema);
  assert.strictEqual(formNodeAt(account, '$["\\x"]'), undefined);
});
