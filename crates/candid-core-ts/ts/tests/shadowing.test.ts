// Issue #188: a generated module binds every declaration as a `$`-prefixed
// local and exports it under its Candid name, so a declaration may be named
// after the schema runtime's bindings (`c`, `Schema`, `PrincipalValue`), an
// ambient type the lowerings reference (`Array`, `Record`, `Uint8Array`,
// `Promise`), or a TypeScript reserved word (`delete`, `string`, `default`).
// Before, the generator refused every one of them (#116, #130).
//
// The tsc equality gate already proves the `shadowing` golden compiles with
// each builder equal to its alias. This suite is the other half of the
// acceptance: the module loads under Node, and a consumer reaches every
// declaration through its export name — renamed on import where the name is
// a reserved word or would shadow a global in the consumer's own scope.

import { test } from "node:test";
import assert from "node:assert/strict";

import { decode, encode } from "../codec.ts";
import { serviceMethods, type PrincipalValue as RuntimePrincipal } from "../schema.ts";
import { validate } from "../validate.ts";

import * as shadowing from "../../tests/goldens/shadowing.ts";
import Default, {
  Array as ArraySchema,
  Bytes,
  PrincipalValue as PrincipalRecord,
  Promise as PromiseSchema,
  Record as RecordSchema,
  Schema as SchemaDecl,
  Texts,
  Uint8Array as Uint8ArraySchema,
  Unit,
  Uses,
  actor,
  c as C,
  delete as Delete,
  string as StringSchema,
  type Actor,
} from "../../tests/goldens/shadowing.ts";

const principal: RuntimePrincipal = { toText: () => "aaaaa-aa" };

// Type level: each export carries its reviewed alias, and the ambient types
// the lowerings reference kept their global meaning inside the module.
const texts: Texts = ["a", "b"] satisfies string[];
const bytes: Bytes = new Uint8Array([1, 2]);
const unit: Unit = {};
const uses: Uses = {
  a: 7,
  s: -3,
  d: 1.5,
  p: { id: 42n },
  w: { p: principal },
  x: -1,
  t: texts,
};
const record: RecordSchema = 1n;
const u8: Uint8ArraySchema = 0.5;
const def: Default = 9;
const str: StringSchema = 16;
// The `Actor` interface uses the global `Promise`, beside a declaration that
// is exported as `Promise`.
const call: Actor["ping"] = async (_arg0: C) => ({ id: 1n });
void call;

test("every former binding name is exported and loads under Node", () => {
  assert.deepStrictEqual(Object.keys(shadowing).sort(), [
    "Array",
    "Bytes",
    "PrincipalValue",
    "Promise",
    "Record",
    "Schema",
    "Texts",
    "Uint8Array",
    "Unit",
    "Uses",
    "actor",
    "c",
    "default",
    "delete",
    "string",
  ]);
  // No `$`-prefixed local leaks as an export.
  assert(Object.keys(shadowing).every((name) => !name.startsWith("$")));
});

test("the renamed exports validate exactly their declared domains", () => {
  const cases: Array<[string, unknown, unknown, unknown]> = [
    ["Array (int32)", ArraySchema, 7, 2 ** 31],
    ["Schema (int16)", SchemaDecl, -3, 2 ** 15],
    ["c (int8)", C, -1, 128],
    ["Record (int64)", RecordSchema, record, 1],
    ["Uint8Array (float32)", Uint8ArraySchema, u8, "x"],
    ["delete (float64)", Delete, 1.5, 1n],
    ["string (nat16)", StringSchema, str, -1],
    ["default (nat32)", Default, def, 2 ** 32],
    ["Promise (record)", PromiseSchema, { id: 42n }, { id: 42 }],
    ["PrincipalValue (record)", PrincipalRecord, { p: principal }, { p: "aaaaa-aa" }],
    ["Texts (vec text)", Texts, texts, [1]],
    ["Bytes (blob)", Bytes, bytes, [1, 2]],
    ["Unit (empty record)", Unit, unit, null],
    ["Uses", Uses, uses, { ...uses, t: "not a vec" }],
  ];
  for (const [label, schema, good, bad] of cases) {
    const accepted = validate(schema as Parameters<typeof validate>[0], good);
    assert(accepted.ok, `${label} must accept its value: ${JSON.stringify(accepted)}`);
    assert.strictEqual(
      validate(schema as Parameters<typeof validate>[0], bad).ok,
      false,
      `${label} must refuse a value outside its domain`,
    );
  }
});

test("the declarations round-trip through the codec by export name", () => {
  const encoded = encode(Uses, uses);
  assert(encoded.ok, "Uses must encode");
  if (!encoded.ok) return;
  const decoded = decode(Uses, encoded.bytes);
  assert(decoded.ok, "Uses must decode");
  if (!decoded.ok) return;
  const value = decoded.value as Uses;
  assert.deepStrictEqual(
    { ...value, w: { p: value.w.p.toText() } },
    { ...uses, w: { p: "aaaaa-aa" } },
  );
});

test("the actor's service schema references the renamed declarations", () => {
  const methods = serviceMethods(actor);
  assert.deepStrictEqual([...methods.keys()].sort(), ["all", "get", "ping"]);
  const ping = methods.get("ping");
  if (ping === undefined) throw new Error("ping must be a method");
  assert.strictEqual(ping.args[0], C);
  assert.strictEqual(ping.results[0], PromiseSchema);
  const get = methods.get("get");
  if (get === undefined) throw new Error("get must be a method");
  assert.strictEqual(get.args[0], StringSchema);
  assert.strictEqual(get.results[0], Delete);
  assert.strictEqual(get.mode, "query");
});
