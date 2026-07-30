// The issue #103 conformance gate, TypeScript half.
//
// Four layers, weakest claim to strongest:
// 1. Reference vectors: `candid`-crate bytes (tests/goldens/wire/*.wire.json)
//    must decode to pinned domain values under both the generated golden
//    schema and the dynamically built one, and both schemas must encode the
//    value to identical bytes. The TS encoder's bytes are checked in as
//    *.wire-ts.json (regenerate with UPDATE_GOLDENS=1 npm test) and decoded
//    back through the candid crate by tests/wire_vectors.rs — the reference
//    implementation validates our encoder in the other direction.
// 2. Spec-derived adversarial bytes: truncation, overlong LEB128, hostile
//    type tables, zero-width element bombs, deep nesting — every case must
//    fail closed with the pinned issue, never throw, never hang.
// 3. Subtyping-on-decode: the coercion matrix the spec fixes — opt absorbs
//    content mismatches to null, variants trap on unknown tags, extra wire
//    fields are skipped, missing opt-like fields read as null.
// 4. A deterministic property harness: seeded-PRNG random values over every
//    golden fixture schema must round-trip decode(encode(v)) === v, and
//    arbitrary/mutated byte buffers must produce a result, not an exception.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

import {
  encode,
  encodeArgs,
  decode,
  decodeArgs,
  principalTextFromBytes,
  type CodecCode,
} from "../codec.ts";
import { schemaFromContract, type FieldNameEntry } from "../contract.ts";
import { validate } from "../validate.ts";
import { c, type AnySchema, type Schema } from "../schema.ts";

import * as primitives from "../../tests/goldens/primitives.ts";
import * as collections from "../../tests/goldens/collections.ts";
import * as variants from "../../tests/goldens/variants.ts";
import * as recursion from "../../tests/goldens/recursion.ts";
import * as quoting from "../../tests/goldens/quoting.ts";
import * as deferred from "../../tests/goldens/deferred.ts";

const goldens = new URL("../../tests/goldens/", import.meta.url);

function loadContractSchemas(name: string): { readonly [key: string]: AnySchema } {
  const contract = JSON.parse(
    readFileSync(new URL(`${name}.contract.json`, goldens), "utf8"),
  ) as unknown;
  const names = JSON.parse(
    readFileSync(new URL(`${name}.names.json`, goldens), "utf8"),
  ) as FieldNameEntry[];
  const built = schemaFromContract(contract, { names });
  assert(built.ok, `schemaFromContract must accept the ${name} golden`);
  if (!built.ok) {
    throw new Error("unreachable");
  }
  return built.schemas;
}

