// Per-combinator unit tests for `validate`, boundary and one-step-over cases
// included. Every combinator `schema.ts` exports is covered; the golden
// cross-check in crosscheck.test.ts is what proves generated and dynamic
// schemas agree — this file pins the validator's own behavior.

import { test } from "node:test";
import assert from "node:assert/strict";
import { runInNewContext } from "node:vm";

import { c, type Schema } from "../schema.ts";
import { validate, type ValidateResult, type ValidationCode } from "../validate.ts";

function ok(schema: { readonly kind: string }, value: unknown): void {
  assert.deepStrictEqual(validate(schema as Schema<unknown>, value), { ok: true });
}

function codesOf(result: ValidateResult): ValidationCode[] {
  return result.ok ? [] : result.issues.map((issue) => issue.code);
}

function firstIssue(result: ValidateResult): { code: string; path: string } {
  assert(!result.ok, "expected validation to fail");
  if (result.ok) {
    throw new Error("unreachable");
  }
  const issue = result.issues[0];
  return { code: issue.code, path: issue.path };
}

function fails(
  schema: { readonly kind: string },
  value: unknown,
  code: ValidationCode,
  path = "$",
): void {
  const result = validate(schema as Schema<unknown>, value);
  assert.deepStrictEqual(firstIssue(result), { code, path });
}

test("null accepts exactly null", () => {
  ok(c.null, null);
  fails(c.null, undefined, "invalid_type");
  fails(c.null, 0, "invalid_type");
});

test("bool accepts exactly booleans", () => {
  ok(c.bool, true);
  ok(c.bool, false);
  fails(c.bool, 1, "invalid_type");
  fails(c.bool, "true", "invalid_type");
});

test("text accepts exactly strings", () => {
  ok(c.text, "");
  ok(c.text, "hi");
  fails(c.text, 5, "invalid_type");
});

test("nat requires a non-negative bigint and rejects number", () => {
  ok(c.nat, 0n);
  ok(c.nat, 2n ** 200n);
  fails(c.nat, -1n, "out_of_range");
  fails(c.nat, 5, "invalid_type");
});

test("int requires a bigint of any sign", () => {
  ok(c.int, -(2n ** 200n));
  ok(c.int, 0n);
  fails(c.int, -5, "invalid_type");
});

test("nat64 boundaries: 2^64-1 passes, 2^64 and -1 do not", () => {
  ok(c.nat64, 0n);
  ok(c.nat64, 2n ** 64n - 1n);
  fails(c.nat64, 2n ** 64n, "out_of_range");
  fails(c.nat64, -1n, "out_of_range");
  fails(c.nat64, 7, "invalid_type");
});

test("int64 boundaries: [-2^63, 2^63-1] exactly", () => {
  ok(c.int64, -(2n ** 63n));
  ok(c.int64, 2n ** 63n - 1n);
  fails(c.int64, -(2n ** 63n) - 1n, "out_of_range");
  fails(c.int64, 2n ** 63n, "out_of_range");
});

test("fixed-width naturals: boundary and one step over", () => {
  ok(c.nat8, 0);
  ok(c.nat8, 255);
  fails(c.nat8, 256, "out_of_range");
  fails(c.nat8, -1, "out_of_range");
  fails(c.nat8, 1.5, "not_integer");
  fails(c.nat8, NaN, "not_integer");
  fails(c.nat8, 1n, "invalid_type");
  fails(c.nat8, "1", "invalid_type");
  ok(c.nat16, 65_535);
  fails(c.nat16, 65_536, "out_of_range");
  ok(c.nat32, 4_294_967_295);
  fails(c.nat32, 4_294_967_296, "out_of_range");
});

test("fixed-width integers: boundary and one step over", () => {
  ok(c.int8, -128);
  ok(c.int8, 127);
  fails(c.int8, -129, "out_of_range");
  fails(c.int8, 128, "out_of_range");
  ok(c.int16, -32_768);
  ok(c.int16, 32_767);
  fails(c.int16, -32_769, "out_of_range");
  fails(c.int16, 32_768, "out_of_range");
  ok(c.int32, -2_147_483_648);
  ok(c.int32, 2_147_483_647);
  fails(c.int32, -2_147_483_649, "out_of_range");
  fails(c.int32, 2_147_483_648, "out_of_range");
});

