// Issue #231: a `rec` hop is an indirection — an alias, a declaration
// reference, a lazy edge — not a level of the value. `validate`, encode's
// value walk and `decode` charge a value for its constructors alone, so
// `maxDepth` and `maxElements` mean what candid-core's `max_value_depth` and
// `max_value_elements` mean, and a recursive value is judged alike whether
// its schema was built with `c.rec`, loaded from a Contract (where every
// type reference is a hop) or built with no `rec` at all. What bounds a
// chain of thunks is its own cap: more than `maxDepth` consecutive hops.
//
// The boundaries are candid-core's: `validate_host_value` under
// `Limits::default()` accepts `vec T` with `T = vec T` nested 257 levels
// (the root at depth 0, the innermost at 256) and refuses 258, and charges a
// variant's `null` payload as a node one level below it. The differential
// corpus pins the same boundaries against the Rust reference
// (`validate_depth_vec_257_levels`, `validate_depth_vec_258_levels`, and the
// `validate_depth_variant_null_*` vectors).

import { test } from "node:test";
import assert from "node:assert/strict";

import { c, type AnySchema, type Schema } from "../schema.ts";
import { validate } from "../validate.ts";
import { decode, encode } from "../codec.ts";
import { schemaFromContract } from "../contract.ts";

type Json = ReturnType<typeof JSON.parse>;
type Nest = Nest[];

/** A one-declaration Contract document, loaded: every edge is a `rec` hop. */
function loaded(types: Json[]): AnySchema {
  const result = schemaFromContract({
    format: "candid-core",
    format_version: 1,
    semantics_profile: "candid-1",
    canonicalization_profile: "candid-core-canon-1",
    types,
    declarations: [{ name: "T", type: 0 }],
  });
  if (!result.ok) {
    throw new Error(`the document must load: ${JSON.stringify(result.issues)}`);
  }
  return result.schemas.T;
}

/** `levels` nested arrays, the innermost empty: `vec T` nested `levels` levels. */
function vecLevels(levels: number): Nest {
  let value: Nest = [];
  for (let level = 1; level < levels; level += 1) {
    value = [value];
  }
  return value;
}

/** The same value's wire bytes at `vec T`: one table entry, `levels` vecs. */
function vecWire(levels: number): Uint8Array {
  const head = [0x44, 0x49, 0x44, 0x4c, 0x01, 0x6d, 0x00, 0x01, 0x00];
  return Uint8Array.from([...head, ...new Array<number>(levels - 1).fill(0x01), 0x00]);
}

/** `vec … vec null`, `levels` vecs deep, built with no `rec` at all. */
function staticVec(levels: number): AnySchema {
  let schema: AnySchema = c.vec(c.null);
  for (let level = 1; level < levels; level += 1) {
    schema = c.vec(schema);
  }
  return schema;
}

const RecVec: Schema<Nest> = c.rec(() => c.vec(RecVec));

/** What a result says, for comparing walks: `ok`, or its first issue whole. */
function verdict(result: { readonly ok: boolean; readonly issues?: readonly unknown[] }): unknown {
  return result.ok ? "ok" : result.issues?.[0];
}

function bytesOf(result: ReturnType<typeof encode>): Uint8Array {
  if (!result.ok) {
    throw new Error(`must encode: ${JSON.stringify(result.issues)}`);
  }
  return result.bytes;
}