function fromHex(hexText: string): Uint8Array {
  const out = new Uint8Array(hexText.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = parseInt(hexText.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function toHex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * Structural comparison form: decoded principals are `{ toText }` closures,
 * which never compare equal as functions, so both sides normalize a
 * principal-shaped object to its canonical text.
 */
function normalize(value: unknown): unknown {
  if (value === null || typeof value !== "object") {
    return value;
  }
  if (value instanceof Uint8Array) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map(normalize);
  }
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record);
  if (keys.length === 1 && keys[0] === "toText" && typeof record.toText === "function") {
    return `principal:${(record as { toText(): string }).toText()}`;
  }
  const out: Record<string, unknown> = {};
  for (const key of keys) {
    out[key] = normalize(record[key]);
  }
  return out;
}

function principal(text: string): { toText(): string } {
  return { toText: () => text };
}

// ---------------------------------------------------------------------------
// 1. Reference vectors
// ---------------------------------------------------------------------------

interface WireCase {
  readonly name: string;
  readonly declaration: string;
  readonly textual: string;
  readonly hex: string;
}

/** The domain value each reference vector must decode to. */
const EXPECTED: Record<string, Record<string, unknown>> = {
  primitives: {
    null: null,
    bool_true: true,
    nat_big: 340_282_366_920_938_463_463_374_607_431_768_211_455n,
    int_negative_big: -170_141_183_460_469_231_731_687_303_715_884_105_728n,
    nat8_max: 255,
    nat16: 65_535,
    nat32: 4_294_967_295,
    nat64_max: 18_446_744_073_709_551_615n,
    int8_min: -128,
    int16_min: -32_768,
    int32_min: -2_147_483_648,
    int64_min: -9_223_372_036_854_775_808n,
    float32: 1.5,
    float64_neg_zero: -0,
    text_unicode: "héllo ☃",
    reserved: null,
    principal_management: principal("aaaaa-aa"),
    principal_anonymous: principal("2vxsx-fae"),
  },
  collections: {
    unit: {},
    pair: [1n, "x"],
    item: { id: 7, label: "seven", payload: Uint8Array.from([1, 2]) },
    maybe_item_none: null,
    maybe_item_some: { id: 7, label: "seven", payload: Uint8Array.from([7]) },
    note_some: "n",
    items: [{ id: 1, label: "a", payload: new Uint8Array(0) }],
    grid: [Uint8Array.from([1]), new Uint8Array(0)],
  },
  variants: {
    status_ok: { tag: "ok" },
    status_busy: { tag: "busy", value: 3 },
    status_failed: { tag: "failed", value: "why" },
    numbered_0: { tag: "_0_" },
    numbered_5: { tag: "_5_", value: 4n },
    tree: {
      tag: "node",
      value: { left: { tag: "leaf", value: 1n }, right: { tag: "leaf", value: 2n } },
    },
  },
  recursion: {
    list_empty: null,
    list_two: { head: 1n, tail: { head: 2n, tail: null } },
    even_two_hops: { next: { next: null } },
  },
  quoting: {
    weird: { "has space": 1n, "naïve": "x", 'quote"mark': true },
  },
  deferred: {
    kept: { value: 1n },
    // Reference values (issue #104): a func is { principal, method }, a
    // service is its principal.
    callback_value: { principal: principal("aaaaa-aa"), method: "go" },
    registry_value: principal("aaaaa-aa"),
  },
};

const GENERATED: Record<string, Record<string, unknown>> = {
  primitives,
  collections,
  variants,
  recursion,
  quoting,
  deferred,
};

for (const fixture of Object.keys(EXPECTED)) {
  test(`reference vectors: ${fixture} decodes, agrees, and re-encodes`, () => {
    const cases = JSON.parse(
      readFileSync(new URL(`wire/${fixture}.wire.json`, goldens), "utf8"),
    ) as WireCase[];
    const generated = GENERATED[fixture];
    const dynamic = loadContractSchemas(fixture);
    const expected = EXPECTED[fixture];
    assert.deepStrictEqual(
      cases.map((entry) => entry.name).sort(),
      Object.keys(expected).sort(),
      "every reference vector needs a pinned expected value",
    );
    const tsEntries: { name: string; hex: string }[] = [];
    for (const wireCase of cases) {
      const genSchema = generated[wireCase.declaration] as AnySchema;
      const dynSchema = dynamic[wireCase.declaration];
      const bytes = fromHex(wireCase.hex);

      const decodedGen = decode(genSchema as Schema<unknown>, bytes);
      assert(decodedGen.ok, `${wireCase.name} must decode (generated schema)`);
      const decodedDyn = decode(dynSchema as Schema<unknown>, bytes);
      assert(decodedDyn.ok, `${wireCase.name} must decode (dynamic schema)`);
      if (!decodedGen.ok || !decodedDyn.ok) {
        continue;
      }
      assert.deepStrictEqual(
        normalize(decodedGen.value),
        normalize(expected[wireCase.name]),
        `${fixture}/${wireCase.name} decoded value`,
      );
      assert.deepStrictEqual(normalize(decodedDyn.value), normalize(decodedGen.value));
      // A decoded value is a valid domain value — the runtime agrees with
      // the codec about what the schema admits.
      assert.deepStrictEqual(validate(genSchema as Schema<unknown>, decodedGen.value), {
        ok: true,
      });

      const encodedGen = encode(genSchema as Schema<unknown>, expected[wireCase.name]);
      assert(encodedGen.ok, `${wireCase.name} must encode (generated schema)`);
      const encodedDyn = encode(dynSchema as Schema<unknown>, expected[wireCase.name]);
      assert(encodedDyn.ok, `${wireCase.name} must encode (dynamic schema)`);
      if (!encodedGen.ok || !encodedDyn.ok) {
        continue;
      }
      assert.strictEqual(
        toHex(encodedDyn.bytes),
        toHex(encodedGen.bytes),
        `${fixture}/${wireCase.name}: dynamic and generated schemas must emit identical bytes`,
      );
      // Byte-exact round trip through our own encoder.
      const again = decode(genSchema as Schema<unknown>, encodedGen.bytes);
      assert(again.ok);
      if (again.ok) {
        assert.deepStrictEqual(normalize(again.value), normalize(expected[wireCase.name]));
      }
      tsEntries.push({ name: wireCase.name, hex: toHex(encodedGen.bytes) });
    }
    const tsGoldenUrl = new URL(`wire/${fixture}.wire-ts.json`, goldens);
    const text = `${JSON.stringify(tsEntries, null, 2)}\n`;
    if (process.env.UPDATE_GOLDENS !== undefined) {
      writeFileSync(tsGoldenUrl, text);
    } else {
      let golden = "";
      try {
        golden = readFileSync(tsGoldenUrl, "utf8");
      } catch {
        assert(false, "missing TS-encoder golden; run UPDATE_GOLDENS=1 npm test");
      }
      assert.strictEqual(
        text,
        golden,
        "TS encoder output diverged from its golden; regenerate deliberately and review",
      );
    }
  });
}

// ---------------------------------------------------------------------------
// 2. Adversarial bytes
// ---------------------------------------------------------------------------

function failsDecode(
  schema: AnySchema,
  bytes: Uint8Array,
  code: CodecCode,
  options: Parameters<typeof decode>[2] = {},
): void {
  const result = decode(schema as Schema<unknown>, bytes, options);
  assert(!result.ok, `expected decode failure with ${code}`);
  if (!result.ok) {
    assert.strictEqual(result.issues[0].code, code, result.issues[0].message);
  }
}

/** Minimal signed-LEB128 bytes of a non-negative number (type indices). */
function slebBytes(value: number): number[] {
  const out: number[] = [];
  let v = value;
  for (;;) {
    const byte = v & 0x7f;
    v >>= 7;
    if (v === 0 && (byte & 0x40) === 0) {
      out.push(byte);
      return out;
    }
    out.push(byte | 0x80);
  }
}

/** DIDL + type table + arg types + value bytes, all from plain byte lists. */
function message(table: number[][], argTypes: number[][], value: number[]): Uint8Array {
  const out = [0x44, 0x49, 0x44, 0x4c, table.length];
  for (const entry of table) {
    out.push(...entry);
  }
  out.push(argTypes.length);
  for (const ref of argTypes) {
    out.push(...ref);
  }
  out.push(...value);
  return Uint8Array.from(out);
}

test("adversarial bytes fail closed with pinned issues", () => {
  // Magic and truncation.
  failsDecode(c.bool, Uint8Array.from([]), "truncated");
  failsDecode(c.bool, Uint8Array.from([0x44, 0x49, 0x44]), "truncated");
  failsDecode(c.bool, Uint8Array.from([0x44, 0x49, 0x44, 0x58, 0x00]), "invalid_magic");
  failsDecode(c.bool, message([], [[0x7e]], []), "truncated");
  failsDecode(c.text, message([], [[0x71]], [0x05, 0x68, 0x69]), "truncated");
  // Overlong LEB128 in a count, a nat value, and an sleb int value.
  failsDecode(c.bool, Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x80, 0x00]), "overlong_leb128");
  failsDecode(c.nat, message([], [[0x7d]], [0x80, 0x00]), "overlong_leb128");
  failsDecode(c.int, message([], [[0x7c]], [0xff, 0x7f]), "overlong_leb128");
  // The type table may not contain primitives, dangling indices, or unknown
  // annotations; inline composites cannot appear in reference position.
  failsDecode(c.bool, message([[0x7e]], [[0x7e]], [0x01]), "malformed_type_table");
  failsDecode(c.bool, message([[0x6e, 0x05]], [[0x7e]], [0x01]), "malformed_type_table");
  failsDecode(c.bool, message([], [[0x6e]], [0x01]), "malformed_type_table");
  failsDecode(
    c.bool,
    message([[0x6a, 0x00, 0x00, 0x01, 0x07]], [[0x7e]], [0x01]),
    "malformed_type_table",
  );
  // Record field ids must be strictly increasing.
  failsDecode(
    c.record({ a: c.bool }),
    message([[0x6c, 0x02, 0x02, 0x7e, 0x01, 0x7e]], [[0x00]], [0x01, 0x01]),
    "malformed_type_table",
  );
  // Tag bytes outside {0, 1}.
  failsDecode(c.bool, message([], [[0x7e]], [0x02]), "invalid_tag_byte");
  failsDecode(c.opt(c.bool), message([[0x6e, 0x7e]], [[0x00]], [0x02]), "invalid_tag_byte");
  // Malformed UTF-8 text.
  failsDecode(c.text, message([], [[0x71]], [0x01, 0xff]), "invalid_utf8");
  failsDecode(c.text, message([], [[0x71]], [0x02, 0xc0, 0xaf]), "invalid_utf8");
  // A variant index beyond the wire arm list.
  failsDecode(
    c.variant({ ok: c.null }),
    message([[0x6b, 0x01, 0x9c, 0xc2, 0x01, 0x7f]], [[0x00]], [0x07]),
    "invalid_length",
  );
  // Opaque reference forms are unsupported in this slice.
  failsDecode(c.principal, message([], [[0x68]], [0x00]), "invalid_principal");
  // Bytes after the value section: no reference sequence is supported.
  failsDecode(c.bool, message([], [[0x7e]], [0x01, 0x99]), "trailing_bytes");
});

