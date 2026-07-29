import { test } from "node:test";
import assert from "node:assert/strict";
import { c } from "../src/schema.ts";
import { checkSchema, validate } from "../src/validate.ts";
import { principalFromText } from "../src/principal.ts";
import type { AnySchema } from "../src/walk.ts";

function firstCode(schema: AnySchema, value: unknown): string {
  const result = validate(schema, value);
  assert.equal(result.ok, false, "expected validation to fail");
  if (result.ok) {
    throw new Error("unreachable");
  }
  const first = result.issues[0];
  assert.notEqual(first, undefined);
  return first?.code ?? "";
}

test("fixed-width integers: exact boundary and one step over", () => {
  assert.equal(validate(c.nat8, 0).ok, true);
  assert.equal(validate(c.nat8, 255).ok, true);
  assert.equal(firstCode(c.nat8, 256), "int_out_of_range");
  assert.equal(firstCode(c.nat8, -1), "int_out_of_range");
  assert.equal(firstCode(c.nat8, 1.5), "number_not_integer");
  assert.equal(validate(c.int8, -128).ok, true);
  assert.equal(firstCode(c.int8, -129), "int_out_of_range");
  assert.equal(validate(c.nat32, 4_294_967_295).ok, true);
  assert.equal(firstCode(c.nat32, 4_294_967_296), "int_out_of_range");
});

test("unbounded and 64-bit integers require bigint, no number coercion", () => {
  assert.equal(validate(c.nat, 0n).ok, true);
  assert.equal(firstCode(c.nat, 0), "expected_bigint");
  assert.equal(firstCode(c.nat, -1n), "bigint_out_of_range");
  assert.equal(validate(c.nat64, (1n << 64n) - 1n).ok, true);
  assert.equal(firstCode(c.nat64, 1n << 64n), "bigint_out_of_range");
  assert.equal(validate(c.int64, -(1n << 63n)).ok, true);
  assert.equal(firstCode(c.int64, -(1n << 63n) - 1n), "bigint_out_of_range");
  assert.equal(validate(c.int, -123456789012345678901234567890n).ok, true);
});

test("floats accept NaN and the infinities", () => {
  assert.equal(validate(c.float64, Number.NaN).ok, true);
  assert.equal(validate(c.float32, Number.POSITIVE_INFINITY).ok, true);
  assert.equal(firstCode(c.float64, "1.5"), "expected_number");
});

test("reserved accepts anything; empty accepts nothing", () => {
  assert.equal(validate(c.reserved, { anything: true }).ok, true);
  // `Schema<never>` sits outside even the `any` variance wildcard (the
  // phantom's parameter position is contravariant), so `empty` needs a cast
  // anywhere a generic schema is expected — worth recording for #102.
  assert.equal(firstCode(c.empty as unknown as AnySchema, null), "expected_empty");
});

test("principal is structurally checked", () => {
  assert.equal(validate(c.principal, principalFromText("2vxsx-fae")).ok, true);
  assert.equal(firstCode(c.principal, "2vxsx-fae"), "expected_principal");
});

test("records: missing and unexpected fields, path-addressed", () => {
  const schema = c.record({ id: c.nat32, label: c.text });
  assert.equal(validate(schema, { id: 1, label: "x" }).ok, true);
  const missing = validate(schema, { id: 1 });
  assert.equal(missing.ok, false);
  if (!missing.ok) {
    assert.deepEqual(
      missing.issues.map((issue) => [issue.code, issue.path]),
      [["missing_field", "$.label"]],
    );
  }
  const extra = validate(schema, { id: 1, label: "x", bogus: 2 });
  assert.equal(extra.ok, false);
  if (!extra.ok) {
    assert.deepEqual(
      extra.issues.map((issue) => [issue.code, issue.path]),
      [["unexpected_field", "$.bogus"]],
    );
  }
});

test("variants: tag shapes, hostile tags, payload rules", () => {
  const schema = c.variant({ ok: c.null, busy: c.nat32 });
  assert.equal(validate(schema, { tag: "ok" }).ok, true);
  assert.equal(validate(schema, { tag: "busy", value: 2 }).ok, true);
  assert.equal(
    firstCode(schema, { tag: "ok", value: null }),
    "variant_payload_unexpected",
  );
  assert.equal(firstCode(schema, { tag: "busy" }), "variant_payload_missing");
  assert.equal(firstCode(schema, { tag: "nope" }), "unknown_variant_tag");
  // Prototype-chain tags must miss, not resolve to Object.prototype members.
  assert.equal(firstCode(schema, { tag: "constructor" }), "unknown_variant_tag");
  assert.equal(firstCode(schema, { tag: "ok", extra: 1 }), "expected_variant");
  assert.equal(firstCode(schema, "ok"), "expected_variant");
});

