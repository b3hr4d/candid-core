// The depth property of issue #192 (decision D1 as the maintainer settled it):
// any type the candid-core compiler accepts encodes through its generated
// module at the default limits, however it is split into declarations.
//
// The encoder's type-table walk charges `maxDepth` for Candid nesting depth
// only — one unit per combinator level at which it opens an entry — and
// nothing for `rec` hops, which are aliases and lazy edges. That is the count
// the compiler bounds with `max_type_depth` (256), where an alias adds no
// depth either. `tests/depth_limit.rs` builds each shape at the compiler's
// maximum (a composite at Candid depth 256: inline, through 250 aliases, and
// through a chain cycling every composite kind), asserts the compiler accepts
// it and refuses one level more, and emits the generated module and the
// Contract envelope into `tests/goldens/depth/`. This file walks them at the
// default limits, pins the encoder's own refusal point for hand-built
// schemas, and pins the work a hostile hand-built schema can cost.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { decode, encode, encodeArgs } from "../codec.ts";
import { schemaFromContract } from "../contract.ts";
import { c, type AnySchema, type Schema } from "../schema.ts";
import { validate } from "../validate.ts";

import * as aliases from "../../tests/goldens/depth/aliases.ts";
import * as mixed from "../../tests/goldens/depth/mixed.ts";

// Loaded by a computed specifier so the type checker does not follow it: it
// cannot prove the generated equality annotation of a type nested 256 levels
// inline (see `exclude` in tsconfig.json). At runtime it is an ordinary
// generated module.
const inlineUrl = new URL("../../tests/goldens/depth/inline.ts", import.meta.url).href;
const inline = (await import(inlineUrl)) as { readonly Deep: AnySchema };

function loaded(name: string): { readonly [declaration: string]: AnySchema } {
  const envelope = JSON.parse(
    readFileSync(
      new URL(`../../tests/goldens/depth/${name}.envelope.json`, import.meta.url),
      "utf8",
    ),
  ) as unknown;
  const built = schemaFromContract(envelope);
  if (!built.ok) {
    throw new Error(`${name}: the Contract must load: ${JSON.stringify(built.issues)}`);
  }
  return built.schemas;
}

type Issue = { readonly resource_limit?: unknown; readonly path: string; readonly code: string };

function only(result: { readonly ok: boolean; readonly issues?: readonly Issue[] }): Issue {
  assert.strictEqual(result.ok, false);
  assert.strictEqual(result.issues?.length, 1, "exactly one issue");
  return (result.issues as readonly Issue[])[0];
}

test("every shape at the compiler's maximum depth validates, encodes and decodes at default limits", () => {
  const cases: readonly [string, AnySchema, AnySchema, unknown][] = [
    ["inline (256 levels)", inline.Deep as AnySchema, loaded("inline").Deep, []],
    ["250 aliases, then inline", aliases.A0 as AnySchema, loaded("aliases").A0, []],
    ["every composite kind", mixed.M0 as AnySchema, loaded("mixed").M0, { next: null }],
  ];
  for (const [name, generated, fromContract, value] of cases) {
    const bytes: Uint8Array[] = [];
    for (const [how, schema] of [
      ["generated", generated],
      ["loaded", fromContract],
    ] as const) {
      const label = `${name}, ${how}`;
      assert.deepStrictEqual(validate(schema as Schema<unknown>, value), { ok: true }, label);
      const encoded = encode(schema as Schema<unknown>, value);
      if (!encoded.ok) {
        throw new Error(`${label} must encode: ${JSON.stringify(encoded.issues)}`);
      }
      assert.deepStrictEqual(
        decode(schema as Schema<unknown>, encoded.bytes),
        { ok: true, value },
        label,
      );
      bytes.push(encoded.bytes);
    }
    // One message whichever way the schema was built.
    assert.deepStrictEqual(bytes[1], bytes[0], name);
  }
});

function vecs(levels: number, leaf: AnySchema): AnySchema {
  let schema = leaf;
  for (let level = 0; level < levels; level += 1) {
    schema = c.vec(schema);
  }
  return schema;
}

/** One `rec` alias per level, as a generated module spells a chain of declarations. */
function aliasedVecs(levels: number, leaf: AnySchema): AnySchema {
  let schema = leaf;
  for (let level = 0; level < levels; level += 1) {
    const inner = schema;
    schema = c.rec(() => c.vec(inner));
  }
  return schema;
}

const DEPTH_257 = { resource: "value_depth", limit: 256, observed: 257 };

