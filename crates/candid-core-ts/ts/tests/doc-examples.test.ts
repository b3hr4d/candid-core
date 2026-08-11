// The gate behind the `@example` blocks in the shipped documentation.
//
// JSDoc flows into `dist/*.d.ts` at build time, so an example is not a
// comment a reader can shrug off — it is the first code a consumer meets in
// editor hover, and a wrong one costs more than no example at all. Two
// things make these verifiable rather than decorative:
//
// - every example line is mirrored verbatim below, inside a scoped block, so
//   `tsc --noEmit` compiles it against the real declarations. An example
//   naming a member that does not exist, passing the wrong arity, or putting
//   a `number` where a `bigint` is required is a red type-check, not a bad
//   hover.
// - the first test reads the shipped sources back and asserts that mirroring
//   is complete. Editing an example without updating its mirror fails here,
//   which is what keeps the compiled copy from drifting away from the
//   published one.
//
// The mirrors are deliberately dumb copies: no refactoring, no shared
// helpers, no tidying, and no assertions folded in — a mirror that is
// "improved" no longer proves anything about the text a consumer reads. The
// claims those examples' trailing comments make are asserted separately,
// below the mirrors. Preamble bindings an example leaves to context (the
// principal it names) are declared inside its block.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  c,
  resolveSchema,
  serviceMethods,
  type AnyFieldSchema,
  type AnySchema,
  type FieldSchemas,
  type FuncValue,
  type Infer,
  type MethodMode,
  type ResolvedNode,
  type Schema,
  type SchemaNode,
  type ServiceMethod,
} from "../schema.ts";
import { formModel } from "../forms.ts";
import {
  isResultSchema,
  unwrapResult,
  type ResultErr,
  type ResultOk,
  type UnwrapResult,
} from "../validate.ts";

// Every module whose JSDoc ships. schema.ts carries the examples today; the
// others are listed so an example added to one of them is covered the same
// way rather than silently uncompiled.
const SHIPPED_SOURCES = [
  "../schema.ts",
  "../validate.ts",
  "../contract.ts",
  "../codec.ts",
  "../actor.ts",
  "../forms.ts",
  "../labels.ts",
];

/** The lines of every `@example` block: from the tag to the block's end. */
function exampleLines(source: string): string[] {
  const lines: string[] = [];
  let inExample = false;
  for (const raw of source.split("\n")) {
    const trimmed = raw.trim();
    if (trimmed === "*/") {
      inExample = false;
    } else if (trimmed === "* @example") {
      inExample = true;
    } else if (inExample) {
      lines.push(trimmed.replace(/^\*\s?/, ""));
    }
  }
  return lines;
}

test("every shipped @example is mirrored verbatim in this file", () => {
  const mirror = readFileSync(new URL(import.meta.url), "utf8");
  const examples = SHIPPED_SOURCES.flatMap((module) =>
    exampleLines(readFileSync(new URL(module, import.meta.url), "utf8")),
  );
  assert(examples.length > 0, "no @example blocks found — the extractor is broken");
  const missing = examples.filter((line) => line.length === 0 || !mirror.includes(line));
  assert.deepStrictEqual(
    missing,
    [],
    `these examples are not mirrored below, so nothing compiles them:\n  ${missing.join("\n  ")}`,
  );
});

// --- Mirrors: Schema, Infer, and the node interfaces ----------------------

{
  const Balance: Schema<bigint> = c.nat;
  void Balance;
}

{
  const Account = c.record({ owner: c.principal, balance: c.nat });
  type Account = Infer<typeof Account>; // { owner: Principal; balance: bigint }
  void Account;
}

{
  c.nat64.primitive; // "nat64"
  c.opt(c.text).inner; // the `text` schema underneath
  c.vec(c.nat).inner; // the element schema
  c.blob().kind; // "blob"
  c.unit().kind; // "unit"
  c.record({ balance: c.nat }).fields.balance; // the field's own schema
  c.tuple([c.text, c.nat]).elements.length; // 2
  c.variant({ ok: c.nat, err: c.text }).arms.ok; // the `ok` payload schema
  c.rec(() => c.nat).body(); // resolves the thunk to the `nat` schema
  c.func([c.principal], [c.nat], "query").mode; // "query"
  c.service({ balance: c.func([], [c.nat], "query") }).methods.balance;
}

