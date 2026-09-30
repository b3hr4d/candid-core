// Regression vectors ported from the downstream library ic-reactor v3.
//
// ic-reactor v3 compared its Candid handling (over `@icp-sdk/core` 6.1.0)
// with the Rust `candid` crate and filed what diverged. The codec those bugs
// lived in is not this one, but ic-reactor 4 takes its Candid layer from this
// package, so each lesson v3 paid for is pinned here, where the codec now
// lives. Every test names its source issue in B3Pay/ic-reactor; the hex is
// the message the source issue describes, as this encoder writes it.
//
// Scope is the wire and domain-value semantics only. The display-layer
// questions in those issues (DisplayReactor's object form for text-keyed
// pairs, its JSON advice, its `_type` discriminator) are ic-reactor's, not
// this package's. #634 items this suite does not repeat because other
// suites already pin them: U3 (more values than types is `invalid_length`,
// codec.test.ts), U7 (truncated blob, issue #128, codec.test.ts), U8
// (missing trailing opt arguments read as null, codec.test.ts), and U5
// (`__proto__` fields, issue #114, crosscheck.test.ts).

import { test } from "node:test";
import assert from "node:assert/strict";

import { encode, encodeArgs, decode, decodeArgs } from "../codec.ts";
import { schemaFromContract } from "../contract.ts";
import { c, type AnySchema, type Schema } from "../schema.ts";

import * as options from "../../tests/goldens/options.ts";

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

function encodesTo(schema: AnySchema, value: unknown, hexText: string): Uint8Array {
  const result = encode(schema as Schema<unknown>, value);
  assert(result.ok, `encode must succeed: ${result.ok ? "" : JSON.stringify(result.issues)}`);
  if (!result.ok) {
    throw new Error("unreachable");
  }
  assert.strictEqual(toHex(result.bytes), hexText);
  return result.bytes;
}

function decodes(schema: AnySchema, bytes: Uint8Array): unknown {
  const result = decode(schema as Schema<unknown>, bytes);
  assert(result.ok, `decode must succeed: ${result.ok ? "" : JSON.stringify(result.issues)}`);
  if (!result.ok) {
    throw new Error("unreachable");
  }
  return result.value;
}

/** `DIDL`, an empty type table, one float64 argument, then `bits` (little-endian). */
function float64Message(bitsHex: string): Uint8Array {
  return fromHex(`4449444c000172${bitsHex}`);
}

/** Little-endian float64 bytes as a 64-bit unsigned bigint. */
function float64Bits(bytes: Uint8Array): bigint {
  let bits = 0n;
  for (let i = 7; i >= 0; i -= 1) {
    bits = (bits << 8n) | BigInt(bytes[i]);
  }
  return bits;
}

/** An IEEE 754 binary64 NaN: all exponent bits set, a nonzero mantissa. */
function isFloat64NaNBits(bits: bigint): boolean {
  return ((bits >> 52n) & 0x7ffn) === 0x7ffn && (bits & ((1n << 52n) - 1n)) !== 0n;
}

test("ic-reactor #634 U1: a leading U+FEFF in text survives the round trip", () => {
  // `@icp-sdk/core`'s decoder dropped it (TextDecoder's BOM stripping). A
  // leading U+FEFF inside a text *value* is data, never a byte-order mark.
  const bytes = encodesTo(c.text, "﻿abc", "4449444c00017106efbbbf616263");
  const value = decodes(c.text, bytes);
  assert.strictEqual(value, "﻿abc");
  assert.strictEqual((value as string).length, 4);
});