test("floats accept any number, NaN and infinities included", () => {
  ok(c.float32, 0.1);
  ok(c.float32, NaN);
  ok(c.float64, Infinity);
  fails(c.float32, 1n, "invalid_type");
  fails(c.float64, "1.5", "invalid_type");
});

test("reserved accepts anything", () => {
  ok(c.reserved, null);
  ok(c.reserved, undefined);
  ok(c.reserved, { anything: [1, 2, 3] });
});

test("empty accepts nothing", () => {
  fails(c.empty, null, "uninhabited_type");
  fails(c.empty, 0, "uninhabited_type");
  fails(c.empty, undefined, "uninhabited_type");
});

test("principal is checked structurally", () => {
  ok(c.principal, { toText: () => "aaaa-aa" });
  fails(c.principal, "aaaa-aa", "invalid_type");
  fails(c.principal, {}, "invalid_type");
  fails(c.principal, null, "invalid_type");
});

test("opt admits null and the inner type, nothing else", () => {
  const schema = c.opt(c.text);
  ok(schema, null);
  ok(schema, "hi");
  fails(schema, undefined, "invalid_type");
  fails(schema, 5, "invalid_type");
});

test("vec validates every element with an indexed path", () => {
  const schema = c.vec(c.nat8);
  ok(schema, []);
  ok(schema, [0, 255]);
  fails(schema, "not-an-array", "invalid_type");
  const result = validate(schema, [0, 256, -1]);
  assert.deepStrictEqual(codesOf(result), ["out_of_range", "out_of_range"]);
  assert(!result.ok);
  if (!result.ok) {
    assert.deepStrictEqual(
      result.issues.map((issue) => issue.path),
      ["$[1]", "$[2]"],
    );
  }
});

test("blob requires a Uint8Array, not a number array", () => {
  ok(c.blob(), new Uint8Array([1, 2]));
  ok(c.blob(), new Uint8Array(0));
  fails(c.blob(), [1, 2], "invalid_type");
});

test("unit is the empty record, not any non-nullish thing", () => {
  ok(c.unit(), {});
  fails(c.unit(), { a: 1 }, "unexpected_field", "$.a");
  fails(c.unit(), [], "invalid_type");
  fails(c.unit(), null, "invalid_type");
});

test("record requires every field and rejects unknown keys", () => {
  const schema = c.record({ id: c.nat32, label: c.text });
  ok(schema, { id: 1, label: "a" });
  fails(schema, { id: 1 }, "missing_field", "$.label");
  fails(schema, { id: 1, label: "a", extra: true }, "unexpected_field", "$.extra");
  fails(schema, null, "invalid_type");
  fails(schema, [], "invalid_type");
});

test("record issue order is schema fields first, then extras", () => {
  const schema = c.record({ a: c.bool, b: c.text });
  const result = validate(schema, { b: 5, z: 1 });
  assert.deepStrictEqual(codesOf(result), ["missing_field", "invalid_type", "unexpected_field"]);
});

test("a value key from Object.prototype does not satisfy a record field", () => {
  const schema = c.record({ a: c.bool });
  fails(schema, { a: true, toString: 1 }, "unexpected_field", "$.toString");
});

test("non-identifier record keys render bracketed in paths", () => {
  const schema = c.record({ "has space": c.nat });
  fails(schema, {}, "missing_field", '$["has space"]');
});

test("tuple checks exact length before elements", () => {
  const schema = c.tuple([c.nat, c.text]);
  ok(schema, [1n, "x"]);
  fails(schema, [1n], "invalid_length");
  fails(schema, [1n, "x", true], "invalid_length");
  fails(schema, [1, "x"], "invalid_type", "$[0]");
});