test("the encoder refuses at Candid depth 257, and a rec hop is not a level", () => {
  // A composite at Candid depth 256 — 256 vecs around an empty record, the
  // compiler's own maximum — encodes; one more is refused at the first entry
  // past the limit. A `rec` around every level changes neither answer.
  for (const build of [vecs, aliasedVecs]) {
    assert(encode(build(256, c.unit()) as Schema<unknown>, []).ok, build.name);
    const refused = only(encode(build(257, c.unit()) as Schema<unknown>, []));
    assert.strictEqual(refused.path, "$", build.name);
    assert.deepStrictEqual(refused.resource_limit, DEPTH_257, build.name);
  }
  // Primitives open no entry: 257 vecs around a nat still end in an entry at
  // depth 256.
  assert(encode(vecs(257, c.nat) as Schema<unknown>, []).ok);
  // The limit is the call's own maxDepth.
  assert(encode(vecs(10, c.unit()) as Schema<unknown>, [], { maxDepth: 10 }).ok);
  assert.deepStrictEqual(
    only(encode(vecs(11, c.unit()) as Schema<unknown>, [], { maxDepth: 10 })).resource_limit,
    { resource: "value_depth", limit: 10, observed: 11 },
  );
});

/** A `rec` chain of `length` hops, built lazily, counting its thunk calls. */
function thunkChain(length: number, end: () => AnySchema, calls: { count: number }): AnySchema {
  const hop = (index: number): AnySchema =>
    c.rec(() => {
      calls.count += 1;
      return index + 1 < length ? hop(index + 1) : end();
    });
  return hop(0);
}

test("hostile hand-built schemas are refused after bounded, pinned work", () => {
  for (const maxDepth of [256, 10_000]) {
    const limit = { resource: "value_depth", limit: maxDepth, observed: maxDepth + 1 };

    // Deep combinators: 20,000 nested vecs open maxDepth + 1 entries and stop.
    const deep = vecs(Math.max(20_000, maxDepth * 2), c.nat) as Schema<unknown>;
    assert.deepStrictEqual(only(encode(deep, [], { maxDepth })).resource_limit, limit);
    assert.deepStrictEqual(
      only(encode(deep, [], { maxDepth, maxTypeTableEntries: maxDepth + 1 })).resource_limit,
      limit,
      "exactly maxDepth + 1 entries were opened",
    );
    assert.deepStrictEqual(
      only(encode(deep, [], { maxDepth, maxTypeTableEntries: maxDepth })).resource_limit,
      { resource: "type_table_entries", limit: maxDepth, observed: maxDepth + 1 },
    );

    // A thunk chain a million hops long: refused at hop maxDepth + 1, after
    // exactly maxDepth thunk calls.
    const chainCalls = { count: 0 };
    const chain = thunkChain(1_000_000, () => c.nat, chainCalls) as Schema<unknown>;
    assert.deepStrictEqual(only(encode(chain, null, { maxDepth })).resource_limit, limit);
    assert.strictEqual(chainCalls.count, maxDepth);
  }

  // Mixed, the worst case per path: every level a full chain of 256 hops
  // (the most one reference may take) around a fresh vec, so neither memo
  // nor the chain cap stops it early. Levels 0 to 256 are walked, the 258th
  // chain resolves and its vec is refused at depth 257: 258 × 256 thunk calls
  // and 257 entries — bounded by maxDepth × (maxDepth + 2), whatever lies
  // beyond.
  const mixedCalls = { count: 0 };
  const level = (): AnySchema => thunkChain(256, () => c.vec(level()), mixedCalls);
  const hostile = level() as Schema<unknown>;
  assert.deepStrictEqual(only(encode(hostile, null)).resource_limit, DEPTH_257);
  assert.strictEqual(mixedCalls.count, 258 * 256);
  mixedCalls.count = 0;
  assert.deepStrictEqual(
    only(encode(hostile, null, { maxTypeTableEntries: 257 })).resource_limit,
    DEPTH_257,
    "exactly 257 entries were opened",
  );

  // One hop too many inside the nesting is refused at that chain.
  const longCalls = { count: 0 };
  const tooLong = c.vec(c.record({ a: thunkChain(257, () => c.nat, longCalls) }));
  assert.deepStrictEqual(only(encode(tooLong as Schema<unknown>, [])).resource_limit, DEPTH_257);
  assert.strictEqual(longCalls.count, 256);
});