test("ic-reactor #634 U2: records whose labels render alike get separate table entries", () => {
  // `@icp-sdk/core` keyed its type table by the type's display name, and
  // `record { "a:nat; b" : nat }` renders exactly like `record { a : nat;
  // b : nat }`, so both arguments shared one entry and Rust refused the
  // message. Two entries here: table index 0 has the one field hash("a:nat;
  // b"), index 1 has the fields a and b. tests/wire_vectors.rs decodes these
  // exact bytes with the `candid` crate, which also writes the same bytes.
  const quoted = c.record({ "a:nat; b": c.nat });
  const plain = c.record({ a: c.nat, b: c.nat });
  const result = encodeArgs([quoted, plain], [{ "a:nat; b": 1n }, { a: 2n, b: 3n }]);
  assert(result.ok);
  if (!result.ok) {
    throw new Error("unreachable");
  }
  assert.strictEqual(toHex(result.bytes), "4449444c026c01f5f8cecf037d6c02617d627d020001010203");
  const decoded = decodeArgs([quoted, plain], result.bytes);
  assert(decoded.ok);
  if (!decoded.ok) {
    throw new Error("unreachable");
  }
  assert.deepStrictEqual(
    decoded.values.map((value) => ({ ...(value as object) })),
    [{ "a:nat; b": 1n }, { a: 2n, b: 3n }],
  );
});

test("ic-reactor #634 U4: service methods are ordered by UTF-8 bytes, not UTF-16 units", () => {
  // U+FF01 is EF BC 81 in UTF-8 and U+1F600 is F0 9F 98 80, so UTF-8 order
  // puts "！" first. In UTF-16, U+1F600 is the surrogate pair D83D DE00,
  // which sorts before FF01; `@icp-sdk/core` wrote that order and Rust
  // refused the type table. tests/wire_vectors.rs decodes these exact bytes
  // with the `candid` crate, which also writes the same bytes.
  const method = c.func([], [], "query");
  const service = c.service({ "\u{1F600}": method, "！": method });
  const bytes = encodesTo(
    service,
    "aaaaa-aa",
    "4449444c02690203efbc810104f09f9880016a0000010101000100",
  );
  // A decoded service reference is the canonical principal text itself.
  assert.strictEqual(decodes(service, bytes), "aaaaa-aa");
});

test("ic-reactor #634 U6: a field or arm named hasOwnProperty encodes and decodes", () => {
  // `@icp-sdk/core` called `x.hasOwnProperty(...)` on the value, which a
  // field of that name shadows ("x.hasOwnProperty is not a function").
  const record = c.record({ hasOwnProperty: c.nat });
  const recordBytes = encodesTo(record, { hasOwnProperty: 5n }, "4449444c016c0181bff6f1057d010005");
  assert.deepStrictEqual({ ...(decodes(record, recordBytes) as object) }, { hasOwnProperty: 5n });

  const variant = c.variant({ hasOwnProperty: c.nat });
  const variantBytes = encodesTo(
    variant,
    { tag: "hasOwnProperty", value: 5n },
    "4449444c016b0181bff6f1057d01000005",
  );
  assert.deepStrictEqual(
    { ...(decodes(variant, variantBytes) as object) },
    {
      tag: "hasOwnProperty",
      value: 5n,
    },
  );
});

test("ic-reactor #634 U9: a tuple decodes a wire record with extra fields", () => {
  // Candid subtyping: the wire `record { nat; text; bool }` is a subtype of
  // the expected `record { nat; text }`, so the extra field is skipped. The
  // `@icp-sdk/core` tuple decoder refused it; Rust accepts it.
  const wide = c.record({ _0_: c.nat, _1_: c.text, _2_: c.bool });
  const bytes = encodesTo(
    wide,
    { _0_: 1n, _1_: "x", _2_: true },
    "4449444c016c03007d0171027e010001017801",
  );
  assert.deepStrictEqual(decodes(c.tuple([c.nat, c.text]), bytes), [1n, "x"]);
});

test("ic-reactor #633: vec record { text; T } keeps repeated keys and their order", () => {
  // v3's display form made these pairs an object, which holds a key once and
  // puts integer-like keys first. The domain value here is the array of
  // pairs, so a second Set-Cookie header and the wire order both survive,
  // and the value goes back as the same bytes.
  const headers = c.vec(c.tuple([c.text, c.text]));
  const headerValue = [
    ["set-cookie", "a=1"],
    ["set-cookie", "b=2"],
    ["content-type", "text/plain"],
  ];
  const headerBytes = encodesTo(
    headers,
    headerValue,
    "4449444c026d016c02007101710100030a7365742d636f6f6b696503613d310a7365742d636f6f6b696503623d32" +
      "0c636f6e74656e742d747970650a746578742f706c61696e",
  );
  const decodedHeaders = decodes(headers, headerBytes);
  assert.deepStrictEqual(decodedHeaders, headerValue);
  encodesTo(headers, decodedHeaders, toHex(headerBytes));

  const metadata = c.vec(c.tuple([c.text, c.nat]));
  const metadataValue = [
    ["10", 1n],
    ["2", 2n],
    ["b", 3n],
    ["1", 4n],
  ];
  const metadataBytes = encodesTo(
    metadata,
    metadataValue,
    "4449444c026d016c020071017d01000402313001013202016203013104",
  );
  const decodedMetadata = decodes(metadata, metadataBytes);
  assert.deepStrictEqual(decodedMetadata, metadataValue);
  encodesTo(metadata, decodedMetadata, toHex(metadataBytes));
});