test("variant: bare tag for null payloads, { tag, value } otherwise", () => {
  const schema = c.variant({ ok: c.null, busy: c.nat32, failed: c.text });
  ok(schema, { tag: "ok" });
  ok(schema, { tag: "busy", value: 3 });
  fails(schema, { tag: "ok", value: null }, "unexpected_field", "$.value");
  fails(schema, { tag: "busy" }, "missing_field", "$.value");
  fails(schema, { tag: "busy", value: 3, extra: 1 }, "unexpected_field", "$.extra");
  fails(schema, { tag: "nope" }, "unknown_tag", "$.tag");
  fails(schema, {}, "missing_field", "$.tag");
  fails(schema, { tag: 5 }, "invalid_type", "$.tag");
  fails(schema, "ok", "invalid_type");
});

test("variant arm classification sees through rec, as dynamic schemas need", () => {
  const schema = c.variant({
    ok: c.rec(() => c.null),
    busy: c.rec(() => c.nat32),
  });
  ok(schema, { tag: "ok" });
  fails(schema, { tag: "ok", value: null }, "unexpected_field", "$.value");
  ok(schema, { tag: "busy", value: 3 });
  fails(schema, { tag: "busy" }, "missing_field", "$.value");
});

test("a tag from Object.prototype is not an arm", () => {
  const schema = c.variant({ ok: c.null });
  fails(schema, { tag: "toString" }, "unknown_tag", "$.tag");
});

test("rec unwraps to its body", () => {
  type List = { head: bigint; tail: List } | null;
  const List: Schema<List> = c.rec(() => c.opt(c.record({ head: c.nat, tail: List })));
  ok(List, null);
  ok(List, { head: 1n, tail: { head: 2n, tail: null } });
  fails(List, { head: 1, tail: null }, "invalid_type", "$.head");
});

test("deep recursion fails closed at the depth limit, without overflowing", () => {
  type List = { head: bigint; tail: List } | null;
  const List: Schema<List> = c.rec(() => c.opt(c.record({ head: c.nat, tail: List })));
  let list: List = null;
  for (let i = 0; i < 100_000; i += 1) {
    list = { head: BigInt(i), tail: list };
  }
  const result = validate(List, list);
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues.length, 1);
    const issue = result.issues[0];
    assert.strictEqual(issue.code, "resource_limit_exceeded");
    assert.strictEqual(issue.resource_limit?.resource, "value_depth");
    assert.strictEqual(issue.resource_limit?.limit, 256);
  }
});

test("a cyclic value terminates via the depth limit", () => {
  type List = { head: bigint; tail: List } | null;
  const List: Schema<List> = c.rec(() => c.opt(c.record({ head: c.nat, tail: List })));
  const cyclic: { head: bigint; tail: unknown } = { head: 0n, tail: null };
  cyclic.tail = cyclic;
  const result = validate(List, cyclic as List);
  assert.deepStrictEqual(codesOf(result), ["resource_limit_exceeded"]);
});

test("a self-referential rec chain terminates via the depth limit", () => {
  // Not constructible from a validated Contract; a hand-built pathological
  // schema must still terminate rather than loop.
  const loop: { schema?: Schema<null> } = {};
  loop.schema = c.rec(() => loop.schema!);
  const result = validate(loop.schema, null);
  assert.deepStrictEqual(codesOf(result), ["resource_limit_exceeded"]);
});

test("the element budget bounds total work", () => {
  const big = new Array<number>(1000).fill(0);
  const result = validate(c.vec(c.nat8), big, { maxElements: 100 });
  assert(!result.ok);
  if (!result.ok) {
    const issue = result.issues[result.issues.length - 1];
    assert.strictEqual(issue.code, "resource_limit_exceeded");
    assert.strictEqual(issue.resource_limit?.resource, "value_elements");
  }
});

test("maxIssues caps collection deterministically", () => {
  const bad = new Array<string>(50).fill("x");
  const result = validate(c.vec(c.nat8), bad, { maxIssues: 3 });
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues.length, 3);
    assert.deepStrictEqual(
      result.issues.map((issue) => issue.path),
      ["$[0]", "$[1]", "$[2]"],
    );
  }
});

test("a resource failure never reports ok on a truncated walk", () => {
  // The value is entirely valid; only the budget stops the walk. Reporting
  // ok here would claim an examination that never happened.
  const valid = new Array<number>(1000).fill(1);
  const result = validate(c.vec(c.nat8), valid, { maxElements: 10 });
  assert(!result.ok);
});