{
  const byName: Record<string, AnySchema> = { Account: c.record({ id: c.nat }) };
  void byName;
}

{
  const args: readonly AnyFieldSchema[] = [c.text, c.empty];
  void args;
}

{
  const fields: FieldSchemas = { owner: c.principal, balance: c.nat };
  void fields;
}

{
  const archive = { toText: () => "aaaaa-aa" };
  const nextPage: FuncValue = { principal: archive, method: "get_blocks" };
  void nextPage;
}

{
  const mode: MethodMode = "composite_query";
  void mode;
}

// --- Mirrors: the `c` builders --------------------------------------------

{
  const Account = c.record({ owner: c.principal, balance: c.nat });
  void Account;
}

{
  c.variant({ ok: c.null }); // { tag: "ok" }
  c.record({ active: c.bool }); // { active: boolean }
  c.record({ memo: c.text }); // { memo: string }
  c.record({ owner: c.principal }); // { owner: Principal }
  c.variant({ ok: c.text, never: c.empty });
  c.record({ memo: c.opt(c.text) }); // { memo: string | null }
  c.vec(c.nat32); // number[]
  c.record({ payload: c.blob() }); // { payload: Uint8Array }
  c.record({ ack: c.unit() }); // { ack: Record<string, never> }
  c.record({ owner: c.principal, balance: c.nat });
  c.tuple([c.text, c.nat]); // [string, bigint]
  c.variant({ ok: c.nat, err: c.text, pending: c.null });
  // { tag: "ok"; value: bigint } | { tag: "err"; value: string } | { tag: "pending" }
  c.func([c.principal], [c.nat], "query");
  c.service({ balance: c.func([c.principal], [c.nat], "query") });
}

{
  const balance: Infer<typeof c.nat> = 12_345_678_901_234_567_890n;
  const delta: Infer<typeof c.int> = -42n;
  const byte: Infer<typeof c.nat8> = 255;
  const port: Infer<typeof c.nat16> = 8080;
  const height: Infer<typeof c.nat32> = 4_294_967_295;
  const e8s: Infer<typeof c.nat64> = 100_000_000n;
  const offset: Infer<typeof c.int8> = -128;
  const trim: Infer<typeof c.int16> = -32_768;
  const drift: Infer<typeof c.int32> = -2_147_483_648;
  const nanos: Infer<typeof c.int64> = -9_223_372_036_854_775_808n;
  const ratio: Infer<typeof c.float32> = Math.fround(0.1);
  const rate: Infer<typeof c.float64> = 0.1;
  const anything: Infer<typeof c.reserved> = { whatever: true };
  void [balance, delta, byte, port, height, e8s, offset, trim, drift, nanos, ratio, rate, anything];
}

{
  type List = { head: bigint; tail: List | null };
  const List: Schema<List> = c.rec(() => c.record({ head: c.nat, tail: c.opt(List) }));
  void List;
}

// --- Mirrors: reading a schema back ---------------------------------------

{
  const node: SchemaNode = c.record({ balance: c.nat });
  if (node.kind === "record") node.fields; // narrowed to the field map
  void node;
}

{
  const node: ResolvedNode = resolveSchema(c.rec(() => c.text));
  void node;
}

{
  const method: ServiceMethod = { name: "fee", mode: "query", args: [], results: [c.nat] };
  void method;
}

{
  resolveSchema(c.rec(() => c.nat)).kind; // "primitive"
}

{
  const table = serviceMethods(c.service({ fee: c.func([], [c.nat], "query") }));
  table.get("fee")?.mode; // "query"
}

// --- Mirrors: result unwrapping -------------------------------------------

{
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  type Balance = ResultOk<typeof Transfer>; // bigint
  void Transfer;
}

{
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  type Failure = ResultErr<typeof Transfer>; // string
  void Transfer;
}