test("an empty-payload variant arm cannot be selected", () => {
  const schema = c.variant({
    ok: c.nat,
    err: c.empty as unknown as AnySchema,
  });
  assert.equal(validate(schema, { tag: "ok", value: 1n }).ok, true);
  assert.equal(firstCode(schema, { tag: "err" }), "variant_arm_uninhabited");
});

test("tuples: exact length", () => {
  const schema = c.tuple([c.nat, c.text]);
  assert.equal(validate(schema, [1n, "x"]).ok, true);
  assert.equal(firstCode(schema, [1n]), "tuple_length_mismatch");
  assert.equal(firstCode(schema, [1n, "x", true]), "tuple_length_mismatch");
});

test("unit is the empty record, not any object", () => {
  assert.equal(validate(c.unit(), {}).ok, true);
  assert.equal(firstCode(c.unit(), { a: 1 }), "expected_unit");
  assert.equal(firstCode(c.unit(), []), "expected_unit");
});

test("blob requires Uint8Array", () => {
  assert.equal(validate(c.blob(), new Uint8Array([1, 2])).ok, true);
  assert.equal(firstCode(c.blob(), [1, 2]), "expected_blob");
});

test("opt: null or inner; collapsing options fail closed at use", () => {
  const schema = c.opt(c.text);
  assert.equal(validate(schema, null).ok, true);
  assert.equal(validate(schema, "x").ok, true);
  assert.equal(firstCode(c.opt(c.opt(c.text)), null), "unrepresentable_option");
  assert.equal(firstCode(c.opt(c.null), null), "unrepresentable_option");
  assert.equal(firstCode(c.opt(c.reserved), null), "unrepresentable_option");
});

test("checkSchema rejects collapsing options, aliased included", () => {
  const aliased = c.rec(() => c.opt(c.text));
  assert.deepEqual(checkSchema(c.opt(aliased))[0]?.code, "unrepresentable_option");
  assert.deepEqual(checkSchema(c.opt(c.null))[0]?.code, "unrepresentable_option");
  assert.deepEqual(
    checkSchema(c.opt(c.reserved))[0]?.code,
    "unrepresentable_option",
  );
  assert.deepEqual(checkSchema(c.opt(c.text)), []);
});

test("recursive schemas validate without stack overflow", () => {
  type Tree = { tag: "leaf"; value: bigint } | { tag: "node"; value: { left: Tree; right: Tree } };
  const Tree: import("../src/schema.ts").Schema<Tree> = c.rec(() =>
    c.variant({ leaf: c.nat, node: c.record({ left: Tree, right: Tree }) }),
  );
  const leaf: Tree = { tag: "leaf", value: 1n };
  let tree: Tree = leaf;
  for (let index = 0; index < 20; index += 1) {
    tree = { tag: "node", value: { left: tree, right: leaf } };
  }
  assert.equal(validate(Tree, tree).ok, true);
  const bad: Tree = {
    tag: "node",
    value: { left: { tag: "leaf", value: -1n } as Tree, right: leaf },
  };
  const result = validate(Tree, bad);
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.equal(result.issues[0]?.path, "$.value.left.value");
  }
});

test("depth limit fails closed on adversarial nesting", () => {
  type Deep = unknown[];
  let deepSchema = c.vec(c.nat8) as import("../src/walk.ts").AnySchema;
  let value: Deep = [1];
  for (let index = 0; index < 80; index += 1) {
    deepSchema = c.vec(deepSchema);
    value = [value];
  }
  const result = validate(deepSchema, value);
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.equal(result.issues[0]?.code, "recursion_limit_exceeded");
  }
});

test("work limit bounds total validation effort", () => {
  const schema = c.vec(c.nat8);
  const big = new Array<number>(1000).fill(1);
  const result = validate(schema, big, {
    maxDepth: 64,
    maxWork: 100,
    maxIssues: 64,
  });
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.equal(result.issues[0]?.code, "resource_limit_exceeded");
  }
});

test("issue collection truncates at maxIssues", () => {
  const schema = c.vec(c.nat8);
  const bad = new Array<string>(100).fill("x");
  const result = validate(schema, bad, {
    maxDepth: 64,
    maxWork: 1_000_000,
    maxIssues: 5,
  });
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.equal(result.issues.length, 6);
    assert.equal(result.issues[5]?.code, "issue_limit_reached");
  }
});

test("hostile record keys render bracket-quoted paths", () => {
  const schema = c.record({ "has space": c.nat8 });
  const result = validate(schema, { "has space": "x" });
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.equal(result.issues[0]?.path, '$["has space"]');
  }
});