test("a malformed schema object fails closed", () => {
  const bogus = { kind: "wat" } as unknown as Schema<unknown>;
  fails(bogus, null, "unsupported_schema");
  const bogusPrimitive = {
    kind: "primitive",
    primitive: "wat",
  } as unknown as Schema<unknown>;
  fails(bogusPrimitive, null, "unsupported_schema");
  const bogusRec = c.rec(() => undefined as unknown as Schema<null>);
  fails(bogusRec, null, "unsupported_schema");
});

test("values that throw while inspected fail closed with unreadable_value", () => {
  const cases: readonly [schema: { readonly kind: string }, value: unknown][] = [
    // An own accessor on a required record field.
    [
      c.record({ a: c.bool }),
      {
        get a(): boolean {
          throw new Error("boom");
        },
      },
    ],
    // Accessors on the variant tag and payload.
    [
      c.variant({ ok: c.null }),
      {
        get tag(): string {
          throw new Error("boom");
        },
      },
    ],
    [
      c.variant({ busy: c.nat32 }),
      {
        tag: "busy",
        get value(): number {
          throw new Error("boom");
        },
      },
    ],
    // An accessor behind the principal duck check.
    [
      c.principal,
      {
        get toText(): () => string {
          throw new Error("boom");
        },
      },
    ],
    // Proxy traps: ownKeys, get, getOwnPropertyDescriptor, getPrototypeOf.
    [
      c.record({ a: c.bool }),
      new Proxy(
        {},
        {
          ownKeys() {
            throw new Error("boom");
          },
        },
      ),
    ],
    // The target carries the property so presence passes; the read traps.
    [
      c.record({ a: c.bool }),
      new Proxy(
        { a: true },
        {
          get() {
            throw new Error("boom");
          },
        },
      ),
    ],
    // The target needs a key for Object.keys to consult the trap at all.
    // (A getPrototypeOf trap is no longer reachable anywhere: the typed-array
    // brand check reads internal slots, not the prototype chain.)
    [
      c.unit(),
      new Proxy(
        { z: 1 },
        {
          getOwnPropertyDescriptor() {
            throw new Error("boom");
          },
        },
      ),
    ],
    // An array whose element is an accessor, nested one level down.
    [
      c.vec(c.record({ a: c.bool })),
      [
        {
          get a(): boolean {
            throw new Error("boom");
          },
        },
      ],
    ],
    [
      c.tuple([c.nat]),
      new Proxy([1n], {
        get(_t, key) {
          if (key === "length") {
            throw new Error("boom");
          }
          return undefined;
        },
      }),
    ],
  ];
  for (const [schema, value] of cases) {
    const result = validate(schema as Schema<unknown>, value);
    assert(!result.ok, "hostile value must not validate");
    if (!result.ok) {
      const last = result.issues[result.issues.length - 1];
      assert.strictEqual(last.code, "unreadable_value");
    }
  }
  // A revoked Proxy makes even Array.isArray throw; primitives included.
  const { proxy, revoke } = Proxy.revocable({}, {});
  revoke();
  for (const schema of [c.text, c.record({ a: c.bool }), c.blob()]) {
    const result = validate(schema as Schema<unknown>, proxy);
    assert(!result.ok);
    if (!result.ok) {
      assert.strictEqual(result.issues[0].code, "unreadable_value");
    }
  }
});

test("unreadable_value points at the value that threw", () => {
  const schema = c.record({ a: c.record({ b: c.bool }) });
  const value = {
    a: {
      get b(): boolean {
        throw new Error("boom");
      },
    },
  };
  const result = validate(schema, value as unknown as { a: { b: boolean } });
  assert(!result.ok);
  if (!result.ok) {
    assert.deepStrictEqual(
      result.issues.map((issue) => [issue.code, issue.path]),
      [["unreadable_value", "$.a.b"]],
    );
  }
});

test("blob checks the typed-array brand, not the prototype chain", () => {
  // A prototype forgery has no typed-array internal slots: every real
  // Uint8Array operation would throw on it, so accepting it is fail-open.
  fails(c.blob(), Object.create(Uint8Array.prototype), "invalid_type");
  // A genuine Uint8Array from another realm has the slots but a different
  // prototype chain; rejecting it would be a false negative.
  ok(c.blob(), runInNewContext("new Uint8Array([1, 2])"));
});