test("resource budgets bound hostile wire input", () => {
  // A zero-width-element bomb: vec null claiming 2^28 elements in five bytes.
  const bomb = message([[0x6d, 0x7f]], [[0x00]], [0x80, 0x80, 0x80, 0x80, 0x01]);
  const bombed = decode(c.vec(c.null), bomb, { maxElements: 1000 });
  assert(!bombed.ok);
  if (!bombed.ok) {
    assert.strictEqual(bombed.issues[0].code, "resource_limit_exceeded");
    assert.strictEqual(bombed.issues[0].resource_limit?.resource, "value_elements");
  }
  // Deep wire nesting exhausts the depth budget even when the expected type
  // (reserved) merely skips it.
  const depth = 300;
  const table: number[][] = [];
  for (let i = 0; i < depth; i += 1) {
    const entry = [0x6e];
    if (i === 0) {
      entry.push(0x7f);
    } else {
      entry.push(...slebBytes(i - 1));
    }
    table.push(entry);
  }
  // Built by hand: message()'s single-byte counts cannot carry 300 entries.
  const nestedBytes = [0x44, 0x49, 0x44, 0x4c, 0xac, 0x02];
  for (const entry of table) {
    nestedBytes.push(...entry);
  }
  nestedBytes.push(0x01, 0xab, 0x02);
  for (let i = 0; i < depth; i += 1) {
    nestedBytes.push(0x01);
  }
  const nested = Uint8Array.from(nestedBytes);
  const deep = decode(c.reserved, nested, { maxDepth: 64 });
  assert(!deep.ok);
  if (!deep.ok) {
    assert.strictEqual(deep.issues[0].code, "resource_limit_exceeded");
    assert.strictEqual(deep.issues[0].resource_limit?.resource, "value_depth");
  }
  // An unbounded nat encoding hits the numeric byte cap.
  const longNat = message([], [[0x7d]], [...Array.from({ length: 60 }, () => 0x80), 0x01]);
  const capped = decode(c.nat, longNat, { maxNumericBytes: 50 });
  assert(!capped.ok);
  if (!capped.ok) {
    assert.strictEqual(capped.issues[0].resource_limit?.resource, "numeric_bytes");
  }
  // The input size ceiling fires before any parsing.
  const big = decode(c.bool, message([], [[0x7e]], [0x01]), { maxBytes: 4 });
  assert(!big.ok);
  if (!big.ok) {
    assert.strictEqual(big.issues[0].resource_limit?.resource, "bytes");
  }
  // A type table claiming more entries than the cap fails at the claim.
  const wideTable = decode(
    c.bool,
    Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0xff, 0xff, 0x03]),
    { maxTypeTableEntries: 1000 },
  );
  assert(!wideTable.ok);
  if (!wideTable.ok) {
    assert.strictEqual(wideTable.issues[0].resource_limit?.resource, "type_table_entries");
  }
});

// ---------------------------------------------------------------------------
// 3. Subtyping on decode
// ---------------------------------------------------------------------------