/** The least `maxElements` at which `attempt` succeeds. */
function leastElements(attempt: (maxElements: number) => { readonly ok: boolean }): number {
  let low = 0;
  let high = 1 << 22;
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

const DEPTH_257 = {
  code: "resource_limit_exceeded",
  message: "value_depth limit 256 exceeded (observed 257)",
  resource_limit: { resource: "value_depth", limit: 256, observed: 257 },
};

test("vec T nested 257 levels is accepted and 258 refused, with rec, from a Contract, or without", () => {
  const schemas: readonly [string, AnySchema][] = [
    ["c.rec", RecVec],
    ["Contract", loaded([{ kind: "vec", inner: 0 }])],
    ["no rec", staticVec(258)],
  ];
  const refusedAt = `$${"[0]".repeat(257)}`;
  for (const [name, schema] of schemas) {
    assert.deepStrictEqual(validate(schema, vecLevels(257)), { ok: true }, name);
    assert.deepStrictEqual(
      validate(schema, vecLevels(258)),
      { ok: false, issues: [{ ...DEPTH_257, path: refusedAt }] },
      name,
    );
    assert.deepStrictEqual(decode(schema, vecWire(257)), { ok: true, value: vecLevels(257) }, name);
    // decode reports a depth refusal with the codec's message, at the path
    // of the value refused.
    const refused = decode(schema, vecWire(258));
    assert.deepStrictEqual(
      refused.ok ? undefined : refused.issues,
      [
        {
          code: "resource_limit_exceeded",
          path: refusedAt,
          message: "value_depth limit 256 exceeded",
          resource_limit: DEPTH_257.resource_limit,
        },
      ],
      name,
    );
  }
  // encode: a static schema 258 levels deep is refused by the type table
  // before any value is read (#192 decision D1), so the value walk is
  // compared on the two recursive schemas.
  for (const schema of [RecVec, loaded([{ kind: "vec", inner: 0 }])]) {
    assert.deepStrictEqual(bytesOf(encode(schema, vecLevels(257))), vecWire(257));
    const refused = encode(schema, vecLevels(258));
    assert.deepStrictEqual(refused.ok ? undefined : refused.issues[0].resource_limit, {
      resource: "value_depth",
      limit: 256,
      observed: 257,
    });
    assert.strictEqual(refused.ok ? undefined : refused.issues[0].path, refusedAt);
  }
});

type OptNest = OptNest[] | null;
type Boxed = { some: Boxed } | null;

/**
 * `T = opt vec T` (the opt unboxed: its inner `vec` admits no null):
 * `arrays` nested arrays, the opts and vecs alternating from the root opt at
 * depth 0, so array k sits at depth 2k - 1. With `innerNull` the innermost
 * array holds one absent opt, one level below it; otherwise it is empty.
 */
function optVecLevels(arrays: number, innerNull: boolean): OptNest {
  let value: OptNest = innerNull ? [null] : [];
  for (let level = 1; level < arrays; level += 1) {
    value = [value];
  }
  return value;
}

/** `opts` boxed opts, the innermost absent: `T = opt T` nested `opts` deep. */
function boxedLevels(opts: number): Boxed {
  let value: Boxed = null;
  for (let level = 1; level < opts; level += 1) {
    value = { some: value };
  }
  return value;
}

/** `opt vec opt vec …`, `levels` constructors deep, built with no `rec`. */
function staticOptVec(levels: number): AnySchema {
  let schema: AnySchema = c.opt(c.vec(c.null));
  for (let level = 2; level < levels; level += 2) {
    schema = c.opt(c.vec(schema));
  }
  return schema;
}

/** `opt opt … opt null`, `opts` opts deep, built with no `rec`. */
function staticBoxed(opts: number): AnySchema {
  let schema: AnySchema = c.opt(c.null);
  for (let level = 1; level < opts; level += 1) {
    schema = c.opt(schema);
  }
  return schema;
}

test("an opt is a level, boxed or not: accepted with its innermost node at 256, refused at 257", () => {
  // Each opt's payload sits one level below it whether the payload is the
  // value itself (unboxed) or its `some` (boxed). This pins the depth step
  // of each opt site — validate's two, encode's two and decode's — and
  // compares the whole issue across c.rec, Contract-loaded and rec-free
  // schemas (encode's type table refuses the rec-free ones outright, D1).
  const RecOptVec: AnySchema = c.rec(() => c.opt(c.vec(RecOptVec)));
  const RecBoxed: AnySchema = c.rec(() => c.opt(RecBoxed));
  const cases: readonly {
    readonly name: string;
    readonly schemas: readonly [string, AnySchema][];
    readonly accepted: unknown;
    readonly refused: unknown;
    readonly refusedAt: string;
    readonly decodeRefusedAt: string;
  }[] = [
    {
      name: "unboxed opt vec T",
      schemas: [
        ["c.rec", RecOptVec],
        [
          "Contract",
          loaded([
            { kind: "opt", inner: 1 },
            { kind: "vec", inner: 0 },
          ]),
        ],
        ["no rec", staticOptVec(260)],
      ],
      // 128 arrays: the innermost array at 255, its absent opt at 256.
      accepted: optVecLevels(128, true),
      // 129 arrays: the innermost, empty, at 257.
      refused: optVecLevels(129, false),
      refusedAt: `$${"[0]".repeat(128)}`,
      decodeRefusedAt: `$${"[0]".repeat(128)}`,
    },
    {
      name: "boxed opt T",
      schemas: [
        ["c.rec", RecBoxed],
        ["Contract", loaded([{ kind: "opt", inner: 0 }])],
        ["no rec", staticBoxed(260)],
      ],
      // 257 opts: the innermost, absent, at 256.
      accepted: boxedLevels(257),
      refused: boxedLevels(258),
      refusedAt: `$${".some".repeat(257)}`,
      // The decoder's paths carry no `.some` step for an opt.
      decodeRefusedAt: "$",
    },
  ];
  for (const { name, schemas, accepted, refused, refusedAt, decodeRefusedAt } of cases) {
    // The wire bytes, written once through the c.rec schema; the refused
    // value needs a raised maxDepth to be written at all.
    const acceptedWire = bytesOf(encode(schemas[0][1], accepted));
    const refusedWire = bytesOf(encode(schemas[0][1], refused, { maxDepth: 300 }));
    for (const [form, schema] of schemas) {
      const at = `${name} (${form})`;
      assert.deepStrictEqual(validate(schema, accepted), { ok: true }, `validate ${at}`);
      assert.deepStrictEqual(
        validate(schema, refused),
        { ok: false, issues: [{ ...DEPTH_257, path: refusedAt }] },
        `validate ${at}`,
      );
      assert.deepStrictEqual(
        decode(schema, acceptedWire),
        { ok: true, value: accepted },
        `decode ${at}`,
      );
      assert.deepStrictEqual(
        verdict(decode(schema, refusedWire)),
        {
          code: "resource_limit_exceeded",
          path: decodeRefusedAt,
          message: "value_depth limit 256 exceeded",
          resource_limit: DEPTH_257.resource_limit,
        },
        `decode ${at}`,
      );
      if (form !== "no rec") {
        assert.deepStrictEqual(bytesOf(encode(schema, accepted)), acceptedWire, `encode ${at}`);
        assert.deepStrictEqual(
          verdict(encode(schema, refused)),
          {
            code: "resource_limit_exceeded",
            path: refusedAt,
            message: "value_depth limit 256 exceeded",
            resource_limit: DEPTH_257.resource_limit,
          },
          `encode ${at}`,
        );
      }
    }
  }
});

test("a value is charged the same elements with rec and without, one per constructor", () => {
  const nullVec = c.rec(() => c.vec(c.rec(() => c.null)));
  const nulls = new Array<null>(100).fill(null);
  const rows: readonly [string, AnySchema, AnySchema, unknown, number][] = [
    // 40 vecs: 40 constructors.
    ["vec T, 40 levels", RecVec, staticVec(40), vecLevels(40), 40],
    // The vec and its 100 nulls: 101, as candid-core counts the HostValue.
    ["vec null, 100 elements", nullVec, c.vec(c.null), nulls, 101],
  ];
  for (const [name, recursive, plain, value, expected] of rows) {
    const bytes = bytesOf(encode(plain, value));
    for (const [label, schema] of [
      ["rec", recursive],
      ["no rec", plain],
    ] as const) {
      const at = `${name} (${label})`;
      assert.strictEqual(
        leastElements((maxElements) => validate(schema, value, { maxElements })),
        expected,
        `validate ${at}`,
      );
      assert.strictEqual(
        leastElements((maxElements) => encode(schema, value, { maxElements })),
        expected,
        `encode ${at}`,
      );
      assert.strictEqual(
        leastElements((maxElements) => decode(schema, bytes, { maxElements })),
        expected,
        `decode ${at}`,
      );
    }
  }
});

/** `count` variants down arm `a`, the last on the tag-only arm `b`. */
function variantChain(count: number, a: string, b: string): unknown {
  let value: unknown = { tag: b };
  for (let level = 1; level < count; level += 1) {
    value = { tag: a, value };
  }
  return value;
}

test("a tag-only arm's null payload is a level below its variant, as candid-core counts it", () => {
  // variant { a : T; b } nested `count` levels: the variants sit at depths 0
  // to count - 1 and the innermost `null` payload at count, so 256 variants
  // are accepted and 257 refused (validate_depth_variant_null_*).
  const RecVariant: AnySchema = c.rec(() => c.variant({ a: RecVariant, b: c.null }));
  let plain: AnySchema = c.variant({ a: c.null, b: c.null });
  for (let level = 1; level < 257; level += 1) {
    plain = c.variant({ a: plain, b: c.null });
  }
  // Contract field ids: a = 97, b = 98; without a name table the keys are
  // the `_N_` spellings of the ids.
  const fromContract = loaded([
    {
      kind: "variant",
      fields: [
        { id: 97, type: 0 },
        { id: 98, type: 1 },
      ],
    },
    { kind: "primitive", primitive: "null" },
  ]);
  const rows: readonly [string, AnySchema, string, string][] = [
    ["c.rec", RecVariant, "a", "b"],
    ["no rec", plain, "a", "b"],
    ["Contract", fromContract, "_97_", "_98_"],
  ];
  // The refusal is reported at the innermost variant, whose payload it is.
  const refusedAt = `$${".value".repeat(256)}`;
  for (const [name, schema, a, b] of rows) {
    // Elements: the null is spelled by absence and read from nothing, so it
    // is charged no element; each variant is charged one and each examined
    // key one — 3 variants and 5 keys here, with rec, without, or loaded.
    for (const walk of [validate, encode] as const) {
      assert.strictEqual(
        leastElements((maxElements) => walk(schema, variantChain(3, a, b), { maxElements })),
        8,
        `${walk.name} ${name}`,
      );
    }
    assert.deepStrictEqual(validate(schema, variantChain(256, a, b)), { ok: true }, name);
    const refused = validate(schema, variantChain(257, a, b));
    assert.deepStrictEqual(
      refused,
      { ok: false, issues: [{ ...DEPTH_257, path: refusedAt }] },
      name,
    );
    if (name !== "no rec") {
      // encode's type table refuses the 257-deep static schema outright
      // (D1); the recursive ones reach the value walk.
      const bytes = bytesOf(encode(schema, variantChain(256, a, b)));
      assert(decode(schema, bytes).ok, `${name}: 256 variants decode`);
      const encodeRefused = encode(schema, variantChain(257, a, b));
      assert.deepStrictEqual(
        encodeRefused.ok ? undefined : encodeRefused.issues[0].resource_limit,
        DEPTH_257.resource_limit,
        name,
      );
    }
  }
  // decode charges the payload too: 257 wire variants are refused.
  const wire = bytesOf(encode(RecVariant, variantChain(257, "a", "b"), { maxDepth: 257 }));
  const decoded = decode(RecVariant, wire);
  assert.deepStrictEqual(
    decoded.ok ? undefined : decoded.issues[0].resource_limit,
    DEPTH_257.resource_limit,
  );
});

/** `hops` nested `c.rec` wrappers around `inner`. */
function hopChain(hops: number, inner: AnySchema): AnySchema {
  let schema = inner;
  for (let hop = 0; hop < hops; hop += 1) {
    const body = schema;
    schema = c.rec(() => body);
  }
  return schema;
}

test("a rec chain longer than maxDepth is refused, a self-referential one included", () => {
  // The chain's own cap, `maxDepth` consecutive hops, is what makes a
  // mis-built rec terminate now that a hop is not a level.
  const Self: AnySchema = c.rec(() => Self);
  const natBytes = bytesOf(encode(c.nat, 5n));
  const expected = { resource: "value_depth", limit: 256, observed: 257 };
  for (const [walk, result] of [
    ["validate", validate(Self, 5n)],
    ["encode", encode(Self, 5n)],
    ["decode", decode(Self, natBytes)],
  ] as const) {
    assert(!result.ok, walk);
    if (!result.ok) {
      assert.strictEqual(result.issues.length, 1, walk);
      assert.strictEqual(result.issues[0].code, "resource_limit_exceeded", walk);
      assert.strictEqual(result.issues[0].path, "$", walk);
      assert.deepStrictEqual(result.issues[0].resource_limit, expected, walk);
    }
  }
  // Exactly maxDepth hops resolve, one more is refused — and the hops of a
  // chain charge no depth: with maxDepth 4, a vec behind 4 hops holding a
  // nat behind 4 more is two levels deep, and accepted.
  const options = { maxDepth: 4 };
  const deep = hopChain(4, c.vec(hopChain(4, c.nat)));
  assert.deepStrictEqual(validate(deep, [1n], options), { ok: true });
  const bytes = bytesOf(encode(deep, [1n], options));
  assert.deepStrictEqual(decode(deep, bytes, options), { ok: true, value: [1n] });
  const over = hopChain(5, c.nat);
  const overBytes = bytesOf(encode(c.nat, 1n));
  for (const [walk, result] of [
    ["validate", validate(over, 1n, options)],
    ["encode", encode(over, 1n, options)],
    ["decode", decode(over, overBytes, options)],
  ] as const) {
    assert.deepStrictEqual(
      result.ok ? undefined : result.issues[0].resource_limit,
      { resource: "value_depth", limit: 4, observed: 5 },
      walk,
    );
  }
});

test("the chain cap is maxDepth itself: below a schema's hop chain, rec refuses what no rec accepts", () => {
  // A design call (the cap reuses maxDepth rather than a floor of its own),
  // pinned so a change to it is deliberate. At maxDepth 0 a root scalar is
  // accepted, but one hop is already a chain longer than maxDepth; at
  // maxDepth 1 a generated module's two-hop chain is refused the same way.
  // The runtime refused both before issue #231 too.
  const natBytes = bytesOf(encode(c.nat, 5n));
  for (const [maxDepth, hops] of [
    [0, 1],
    [1, 2],
  ] as const) {
    const options = { maxDepth };
    assert.deepStrictEqual(validate(c.nat, 5n, options), { ok: true });
    assert.deepStrictEqual(bytesOf(encode(c.nat, 5n, options)), natBytes);
    assert.deepStrictEqual(decode(c.nat, natBytes, options), { ok: true, value: 5n });
    const chain = hopChain(hops, c.nat);
    for (const [walk, result] of [
      ["validate", validate(chain, 5n, options)],
      ["encode", encode(chain, 5n, options)],
      ["decode", decode(chain, natBytes, options)],
    ] as const) {
      assert.deepStrictEqual(
        result.ok ? undefined : result.issues[0].resource_limit,
        { resource: "value_depth", limit: maxDepth, observed: hops },
        `${walk} at maxDepth ${maxDepth}`,
      );
    }
    // One hop fewer resolves, and the value is judged as without rec.
    assert.deepStrictEqual(validate(hopChain(hops - 1, c.nat), 5n, options), { ok: true });
  }
});

test("decode terminates on an opt-only cycle through the opt's own depth step (#234)", () => {
  // `type T = opt T` read from a wire `nat`: every expected opt auto-wraps
  // the non-opt wire value, so the walk descends one opt per level and
  // nothing on the wire ends it. Each opt is a constructor charged one level
  // below the last, so the walk stops at maxDepth + 1 — having resolved the
  // inner `T` once per opt, at depths 0 to maxDepth.
  const natBytes = bytesOf(encode(c.nat, 5n));
  for (const maxDepth of [256, 10_000]) {
    let resolved = 0;
    const T: AnySchema = c.rec(() => {
      resolved += 1;
      return c.opt(T);
    });
    const result = decode(T, natBytes, { maxDepth });
    assert.deepStrictEqual(result.ok ? undefined : result.issues[0].resource_limit, {
      resource: "value_depth",
      limit: maxDepth,
      observed: maxDepth + 1,
    });
    // One resolution for the root, and one per opt that wraps.
    assert.strictEqual(resolved, maxDepth + 2);
  }
  // The Contract-loaded cycle terminates the same way.
  const fromContract = decode(loaded([{ kind: "opt", inner: 0 }]), natBytes);
  assert.deepStrictEqual(
    fromContract.ok ? undefined : fromContract.issues[0].resource_limit,
    DEPTH_257.resource_limit,
  );
});