test("presence means own enumerable: the serialized projection is what validates", () => {
  const hidden: Record<string, unknown> = {};
  Object.defineProperty(hidden, "a", { value: true, enumerable: false });
  // JSON.stringify(hidden) is "{}", and {} does not satisfy the record — so
  // neither must this value.
  fails(c.record({ a: c.bool }), hidden, "missing_field", "$.a");
  // A non-enumerable extra is invisible to the same projection: not unknown.
  const extra: Record<string, unknown> = { a: true };
  Object.defineProperty(extra, "z", { value: 1, enumerable: false });
  ok(c.record({ a: c.bool }), extra);
  const unitExtra: Record<string, unknown> = {};
  Object.defineProperty(unitExtra, "z", { value: 1, enumerable: false });
  ok(c.unit(), unitExtra);
  // Symbols are outside `Object.keys` altogether and outside the projection
  // too, so a symbol-keyed extra is not an unknown field either.
  ok(c.unit(), { [Symbol("hidden")]: 1 });
  ok(c.record({ a: c.bool }), { a: true, [Symbol("hidden")]: 1 });
});

test("examined record keys are charged against the element budget", () => {
  const value: Record<string, boolean> = { a: true };
  for (let i = 0; i < 1000; i += 1) {
    value[`extra${i}`] = true;
  }
  const result = validate(c.record({ a: c.bool }), value, {
    maxElements: 10,
    maxIssues: 1_000_000,
  });
  assert(!result.ok);
  if (!result.ok) {
    const last = result.issues[result.issues.length - 1];
    assert.strictEqual(last.code, "resource_limit_exceeded");
    assert.strictEqual(last.resource_limit?.resource, "value_elements");
  }
});

test("a rec hop on a variant payload is charged exactly once", () => {
  // Documented accounting: one extra element per rec hop. The dynamic loader
  // wraps every arm edge in c.rec, so double-charging would reject valid
  // values one budget step early.
  // Cost model for { tag, value }: 1 variant node + 1 payload node + 2
  // examined keys in the unknown-key scan = 4; the rec wrapper adds exactly
  // its 1 hop.
  const direct = c.variant({ busy: c.nat32 });
  const wrapped = c.variant({ busy: c.rec(() => c.nat32) });
  const value = { tag: "busy", value: 3 };
  assert.deepStrictEqual(validate(direct, value, { maxElements: 4 }), { ok: true });
  assert.deepStrictEqual(validate(wrapped, value, { maxElements: 5 }), { ok: true });
  const short = validate(wrapped, value, { maxElements: 4 });
  assert(!short.ok);
});

test("validate never throws on hostile values", () => {
  const schemas = [
    c.null,
    c.nat,
    c.blob(),
    c.unit(),
    c.opt(c.text),
    c.vec(c.nat),
    c.record({ a: c.bool }),
    c.tuple([c.nat]),
    c.variant({ ok: c.null }),
    c.rec(() => c.text),
  ];
  const values = [
    undefined,
    null,
    Symbol("s"),
    () => {},
    Object.create(null),
    new Date(0),
    { tag: null },
    { tag: "ok", value: undefined },
    [[]],
    -0,
  ];
  for (const schema of schemas) {
    for (const value of values) {
      validate(schema as Schema<unknown>, value);
    }
  }
});

test("func values are strict: extras rejected, both fields required", () => {
  const schema = c.func([], [], "update");
  const principal = { toText: () => "aaaaa-aa" };
  ok(schema, { principal, method: "go" });
  fails(schema, { principal, method: "go", extra: 1 }, "unexpected_field", "$.extra");
  fails(schema, { principal }, "missing_field", "$.method");
  fails(schema, { method: "go" }, "missing_field", "$.principal");
  fails(schema, { principal, method: "" }, "invalid_type", "$.method");
  ok(c.service({}), principal);
  fails(c.service({}), "aaaaa-aa", "invalid_type", "$");
});
