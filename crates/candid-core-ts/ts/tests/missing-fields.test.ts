// A field the wire omits, at an expected opt-like type (`opt`, `null`,
// `reserved`), decodes as `null`: a node of the decoded value at the field's
// own level. `decode` charges it there exactly as it charges the same `null`
// read from the wire, and as `validate` charges it — one depth check and one
// element — so a value `decode` returns is never one `validate` refuses on
// `value_depth` under the same `maxDepth`. Before, a synthesized field was
// charged nothing: a record at `maxDepth` decoded with a missing optional
// field one level past the bound, and the synthesized nulls escaped
// `maxElements`.
//
// The differential corpus pins the default bounds against the Rust
// reference: `depth_missing_opt_field_256_levels` (accepted) and
// `depth_missing_opt_field_257_levels` (refused, as the reference's
// `validate_host_value` refuses the same domain value,
// `validate_missing_opt_field_257_levels`), and
// `elements_missing_opt_fields_vec_499999_and_arg` and
// `elements_missing_opt_fields_vec_500000` for `maxElements`.

import { test } from "node:test";
import assert from "node:assert/strict";

import { c, type AnySchema } from "../schema.ts";
import { validate } from "../validate.ts";
import { decode, decodeArgs, encode, encodeArgs } from "../codec.ts";

/** The three opt-like expected types a missing field decodes as `null` at. */
const LEAVES: readonly [string, AnySchema][] = [
  ["opt", c.opt(c.nat)],
  ["null", c.null],
  ["reserved", c.reserved],
];

function bytesOf(result: ReturnType<typeof encode>): Uint8Array {
  if (!result.ok) {
    throw new Error(`must encode: ${JSON.stringify(result.issues)}`);
  }
  return result.bytes;
}

/** The least `maxElements` at which `attempt` succeeds. */
function leastElements(attempt: (maxElements: number) => { readonly ok: boolean }): number {
  let low = 0;
  let high = 1 << 20;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (attempt(middle).ok) {
      high = middle;
    } else {
      low = middle + 1;
    }
  }
  return low;
}

/** Three nested records on the wire, the innermost empty. */
const threeRecords = bytesOf(
  encode(c.record({ a: c.record({ a: c.record({}) }) }), { a: { a: {} } }),
);

/** The same records expected with an opt-like `x` in the innermost: at level 3. */
function expectingX(leaf: AnySchema): AnySchema {
  return c.record({ a: c.record({ a: c.record({ x: leaf }) }) });
}

test("a missing opt-like field is charged at its own level: accepted at maxDepth, refused one past", () => {
  for (const [name, leaf] of LEAVES) {
    const schema = expectingX(leaf);
    assert.deepStrictEqual(
      decode(schema, threeRecords, { maxDepth: 3 }),
      { ok: true, value: { a: { a: { x: null } } } },
      name,
    );
    const resource_limit = { resource: "value_depth", limit: 2, observed: 3 };
    assert.deepStrictEqual(
      decode(schema, threeRecords, { maxDepth: 2 }),
      {
        ok: false,
        issues: [
          {
            code: "resource_limit_exceeded",
            path: "$.a.a.x",
            message: "value_depth limit 2 exceeded",
            resource_limit,
          },
        ],
      },
      name,
    );
    // validate refuses the domain value at the same node, on the same budget.
    const refused = validate(schema, { a: { a: { x: null } } }, { maxDepth: 2 });
    assert.deepStrictEqual(
      refused.ok ? undefined : refused.issues.map((issue) => [issue.path, issue.resource_limit]),
      [["$.a.a.x", resource_limit]],
      name,
    );
  }
});

test("a missing field behind a rec is charged the same: the hop is not a level", () => {
  for (const [name, leaf] of LEAVES) {
    const schema = expectingX(c.rec(() => leaf));
    assert.strictEqual(decode(schema, threeRecords, { maxDepth: 3 }).ok, true, name);
    const refused = decode(schema, threeRecords, { maxDepth: 2 });
    assert.deepStrictEqual(
      refused.ok ? undefined : refused.issues[0].resource_limit,
      { resource: "value_depth", limit: 2, observed: 3 },
      name,
    );
  }
});

test("at the default maxDepth, a missing field at level 256 is accepted and at 257 refused, as validate draws it", () => {
  for (const [name, leaf] of LEAVES) {
    // T = variant { a : T; b : record { x : leaf } }, read from a wire
    // variant { a : W; b : record {} }: `variants` variants, the record one
    // level below the innermost, x one below that.
    const T: AnySchema = c.rec(() => c.variant({ a: T, b: c.record({ x: leaf }) }));
    const W: AnySchema = c.rec(() => c.variant({ a: W, b: c.record({}) }));
    const wireValue = (variants: number): unknown => {
      let value: unknown = { tag: "b", value: {} };
      for (let level = 1; level < variants; level += 1) {
        value = { tag: "a", value };
      }
      return value;
    };
    const domainValue = (variants: number): unknown => {
      let value: unknown = { tag: "b", value: { x: null } };
      for (let level = 1; level < variants; level += 1) {
        value = { tag: "a", value };
      }
      return value;
    };
    // x at level 256: exactly the default bound.
    const at = decode(T, bytesOf(encode(W, wireValue(255))));
    assert.deepStrictEqual(at, { ok: true, value: domainValue(255) }, name);
    assert.deepStrictEqual(validate(T, domainValue(255)), { ok: true }, name);
    // x at level 257: one past it, refused by both at the same node.
    const path = `$${".value".repeat(256)}.x`;
    const resource_limit = { resource: "value_depth", limit: 256, observed: 257 };
    const past = decode(T, bytesOf(encode(W, wireValue(256))));
    assert.deepStrictEqual(
      past.ok ? undefined : past.issues.map((issue) => [issue.path, issue.resource_limit]),
      [[path, resource_limit]],
      name,
    );
    const refused = validate(T, domainValue(256));
    assert.deepStrictEqual(
      refused.ok ? undefined : refused.issues.map((issue) => [issue.path, issue.resource_limit]),
      [[path, resource_limit]],
      name,
    );
  }
});