test("ic-reactor #632: NaN, the infinities, and -0 decode and re-encode to the same bytes", () => {
  // v3 refused to send back a non-finite float it had just decoded, and JSON
  // turned -0 into 0. The codec takes every IEEE value in both directions.
  // These are the bit patterns the Rust `candid` crate writes for f64::NAN
  // and the rest; float32 is the same claim at four bytes.
  const float64Cases: [number, string][] = [
    [NaN, "000000000000f87f"],
    [Infinity, "000000000000f07f"],
    [-Infinity, "000000000000f0ff"],
    [-0, "0000000000000080"],
    [0, "0000000000000000"],
    [1.5, "000000000000f83f"],
  ];
  for (const [value, bits] of float64Cases) {
    const bytes = float64Message(bits);
    const decoded = decodes(c.float64, bytes);
    assert(Object.is(decoded, value), `float64 ${bits} must decode to ${value}`);
    encodesTo(c.float64, decoded, toHex(bytes));
  }
  const float32Cases: [number, string][] = [
    [NaN, "0000c07f"],
    [Infinity, "0000807f"],
    [-Infinity, "000080ff"],
    [-0, "00000080"],
  ];
  for (const [value, bits] of float32Cases) {
    const bytes = fromHex(`4449444c000173${bits}`);
    const decoded = decodes(c.float32, bytes);
    assert(Object.is(decoded, value), `float32 ${bits} must decode to ${value}`);
    encodesTo(c.float32, decoded, toHex(bytes));
  }
});

test("ic-reactor #632: a NaN payload is guaranteed NaN-ness, not its bits", () => {
  // What is and is not guaranteed for a NaN whose payload is not the one
  // above. The domain type is a JS `number`, and ECMA-262 has exactly one NaN
  // value: reading bytes into a number (RawBytesToNumeric) must produce NaN,
  // but writing NaN back (NumericToRawBytes) may use *any* NaN encoding the
  // implementation chooses. V8 uses that latitude, and what it writes depends
  // on what ran earlier in the process, not only on the input. Measured on Node
  // 24.2 (arm64), same bytes in, three processes: `010000000000f87f` kept its
  // payload in a fresh process but came back as `000000000000f87f` through a
  // vec after unrelated codec warm-up; `000000000000f8ff` lost its sign bit
  // through the scalar path after scalar warm-up; the signaling
  // `010000000000f07f` came back quieted (`010000000000f87f`) in a fresh
  // process and unchanged after warm-up. Plain JavaScript with no codec
  // involved (a `DataView` read, a one-element array, a `DataView` write)
  // shows the same. So the exact payload is not pinned in either
  // direction: either assertion would be flaky. What holds on every run is
  // asserted below. A caller that needs NaN payloads must keep the raw bytes;
  // the TypeScript domain value cannot carry them. The canonical quiet NaN in
  // the test above is the fixed point: it is the pattern engines canonicalize
  // to, so it round-trips exactly in every state measured.
  const payloads = [
    "010000000000f87f", // quiet NaN, payload 1
    "000000000000f8ff", // negative quiet NaN
    "ffffffffffffff7f", // quiet NaN, all payload bits set
    "010000000000f07f", // signaling NaN, payload 1
  ];
  for (const bits of payloads) {
    const decoded = decodes(c.float64, float64Message(bits));
    assert(Number.isNaN(decoded), `${bits} must decode to NaN`);
    // Alone and as a vec element: everything before the eight value bytes is
    // exact, and the eight bytes are some NaN. Only the payload is the
    // engine's.
    const cases: [AnySchema, unknown, string][] = [
      [c.float64, decoded, "4449444c000172"],
      [c.vec(c.float64), [decoded], "4449444c016d72010001"],
    ];
    for (const [schema, value, prefix] of cases) {
      const result = encode(schema as Schema<unknown>, value);
      assert(result.ok);
      if (!result.ok) {
        throw new Error("unreachable");
      }
      const written = result.bytes.slice(result.bytes.length - 8);
      assert.strictEqual(toHex(result.bytes.slice(0, result.bytes.length - 8)), prefix);
      assert(
        isFloat64NaNBits(float64Bits(written)),
        `${bits} must re-encode to some NaN, got ${toHex(written)}`,
      );
    }
  }
});