test("subtyping on decode: the coercion matrix", () => {
  // A wire nat is accepted at expected int; the reverse hard-errors.
  const natBytes = encode(c.nat, 5n);
  assert(natBytes.ok);
  if (natBytes.ok) {
    assert.deepStrictEqual(decode(c.int, natBytes.bytes), { ok: true, value: 5n });
  }
  const intBytes = encode(c.int, 5n);
  assert(intBytes.ok);
  if (intBytes.ok) {
    failsDecode(c.nat, intBytes.bytes, "type_mismatch");
  }
  // Extra wire record fields are skipped; missing opt-like fields are null.
  const wide = encode(c.record({ a: c.bool, b: c.text }), { a: true, b: "x" });
  assert(wide.ok);
  if (wide.ok) {
    assert.deepStrictEqual(decode(c.record({ a: c.bool }), wide.bytes), {
      ok: true,
      value: { a: true },
    });
  }
  const narrow = encode(c.record({ a: c.bool }), { a: true });
  assert(narrow.ok);
  if (narrow.ok) {
    assert.deepStrictEqual(
      decode(c.record({ a: c.bool, b: c.opt(c.text) }), narrow.bytes),
      { ok: true, value: { a: true, b: null } },
    );
    failsDecode(c.record({ a: c.bool, b: c.text }), narrow.bytes, "missing_field");
  }
  // Expected opt absorbs: constituent mismatch, absurd pairs, reserved.
  const textBytes = encode(c.text, "hi");
  assert(textBytes.ok);
  if (textBytes.ok) {
    assert.deepStrictEqual(decode(c.opt(c.nat), textBytes.bytes), { ok: true, value: null });
  }
  const someText = encode(c.opt(c.text), "hi");
  assert(someText.ok);
  if (someText.ok) {
    assert.deepStrictEqual(decode(c.opt(c.nat), someText.bytes), { ok: true, value: null });
    // And the auto-wrap: a bare wire value at an expected opt of its type.
    assert.deepStrictEqual(decode(c.opt(c.text), textBytes.ok ? textBytes.bytes : new Uint8Array(0)), {
      ok: true,
      value: "hi",
    });
  }
  const reservedBytes = encode(c.reserved, null);
  assert(reservedBytes.ok);
  if (reservedBytes.ok) {
    assert.deepStrictEqual(decode(c.opt(c.nat), reservedBytes.bytes), {
      ok: true,
      value: null,
    });
    // Any value coerces at expected reserved.
    assert.deepStrictEqual(decode(c.reserved, textBytes.ok ? textBytes.bytes : new Uint8Array(0)), {
      ok: true,
      value: null,
    });
  }
  // Variants trap: an unknown wire tag is a hard error at the variant...
  const okOnly = c.variant({ ok: c.null });
  const withMore = c.variant({ ok: c.null, later: c.nat32 });
  const laterBytes = encode(withMore, { tag: "later", value: 7 });
  assert(laterBytes.ok);
  if (laterBytes.ok) {
    failsDecode(okOnly, laterBytes.bytes, "unknown_variant_tag");
    // ...but an enclosing expected opt absorbs it — the spec's evolution
    // story for variants.
    assert.deepStrictEqual(decode(c.opt(okOnly), laterBytes.bytes), {
      ok: true,
      value: null,
    });
  }
  // Missing trailing arguments follow the record rule.
  const one = encodeArgs([c.bool], [true]);
  assert(one.ok);
  if (one.ok) {
    const optTail = decodeArgs([c.bool, c.opt(c.text)], one.bytes);
    assert.deepStrictEqual(optTail, { ok: true, values: [true, null] });
    const hardTail = decodeArgs([c.bool, c.text], one.bytes);
    assert(!hardTail.ok);
    if (!hardTail.ok) {
      assert.strictEqual(hardTail.issues[0].code, "missing_field");
    }
  }
  // Extra trailing wire arguments are skipped.
  const two = encodeArgs([c.bool, c.text], [true, "x"]);
  assert(two.ok);
  if (two.ok) {
    assert.deepStrictEqual(decodeArgs([c.bool], two.bytes), { ok: true, values: [true] });
  }
});

// ---------------------------------------------------------------------------
// Encode-side strictness
// ---------------------------------------------------------------------------

function failsEncode(schema: AnySchema, value: unknown, code: CodecCode): void {
  const result = encode(schema as Schema<unknown>, value);
  assert(!result.ok, `expected encode failure with ${code}`);
  if (!result.ok) {
    assert.strictEqual(result.issues[0].code, code, result.issues[0].message);
  }
}

test("encode-side strictness decisions", () => {
  // float32 is exact-or-refuse; NaN is representable.
  failsEncode(c.float32, 0.1, "unrepresentable_float32");
  assert(encode(c.float32, Math.fround(0.1)).ok);
  assert(encode(c.float32, Number.NaN).ok);
  // Lone surrogates have no UTF-8 encoding.
  failsEncode(c.text, "a\ud800b", "invalid_text");
  // Non-canonical principal text fails closed.
  failsEncode(c.principal, principal("AAAAA-AA"), "invalid_principal");
  failsEncode(c.principal, principal("aaaaa-ab"), "invalid_principal");
  // Two keys deriving one wire id: hash("a") is 97, the id `_97_` renders.
  failsEncode(c.record({ a: c.nat8, _97_: c.nat8 }), { a: 1, _97_: 2 }, "duplicate_field_id");
  // A hostile getter cannot out-run validation: encode reads once.
  failsEncode(
    c.record({ a: c.bool }),
    { get a(): boolean { throw new Error("boom"); } },
    "unreadable_value",
  );
  // Arity mismatch at the args level.
  const mismatch = encodeArgs([c.bool], [true, false]);
  assert(!mismatch.ok);
  if (!mismatch.ok) {
    assert.strictEqual(mismatch.issues[0].code, "invalid_length");
  }
  // Encode refuses exactly what validate refuses, code for code, on a
  // representative invalid-value matrix.
  const cases: readonly [AnySchema, unknown][] = [
    [c.nat, -1n],
    [c.nat, 5],
    [c.nat8, 256],
    [c.nat8, 1.5],
    [c.int64, 2n ** 63n],
    [c.text, 5],
    [c.record({ a: c.bool }), {}],
    [c.record({ a: c.bool }), { a: true, z: 1 }],
    [c.variant({ ok: c.null }), { tag: "nope" }],
    [c.tuple([c.nat]), [1n, 2n]],
    [c.blob(), [1, 2]],
    [c.empty as AnySchema, 5],
  ];
  for (const [schema, value] of cases) {
    const validated = validate(schema as Schema<unknown>, value);
    const encoded = encode(schema as Schema<unknown>, value);
    assert(!validated.ok && !encoded.ok, "both must refuse");
    if (!validated.ok && !encoded.ok) {
      assert.strictEqual(
        encoded.issues[0].code,
        validated.issues[0].code,
        `codes must agree for ${describeCase(schema, value)}`,
      );
      assert.strictEqual(encoded.issues[0].path, validated.issues[0].path);
    }
  }
});