{
  const outcome: UnwrapResult<bigint, string> = { ok: true, value: 5n };
  outcome.issues === undefined; // readable on every member, no type guard
}

{
  isResultSchema(c.variant({ Ok: c.nat, Err: c.text })); // true
  isResultSchema(c.record({ ok: c.nat, err: c.text })); // false
}

{
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  const outcome = unwrapResult(Transfer, { tag: "ok", value: 5n });
  outcome.ok === true; // and outcome.value is 5n, typed bigint
}

// --- Mirrors: the form model ----------------------------------------------

{
  type Account = { balance: bigint };
  const Account: Schema<Account> = c.rec(() => c.record({ balance: c.nat }));
  let node = formModel(Account); // a rec-wrapped declaration: control "lazy"
  while (node.control === "lazy") node = node.expand();
  void node;
}

// --- The claims those examples' trailing comments make --------------------

test("the node-interface examples read exactly what the comment claims", () => {
  assert.strictEqual(c.nat64.primitive, "nat64");
  assert.strictEqual(c.opt(c.text).inner, c.text);
  assert.strictEqual(c.vec(c.nat).inner, c.nat);
  assert.strictEqual(c.blob().kind, "blob");
  assert.strictEqual(c.unit().kind, "unit");
  assert.strictEqual(c.record({ balance: c.nat }).fields.balance, c.nat);
  assert.strictEqual(c.tuple([c.text, c.nat]).elements.length, 2);
  assert.strictEqual(c.variant({ ok: c.nat, err: c.text }).arms.ok, c.nat);
  assert.strictEqual(c.rec(() => c.nat).body(), c.nat);
  assert.strictEqual(c.func([c.principal], [c.nat], "query").mode, "query");
  assert(c.service({ balance: c.func([], [c.nat], "query") }).methods.balance !== undefined);
});

test("the introspection examples read exactly what the comment claims", () => {
  assert.strictEqual(resolveSchema(c.rec(() => c.nat)).kind, "primitive");
  const table = serviceMethods(c.service({ fee: c.func([], [c.nat], "query") }));
  assert.strictEqual(table.get("fee")?.mode, "query");
  const Account: Schema<{ balance: bigint }> = c.rec(() => c.record({ balance: c.nat }));
  assert.strictEqual(formModel(Account).control, "lazy");
});

test("the result examples read exactly what the comment claims", () => {
  assert.strictEqual(isResultSchema(c.variant({ Ok: c.nat, Err: c.text })), true);
  assert.strictEqual(isResultSchema(c.record({ ok: c.nat, err: c.text })), false);
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  const outcome = unwrapResult(Transfer, { tag: "ok", value: 5n });
  assert.strictEqual(outcome.ok, true);
  assert.strictEqual(outcome.value, 5n);
  // The union's own claim: every member declares all three keys, so this
  // reads on the success member rather than needing a guard first.
  const declared: UnwrapResult<bigint, string> = { ok: true, value: 5n };
  assert.strictEqual(declared.issues, undefined);
});

test("the builder examples build the nodes their comments describe", () => {
  assert.strictEqual(c.variant({ ok: c.null }).arms.ok, c.null);
  assert.strictEqual(c.record({ active: c.bool }).fields.active, c.bool);
  assert.strictEqual(c.record({ memo: c.opt(c.text) }).fields.memo.kind, "opt");
  assert.strictEqual(c.vec(c.nat32).inner, c.nat32);
  assert.strictEqual(c.record({ payload: c.blob() }).fields.payload.kind, "blob");
  assert.strictEqual(c.record({ ack: c.unit() }).fields.ack.kind, "unit");
  assert.strictEqual(c.tuple([c.text, c.nat]).kind, "tuple");
  assert.strictEqual(c.func([c.principal], [c.nat], "query").args.length, 1);
  assert.strictEqual(c.service({ balance: c.func([], [c.nat], "query") }).kind, "service");
  assert.deepStrictEqual(Object.keys(c.variant({ ok: c.nat, err: c.text, pending: c.null }).arms), [
    "ok",
    "err",
    "pending",
  ]);
});