test("a reused schema object is charged at every depth it is referenced from", () => {
  // The type table writes an entry once and references it from every later
  // position, but the Candid depth it spans is the deepest position's. The
  // depth of a reuse — first seen shallow, then deep — is charged as if the
  // entry were walked there. (Review of #208: the memo used to answer the
  // deep reference without any charge.)
  const shared = c.unit();
  const deep = vecs(257, shared); // `shared` at Candid depth 257
  assert.deepStrictEqual(only(encode(deep as Schema<unknown>, [])).resource_limit, DEPTH_257);
  // Both argument orders, one message.
  assert.deepStrictEqual(
    only(encodeArgs([shared, deep], [{}, []])).resource_limit,
    DEPTH_257,
    "shared first",
  );
  assert.deepStrictEqual(
    only(encodeArgs([deep, shared], [[], {}])).resource_limit,
    DEPTH_257,
    "shared second",
  );
  // One argument, the same object reached first shallow, then deep.
  const oneArgument = c.record({ a: shared, b: vecs(256, shared) });
  assert.deepStrictEqual(
    only(encode(oneArgument as Schema<unknown>, { a: {}, b: [] })).resource_limit,
    DEPTH_257,
  );
  // A reused subtree spans its own height: two levels reused at 255 reach 257.
  const subtree = vecs(2, c.unit());
  const spanning = c.record({ a: subtree, b: vecs(254, subtree) });
  assert.deepStrictEqual(
    only(encode(spanning as Schema<unknown>, { a: [], b: [] })).resource_limit,
    DEPTH_257,
  );
  // One level shallower, both encode — exactly as unshared schemas would.
  assert(encodeArgs([shared, vecs(256, shared)], [{}, []]).ok);
  assert(
    encode(c.record({ a: subtree, b: vecs(253, subtree) }) as Schema<unknown>, {
      a: [],
      b: [],
    }).ok,
  );
});

type List = { head: bigint; tail: List } | null;
const List: Schema<List> = c.rec(() => c.opt(c.record({ head: c.nat, tail: List })));

test("a recursive knot reused deeper through an alias is charged; its own back edge is not", () => {
  // `List` is an opt around a record whose tail closes the knot. Walked once
  // shallow, then reached again through an alias: the reuse spans the opt and
  // the record (the back edge adds nothing, as Candid does not expand a type
  // inside itself), so an alias at depth 256 puts the record at 257.
  const Alias: Schema<List> = c.rec(() => List);
  const value = { first: null, deep: [] };
  const at = (depth: number): Schema<unknown> =>
    c.record({ first: List, deep: vecs(depth - 1, Alias) }) as Schema<unknown>;
  assert(encode(at(255), value).ok, "the record at 256");
  assert.deepStrictEqual(only(encode(at(256), value)).resource_limit, DEPTH_257);
  // The same knot built fresh at those depths answers the same way.
  const fresh = (depth: number): Schema<unknown> => {
    type Fresh = { head: bigint; tail: Fresh } | null;
    const Knot: Schema<Fresh> = c.rec(() => c.opt(c.record({ head: c.nat, tail: Knot })));
    return c.record({ first: c.null, deep: vecs(depth - 1, Knot) }) as Schema<unknown>;
  };
  assert(encode(fresh(255), value).ok);
  assert.deepStrictEqual(only(encode(fresh(256), value)).resource_limit, DEPTH_257);
  // And a knot deep inside nothing else still encodes and round-trips.
  const list = { head: 1n, tail: { head: 2n, tail: null } };
  const encoded = encode(List, list);
  assert(encoded.ok && decode(List, encoded.bytes).ok);
});

test("reuse stays deduplicated: a thousand references to one deep subtree open it once", () => {
  // 1,000 fields reference one 200-level subtree: 1 + 201 entries, however
  // many references, and each reference costs one height comparison.
  const shared = vecs(200, c.unit());
  const fields: { [key: string]: AnySchema } = {};
  const value: { [key: string]: unknown } = {};
  for (let i = 0; i < 1_000; i += 1) {
    fields[`f${i}`] = shared;
    value[`f${i}`] = [];
  }
  const wide = c.record(fields) as Schema<unknown>;
  assert(encode(wide, value, { maxTypeTableEntries: 202 }).ok);
  assert.deepStrictEqual(only(encode(wide, value, { maxTypeTableEntries: 201 })).resource_limit, {
    resource: "type_table_entries",
    limit: 201,
    observed: 202,
  });
  // 56 levels above the record put every reference's deepest level at 257.
  assert.deepStrictEqual(only(encode(vecs(56, wide), [])).resource_limit, DEPTH_257);
  assert(encode(vecs(55, wide), []).ok);
});