test("ic-reactor #565: an empty vec inside an opt is some, not none", () => {
  // v3 read `[]` at an optional position as none, so an enabled but empty
  // `opt vec principal` (CMC `notify_create_canister`'s controllers) was
  // sent as none. Here `null` is the only none: `[]` is some(empty vec).
  const controllers = c.opt(c.vec(c.principal));
  const some = encodesTo(controllers, [], "4449444c026e016d6801000100");
  assert.deepStrictEqual(decodes(controllers, some), []);
  const none = encodesTo(controllers, null, "4449444c026e016d68010000");
  assert.strictEqual(decodes(controllers, none), null);

  // `[[]]` is some(a vec holding one empty vec), never none or some([]).
  const nested = c.opt(c.vec(c.vec(c.nat)));
  const bytes = encodesTo(nested, [[]], "4449444c036e016d026d7d0100010100");
  assert.deepStrictEqual(decodes(nested, bytes), [[]]);
});

test("ic-reactor #486: opt opt T carries three states, boxed as { some }", () => {
  // `opt opt T` has three values (none, some(none), some(some x)), and
  // canisters use all three: Internet Identity's config and Orbit's
  // `description : opt opt text` mean "keep", "clear", and "set". An opt whose
  // inner type admits null is boxed, `{ some: X } | null`, so the three wire
  // values decode to three distinct domain values that re-encode to their own
  // bytes. The hex is the `candid` crate's own encoding too (the `options`
  // wire golden's `described_*` cases). This test pinned the fail-closed
  // behaviour until boxing landed; both halves are flipped.
  const none = "4449444c026e016e71010000";
  const someNone = "4449444c026e016e7101000100";
  const someSome = "4449444c026e016e71010001010178";

  // 1. Schemas derived from a Contract accept the declaration.
  const loaded = schemaFromContract({
    format: "candid-core",
    format_version: 1,
    semantics_profile: "candid-1",
    canonicalization_profile: "candid-core-canon-1",
    types: [
      { kind: "opt", inner: 1 },
      { kind: "opt", inner: 2 },
      { kind: "primitive", primitive: "text" },
    ],
    declarations: [{ name: "Description", type: 0 }],
  });
  assert(loaded.ok, "the loader accepts opt opt");
  if (!loaded.ok) {
    return;
  }

  // 2. Hand-built, generated, and loaded schemas all carry keep, clear, and
  //    set losslessly, and refuse the collapsed spelling rather than reading
  //    it as "set".
  for (const description of [
    c.opt(c.opt(c.text)),
    options.AliasedOuter as AnySchema,
    loaded.schemas.Description,
  ]) {
    assert.strictEqual(decodes(description, encodesTo(description, null, none)), null);
    assert.deepStrictEqual(decodes(description, encodesTo(description, { some: null }, someNone)), {
      some: null,
    });
    assert.deepStrictEqual(decodes(description, encodesTo(description, { some: "x" }, someSome)), {
      some: "x",
    });
    const collapsed = encode(description as Schema<unknown>, "x");
    assert(!collapsed.ok, "a bare payload is not a present boxed value");
    if (!collapsed.ok) {
      assert.deepStrictEqual(
        [collapsed.issues[0].code, collapsed.issues[0].path],
        ["invalid_type", "$"],
      );
    }
  }
});
