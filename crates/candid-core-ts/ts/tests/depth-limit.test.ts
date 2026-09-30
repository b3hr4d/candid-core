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

import { decode, encode } from "../codec.ts";
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