function describeCase(schema: AnySchema, value: unknown): string {
  return `${(schema as { kind: string }).kind}/${typeof value}`;
}

// ---------------------------------------------------------------------------
// 4. Property harness
// ---------------------------------------------------------------------------

/** Deterministic PRNG (mulberry32): the suite must not flake. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4_294_967_296;
  };
}

interface GenNode {
  readonly kind: string;
  readonly primitive?: string;
  readonly inner?: AnySchema;
  readonly elements?: readonly AnySchema[];
  readonly fields?: { readonly [key: string]: AnySchema };
  readonly arms?: { readonly [key: string]: AnySchema };
  body?(): AnySchema;
}

const TEXT_POOL = ["", "a", "hé", "☃", "tail", "_0_", "line\nbreak"];

function generate(schema: AnySchema, rand: () => number, depth: number): unknown {
  const node = resolveGen(schema);
  switch (node.kind) {
    case "primitive":
      return generatePrimitive(node.primitive as string, rand);
    case "opt":
      if (depth <= 0 || rand() < 0.4) {
        return null;
      }
      return generate(node.inner as AnySchema, rand, depth - 1);
    case "vec": {
      const length = depth <= 0 ? 0 : Math.floor(rand() * 3);
      return Array.from({ length }, () => generate(node.inner as AnySchema, rand, depth - 1));
    }
    case "blob": {
      const length = Math.floor(rand() * 4);
      return Uint8Array.from({ length }, () => Math.floor(rand() * 256));
    }
    case "unit":
      return {};
    case "tuple":
      return (node.elements as readonly AnySchema[]).map((element) =>
        generate(element, rand, depth - 1),
      );
    case "record": {
      const out: Record<string, unknown> = {};
      for (const key of Object.keys(node.fields as object)) {
        out[key] = generate((node.fields as Record<string, AnySchema>)[key], rand, depth - 1);
      }
      return out;
    }
    case "func":
      return {
        principal: randomPrincipal(rand),
        method: ["go", "get", "page_7"][Math.floor(rand() * 3)],
      };
    case "service":
      return randomPrincipal(rand);
    case "variant": {
      const arms = node.arms as Record<string, AnySchema>;
      const keys = Object.keys(arms);
      // At the depth floor prefer an arm that terminates: a primitive
      // payload (Tree's `leaf`) over a recursive one (`node`).
      const pick =
        depth <= 0
          ? (keys.find((key) => resolveGen(arms[key]).kind === "primitive") ?? keys[0])
          : keys[Math.floor(rand() * keys.length)];
      const payload = resolveGen(arms[pick]);
      if (payload.kind === "primitive" && payload.primitive === "null") {
        return { tag: pick };
      }
      return { tag: pick, value: generate(arms[pick], rand, depth - 1) };
    }
    default:
      throw new Error(`generator has no case for ${node.kind}`);
  }
}

function resolveGen(schema: AnySchema): GenNode {
  let node = schema as unknown as GenNode;
  while (node.kind === "rec") {
    node = (node as { body(): AnySchema }).body() as unknown as GenNode;
  }
  return node;
}

function randomPrincipal(rand: () => number): { toText(): string } {
  return principal(
    principalTextFromBytes(
      Uint8Array.from({ length: Math.floor(rand() * 8) }, () => Math.floor(rand() * 256)),
    ),
  );
}

function generatePrimitive(name: string, rand: () => number): unknown {
  switch (name) {
    case "null":
    case "reserved":
      return null;
    case "bool":
      return rand() < 0.5;
    case "nat":
      return BigInt(Math.floor(rand() * 2 ** 48)) * BigInt(Math.floor(rand() * 2 ** 20) + 1);
    case "int":
      return BigInt(Math.floor(rand() * 2 ** 48)) - 2n ** 47n;
    case "nat8":
      return Math.floor(rand() * 256);
    case "nat16":
      return Math.floor(rand() * 65_536);
    case "nat32":
      return Math.floor(rand() * 4_294_967_296);
    case "nat64":
      return BigInt(Math.floor(rand() * 2 ** 52));
    case "int8":
      return Math.floor(rand() * 256) - 128;
    case "int16":
      return Math.floor(rand() * 65_536) - 32_768;
    case "int32":
      return Math.floor(rand() * 4_294_967_296) - 2_147_483_648;
    case "int64":
      return BigInt(Math.floor(rand() * 2 ** 52)) - 2n ** 51n;
    case "float32":
      return Math.fround(rand() * 1000 - 500);
    case "float64":
      return rand() * 1e12 - 5e11;
    case "text":
      return TEXT_POOL[Math.floor(rand() * TEXT_POOL.length)];
    case "principal":
      return randomPrincipal(rand);
    default:
      throw new Error(`generator has no case for primitive ${name}`);
  }
}

test("round-trip property: decode(encode(v)) is structural identity", () => {
  const rand = mulberry32(0x103);
  let checked = 0;
  for (const fixture of Object.keys(GENERATED)) {
    const moduleExports = GENERATED[fixture];
    for (const declaration of Object.keys(moduleExports)) {
      const schema = moduleExports[declaration] as AnySchema;
      const node = resolveGen(schema);
      if (node.kind === "primitive" && node.primitive === "empty") {
        continue;
      }
      for (let i = 0; i < 40; i += 1) {
        const value = generate(schema, rand, 5);
        const validated = validate(schema as Schema<unknown>, value);
        assert(validated.ok, `generated value must validate (${fixture}.${declaration})`);
        const encoded = encode(schema as Schema<unknown>, value);
        assert(encoded.ok, `generated value must encode (${fixture}.${declaration})`);
        if (!encoded.ok) {
          continue;
        }
        const decoded = decode(schema as Schema<unknown>, encoded.bytes);
        assert(decoded.ok, `round trip must decode (${fixture}.${declaration})`);
        if (decoded.ok) {
          // reserved decodes any value to null — the one legal asymmetry.
          const expectedBack =
            node.kind === "primitive" && node.primitive === "reserved" ? null : value;
          assert.deepStrictEqual(
            normalize(decoded.value),
            normalize(expectedBack),
            `${fixture}.${declaration} round trip`,
          );
        }
        checked += 1;
      }
    }
  }
  assert(checked > 1000, `property harness must actually run (${checked} checks)`);
});

test("arbitrary and mutated bytes never throw and always answer", () => {
  const rand = mulberry32(0xdead);
  const schemas: AnySchema[] = [
    c.bool,
    c.nat,
    c.text,
    c.opt(c.nat),
    c.vec(c.text),
    c.blob(),
    c.record({ a: c.bool, b: c.opt(c.text) }),
    c.variant({ ok: c.null, err: c.text }),
    variants.Tree as AnySchema,
    recursion.List as AnySchema,
  ];
  // Purely random buffers.
  for (let i = 0; i < 500; i += 1) {
    const length = Math.floor(rand() * 200);
    const bytes = Uint8Array.from({ length }, () => Math.floor(rand() * 256));
    const schema = schemas[Math.floor(rand() * schemas.length)];
    const result = decode(schema as Schema<unknown>, bytes);
    assert(typeof result.ok === "boolean");
  }
  // Single-byte mutations of valid encodings: still a result, never a throw.
  const seedValues: [AnySchema, unknown][] = [
    [c.record({ a: c.bool, b: c.opt(c.text) }), { a: true, b: "hi" }],
    [variants.Tree as AnySchema, { tag: "node", value: { left: { tag: "leaf", value: 1n }, right: { tag: "leaf", value: 2n } } }],
    [recursion.List as AnySchema, { head: 1n, tail: { head: 2n, tail: null } }],
  ];
  for (const [schema, value] of seedValues) {
    const encoded = encode(schema as Schema<unknown>, value);
    assert(encoded.ok);
    if (!encoded.ok) {
      continue;
    }
    for (let i = 0; i < 200; i += 1) {
      const mutated = Uint8Array.from(encoded.bytes);
      const at = Math.floor(rand() * mutated.length);
      mutated[at] = Math.floor(rand() * 256);
      const result = decode(schema as Schema<unknown>, mutated);
      assert(typeof result.ok === "boolean");
      if (result.ok) {
        // A mutation that still decodes must still satisfy the schema.
        assert.deepStrictEqual(
          validate(schema as Schema<unknown>, result.value),
          { ok: true },
          `mutated decode at byte ${at} produced an invalid domain value`,
        );
      }
    }
  }
});

// ---------------------------------------------------------------------------
// Review-cycle regressions (pre-publication adversarial review)
// ---------------------------------------------------------------------------

test("large messages encode: no engine argument-limit cliff", () => {
  // push(...body) blew V8's argument limit at ~124k elements, reporting a
  // well-formed 150 KB blob as a hostile value.
  const big = new Uint8Array(300_000).fill(7);
  const encoded = encode(c.blob(), big);
  assert(encoded.ok, "a 300 KB blob is a well-formed message");
  if (encoded.ok) {
    const back = decode(c.blob(), encoded.bytes);
    assert(back.ok);
    if (back.ok) {
      assert.deepStrictEqual(back.value, big);
    }
  }
});

test("a megabyte nat decodes in linear-ish time, not quadratic", () => {
  // The linear bigint fold made maxNumericBytes a CPU budget (~90 s at the
  // 1 MiB default). This is a fail-closed DoS regression gate with two
  // orders of magnitude headroom over the halving implementation (~50 ms),
  // not a performance measurement in the #39 sense.
  const groups = 1_048_575;
  const bytes = new Uint8Array(4 + 1 + 1 + 1 + groups + 1);
  bytes.set([0x44, 0x49, 0x44, 0x4c, 0x00, 0x01, 0x7d], 0);
  for (let i = 0; i < groups; i += 1) {
    bytes[7 + i] = 0x81;
  }
  bytes[7 + groups] = 0x01;
  const started = Date.now();
  const result = decode(c.nat, bytes);
  const elapsed = Date.now() - started;
  assert(result.ok, "a maximal-length nat within the cap must decode");
  assert(elapsed < 10_000, `quadratic regression: ${elapsed} ms for 1 MiB nat`);
});

test("a missing mandatory field is absorbed by an enclosing expected opt", () => {
  // The spec's opt fallback rule: a record that cannot coerce (missing
  // non-optional field included) reads as null under an expected opt — the
  // evolution story for narrowed records. Reference (candid 0.10.30):
  // Decode!(bytes-of-empty-record, Option<RecX>) == Ok(None).
  const emptyRecord = encode(c.unit(), {});
  assert(emptyRecord.ok);
  if (emptyRecord.ok) {
    assert.deepStrictEqual(decode(c.opt(c.record({ a: c.nat })), emptyRecord.bytes), {
      ok: true,
      value: null,
    });
    assert.deepStrictEqual(decode(c.opt(c.tuple([c.nat8])), emptyRecord.bytes), {
      ok: true,
      value: null,
    });
    // At top level it stays the hard error it always was.
    failsDecode(c.record({ a: c.nat }), emptyRecord.bytes, "missing_field");
  }
});

test("absorbed attempts stay charged against the element budget", () => {
  // Refunding a failed absorption attempt would multiply the traversal
  // budget by the nesting depth. The wire empty record at expected
  // opt(record{a}) costs: opt step + record attempt + field null + skip —
  // more than two elements, so a budget of two must fail closed.
  const emptyRecord = encode(c.unit(), {});
  assert(emptyRecord.ok);
  if (emptyRecord.ok) {
    const short = decode(c.opt(c.record({ a: c.nat })), emptyRecord.bytes, {
      maxElements: 2,
    });
    assert(!short.ok, "the absorbed attempt's work must count");
    if (!short.ok) {
      assert.strictEqual(short.issues[0].code, "resource_limit_exceeded");
      assert.strictEqual(short.issues[0].resource_limit?.resource, "value_elements");
    }
  }
});

test("encode and validate agree at the element-budget boundary", () => {
  // Type-table construction is type-graph work and must not consume the
  // value-walk budget, or encode would refuse near-limit values validate
  // accepts. Find validate's minimum passing budget; encode must pass there.
  const schema = recursion.List as AnySchema;
  const value = { head: 1n, tail: { head: 2n, tail: { head: 3n, tail: null } } };
  let minimum = 0;
  for (let budget = 1; budget < 64; budget += 1) {
    if (validate(schema as Schema<unknown>, value, { maxElements: budget }).ok) {
      minimum = budget;
      break;
    }
  }
  assert(minimum > 0, "the sweep must find validate's boundary");
  assert(
    encode(schema as Schema<unknown>, value, { maxElements: minimum }).ok,
    `encode must pass at validate's boundary (${minimum})`,
  );
  assert(!encode(schema as Schema<unknown>, value, { maxElements: minimum - 1 }).ok);
});

test("encode's first issue path matches validate's in declaration order", () => {
  // hash("a") < hash("b"), so id order and declaration order disagree; the
  // first missing field reported must be declaration-first for both.
  const schema = c.record({ b: c.bool, a: c.bool });
  const validated = validate(schema, {} as never);
  const encoded = encode(schema, {});
  assert(!validated.ok && !encoded.ok);
  if (!validated.ok && !encoded.ok) {
    assert.strictEqual(encoded.issues[0].path, validated.issues[0].path);
    assert.strictEqual(encoded.issues[0].path, "$.b");
  }
  // Tag-only variant carrying both a stray key and an explicit value key:
  // enumeration order decides, identically on both sides.
  const variantSchema = c.variant({ ok: c.null });
  const hostile = { tag: "ok", zzz: 1, value: 2 };
  const validatedVariant = validate(variantSchema, hostile as never);
  const encodedVariant = encode(variantSchema, hostile);
  assert(!validatedVariant.ok && !encodedVariant.ok);
  if (!validatedVariant.ok && !encodedVariant.ok) {
    assert.strictEqual(encodedVariant.issues[0].path, validatedVariant.issues[0].path);
    assert.strictEqual(encodedVariant.issues[0].code, validatedVariant.issues[0].code);
  }
});

test("a length-mutating element getter cannot desynchronize the vec count", () => {
  const hostile: unknown[] = [true, true];
  Object.defineProperty(hostile, 0, {
    get(): boolean {
      hostile.length = 1;
      return true;
    },
  });
  const result = encode(c.vec(c.bool), hostile);
  // The written count is snapshotted before the reads, so the shrunken
  // array simply fails validation at the now-missing element — the encoder
  // never emits a count that disagrees with the elements it wrote.
  assert(!result.ok);
});

test("hostile type-table internals fail closed", () => {
  // A func entry whose argument type points outside the table.
  failsDecode(
    c.bool,
    message([[0x6a, 0x01, 0x05, 0x00, 0x00]], [[0x7e]], [0x01]),
    "malformed_type_table",
  );
  // Duplicate (equal) field ids are as malformed as decreasing ones.
  failsDecode(
    c.record({ a: c.bool }),
    message([[0x6c, 0x02, 0x02, 0x7e, 0x02, 0x7e]], [[0x00]], [0x01, 0x01]),
    "malformed_type_table",
  );
});

test("duplicate derived ids fail closed on the decode side too", () => {
  // hash("a") is 97 and `_97_` renders id 97: the same collision encode
  // refuses must not decode by enumeration luck — record and variant alike.
  const emptyRecord = encode(c.unit(), {});
  assert(emptyRecord.ok);
  if (emptyRecord.ok) {
    failsDecode(
      c.record({ a: c.nat8, _97_: c.nat8 }),
      emptyRecord.bytes,
      "duplicate_field_id",
    );
  }
  failsDecode(
    c.variant({ a: c.null, _97_: c.null }),
    message([[0x6b, 0x01, 0x61, 0x7f]], [[0x00]], [0x00]),
    "duplicate_field_id",
  );
});

test("a skipped wire variant with an out-of-range index fails with its own issue", () => {
  // Expected unit ignores the wire field, but the skip walk still parses:
  // index 7 into a one-arm variant is invalid_length, not an accident of
  // undefined comparisons.
  const result = decode(
    c.unit(),
    message(
      [
        [0x6c, 0x01, 0x00, 0x01],
        [0x6b, 0x01, 0x00, 0x7f],
      ],
      [[0x00]],
      [0x07],
    ),
  );
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues[0].code, "invalid_length");
  }
});

// ---------------------------------------------------------------------------
// PR #118 review-bot regressions (each rejection verified against the
// reference implementation, candid 0.10.30, before fixing)
// ---------------------------------------------------------------------------

test("encode enforces the numeric byte cap on unbounded nat and int", () => {
  const capped = encode(c.nat, 128n, { maxNumericBytes: 1 });
  assert(!capped.ok, "128 needs two LEB groups");
  if (!capped.ok) {
    assert.strictEqual(capped.issues[0].resource_limit?.resource, "numeric_bytes");
  }
  const cappedInt = encode(c.int, -(2n ** 100n), { maxNumericBytes: 5 });
  assert(!cappedInt.ok);
  // The bounded emitters stay byte-compatible: a large value inside the cap
  // round-trips, and small edge encodings are unchanged minimal forms.
  const big = 2n ** 7000n - 3n;
  const encoded = encode(c.nat, big);
  assert(encoded.ok);
  if (encoded.ok) {
    assert.deepStrictEqual(decode(c.nat, encoded.bytes), { ok: true, value: big });
  }
  for (const edge of [0n, 63n, 64n, 127n, 128n, -1n, -64n, -65n, -128n]) {
    const schema = edge < 0n ? c.int : c.nat;
    const bytes = encode(schema, edge);
    assert(bytes.ok);
    if (bytes.ok) {
      assert.deepStrictEqual(decode(schema, bytes.bytes), { ok: true, value: edge });
    }
  }
});

test("a function type with more than one annotation fails closed", () => {
  // Reference: REJECTED (two annotations), ACCEPTED (one).
  const two = decodeArgs([], Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x01, 0x6a, 0x00, 0x00, 0x02, 0x01, 0x02, 0x00]));
  assert(!two.ok);
  if (!two.ok) {
    assert.strictEqual(two.issues[0].code, "malformed_type_table");
  }
  const one = decodeArgs([], Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x01, 0x6a, 0x00, 0x00, 0x01, 0x01, 0x00]));
  assert(one.ok, "one annotation is the accepted form");
});

test("service type-table entries are validated even when only skipped", () => {
  // Duplicate method names — reference: REJECTED.
  const duplicate = decodeArgs([], Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x02, 0x6a, 0x00, 0x00, 0x00, 0x69, 0x02, 0x01, 0x61, 0x00, 0x01, 0x61, 0x00, 0x00]));
  assert(!duplicate.ok);
  if (!duplicate.ok) {
    assert.strictEqual(duplicate.issues[0].code, "malformed_type_table");
  }
  // Unsorted method names — reference: REJECTED.
  const unsorted = decodeArgs([], Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x02, 0x6a, 0x00, 0x00, 0x00, 0x69, 0x02, 0x01, 0x62, 0x00, 0x01, 0x61, 0x00, 0x00]));
  assert(!unsorted.ok);
  // A method type that is not a function — reference: REJECTED.
  const nonFunc = decodeArgs([], Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x02, 0x6c, 0x00, 0x69, 0x01, 0x01, 0x61, 0x00, 0x00]));
  assert(!nonFunc.ok);
  if (!nonFunc.ok) {
    assert.strictEqual(nonFunc.issues[0].code, "malformed_type_table");
  }
  // Control: sorted unique methods on a func — reference: ACCEPTED.
  const wellFormed = decodeArgs([], Uint8Array.from([0x44, 0x49, 0x44, 0x4c, 0x02, 0x6a, 0x00, 0x00, 0x00, 0x69, 0x02, 0x01, 0x61, 0x00, 0x01, 0x62, 0x00, 0x00]));
  assert(wellFormed.ok);
});

test("the 29-byte principal bound holds on skip paths too", () => {
  // A 30-byte principal id in an extra (skipped) argument — reference:
  // REJECTED; 29 bytes — ACCEPTED.
  const long = [0x44, 0x49, 0x44, 0x4c, 0x00, 0x01, 0x68, 0x01, 30];
  for (let i = 0; i < 30; i += 1) {
    long.push(0);
  }
  const rejected = decodeArgs([], Uint8Array.from(long));
  assert(!rejected.ok);
  if (!rejected.ok) {
    assert.strictEqual(rejected.issues[0].code, "invalid_principal");
  }
  const ok = [0x44, 0x49, 0x44, 0x4c, 0x00, 0x01, 0x68, 0x01, 29];
  for (let i = 0; i < 29; i += 1) {
    ok.push(0);
  }
  assert(decodeArgs([], Uint8Array.from(ok)).ok);
  // The same bound inside a skipped service value's id form.
  const service = [0x44, 0x49, 0x44, 0x4c, 0x02, 0x6a, 0x00, 0x00, 0x00, 0x69, 0x00, 0x01, 0x01, 0x01, 30];
  for (let i = 0; i < 30; i += 1) {
    service.push(0);
  }
  const serviceRejected = decodeArgs([], Uint8Array.from(service));
  assert(!serviceRejected.ok);
  if (!serviceRejected.ok) {
    assert.strictEqual(serviceRejected.issues[0].code, "invalid_principal");
  }
});

test("reference subtyping is checked on decode: mismatches trap, opt absorbs", () => {
  // The spec's one real subtype CHECK during deserialisation. Annotation
  // sets must be equal: a query func on the wire is not an update func.
  const funcValue = { principal: principal("aaaaa-aa"), method: "go" };
  const asQuery = encode(c.func([c.nat], [c.text], "query"), funcValue);
  assert(asQuery.ok);
  if (asQuery.ok) {
    const same = decode(c.func([c.nat], [c.text], "query"), asQuery.bytes);
    assert(same.ok, "an equal signature decodes");
    if (same.ok) {
      assert.deepStrictEqual(normalize(same.value), normalize(funcValue));
    }
    failsDecode(c.func([c.nat], [c.text], "update"), asQuery.bytes, "type_mismatch");
    // Param contravariance: a wire func taking int serves where nat is
    // expected (callers pass nats; nat <: int), not the reverse.
    const asInt = encode(c.func([c.int], [c.text], "query"), funcValue);
    assert(asInt.ok);
    if (asInt.ok) {
      assert(decode(c.func([c.nat], [c.text], "query"), asInt.bytes).ok);
    }
    failsDecode(c.func([c.int], [c.text], "query"), asQuery.bytes, "type_mismatch");
    // Result covariance: a wire nat result serves an expected int result.
    const natResult = encode(c.func([], [c.nat], "query"), funcValue);
    assert(natResult.ok);
    if (natResult.ok) {
      assert(decode(c.func([], [c.int], "query"), natResult.bytes).ok);
      failsDecode(c.func([], [c.nat8], "query"), natResult.bytes, "type_mismatch");
    }
    // The enclosing-opt absorption the coercion relation mandates.
    assert.deepStrictEqual(
      decode(c.opt(c.func([c.nat], [c.text], "update")), asQuery.bytes),
      { ok: true, value: null },
    );
  }
  // Service width: an expected method the wire type lacks is a mismatch.
  const narrow = c.service({ ping: c.func([], [], "update") });
  const wide = c.service({
    ping: c.func([], [], "update"),
    stop: c.func([], [], "update"),
  });
  const serviceValue = principal("aaaaa-aa");
  const narrowBytes = encode(narrow, serviceValue);
  assert(narrowBytes.ok);
  if (narrowBytes.ok) {
    failsDecode(wide, narrowBytes.bytes, "type_mismatch");
  }
  const wideBytes = encode(wide, serviceValue);
  assert(wideBytes.ok);
  if (wideBytes.ok) {
    assert(decode(narrow, wideBytes.bytes).ok, "a wider wire service serves");
  }
});