test("decode then validate: every value decode returns at a maxDepth, validate accepts at it", () => {
  for (const [name, leaf] of LEAVES) {
    const schema = expectingX(leaf);
    for (let maxDepth = 0; maxDepth <= 5; maxDepth += 1) {
      const decoded = decode(schema, threeRecords, { maxDepth });
      // The least maxDepth either accepts is 3, x's level.
      assert.strictEqual(decoded.ok, maxDepth >= 3, `${name} at maxDepth ${maxDepth}`);
      if (decoded.ok) {
        assert.deepStrictEqual(
          validate(schema, decoded.value, { maxDepth }),
          { ok: true },
          `${name} at maxDepth ${maxDepth}`,
        );
      }
    }
  }
});

test("each synthesized field is one element, as validate charges that value", () => {
  for (const [name, leaf] of LEAVES) {
    const expected = c.vec(c.record({ x: leaf }));
    for (const count of [1, 3, 4]) {
      const wire = bytesOf(encode(c.vec(c.record({})), new Array(count).fill({})));
      // The vec, each record and each record's synthesized x: 1 + 2N.
      const decodeLeast = leastElements((maxElements) => decode(expected, wire, { maxElements }));
      assert.strictEqual(decodeLeast, 1 + 2 * count, `${name}, ${count} records`);
      const decoded = decode(expected, wire);
      if (!decoded.ok) {
        throw new Error(`${name}: must decode: ${JSON.stringify(decoded.issues)}`);
      }
      const value = decoded.value;
      // validate charges the same nodes, and one more per examined record
      // key (`x`, once per record), which decode never charges: the two
      // differ by exactly the key charges and nothing else.
      const validateLeast = leastElements((maxElements) =>
        validate(expected, value, { maxElements }),
      );
      assert.strictEqual(validateLeast - decodeLeast, count, `${name}, ${count} records`);
      // One past the bound, the refusal is at the last synthesized field.
      const refused = decode(expected, wire, { maxElements: decodeLeast - 1 });
      assert.deepStrictEqual(
        refused.ok ? undefined : [refused.issues[0].path, refused.issues[0].resource_limit],
        [
          `$[${count - 1}].x`,
          { resource: "value_elements", limit: decodeLeast - 1, observed: decodeLeast },
        ],
        `${name}, ${count} records`,
      );
    }
  }
});

test("a missing trailing argument is one element at depth 0", () => {
  const wire = bytesOf(encodeArgs([c.nat], [7n]));
  for (const [name, leaf] of LEAVES) {
    // The wire nat and the synthesized argument: two elements.
    assert.strictEqual(
      leastElements((maxElements) => decodeArgs([c.nat, leaf], wire, { maxElements })),
      2,
      name,
    );
    assert.deepStrictEqual(
      decodeArgs([c.nat, leaf], wire, { maxDepth: 0 }),
      { ok: true, values: [7n, null] },
      name,
    );
    const refused = decodeArgs([c.nat, leaf], wire, { maxElements: 1 });
    assert.deepStrictEqual(
      refused.ok ? undefined : [refused.issues[0].path, refused.issues[0].resource_limit],
      ["$args[1]", { resource: "value_elements", limit: 1, observed: 2 }],
      name,
    );
  }
});

test("a missing required field is charged nothing: an enclosing opt still absorbs it at the bound", () => {
  // opt at level 0, the record at 1, its missing `x` would be at 2: there is
  // no value to charge, so the coercion failure is absorbed to null rather
  // than refused on value_depth.
  const emptyRecord = bytesOf(encode(c.record({}), {}));
  assert.deepStrictEqual(decode(c.opt(c.record({ x: c.nat })), emptyRecord, { maxDepth: 1 }), {
    ok: true,
    value: null,
  });
  // A wire record { x } would have its field charged at level 2 and refused.
  const present = bytesOf(encode(c.record({ x: c.nat }), { x: 1n }));
  const refused = decode(c.opt(c.record({ x: c.nat })), present, { maxDepth: 1 });
  assert.deepStrictEqual(refused.ok ? undefined : refused.issues[0].resource_limit, {
    resource: "value_depth",
    limit: 1,
    observed: 2,
  });
});
