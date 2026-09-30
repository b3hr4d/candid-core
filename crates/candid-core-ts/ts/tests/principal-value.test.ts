// Issue #187: a principal's domain value is its canonical text, as the
// branded string `Principal`. `principal()` is the one conversion point and
// refuses non-canonical text rather than repairing it; `isPrincipal` is the
// guard; validate and encode accept exactly the same strings. Proven here at
// the exact boundaries of the text form, and against the real
// `@icp-sdk/core` `Principal` (a devDependency only — no shipped module
// imports it), whose instances go through `principal(sdk)` once.
//
// The serialization properties that motivated the change (JSON, structured
// clone, `===`, Map keys, deep equality) are pinned on decoded values in
// codec.test.ts, "principal values".

import { test } from "node:test";
import assert from "node:assert/strict";

import { Principal as SdkPrincipal } from "@icp-sdk/core/principal";

import {
  c,
  isPrincipal,
  principal,
  type FuncValue,
  type Infer,
  type Principal,
  type Schema,
} from "../schema.ts";
import { validate } from "../validate.ts";
import { decode, encode, principalTextFromBytes } from "../codec.ts";

const LEDGER = "ryjl3-tyaaa-aaaaa-aaaba-cai";
// DIDL, no type table, one argument of type principal (0x68), tag 1, a
// 10-byte id: the ledger canister's bytes. The same bytes the structural
// carrier encoded to before this change.
const LEDGER_MESSAGE_HEX = "4449444c000168010a00000000000000020101";

// --- Type level ------------------------------------------------------------

// `Schema` is invariant, so each annotation compiles only if the domain is
// exactly `Principal` — the same proof the golden equality gate runs.
const exactly: Schema<Principal> = c.principal;
const exactService: Schema<Principal> = c.service({});
const serviceDomain: Infer<typeof exactService> = principal(LEDGER);
const reference: FuncValue = { principal: principal(LEDGER), method: "get" };
// A Principal is a string wherever a string is wanted...
const asText: string = principal(LEDGER);
// ...but a string is not a Principal until it has been checked.
// @ts-expect-error  unchecked text does not carry the brand
const unchecked: Principal = "aaaaa-aa";
// An SDK instance is not one either: it converts once, through principal().
// @ts-expect-error  the SDK class is not the domain type
const sdkDirect: Principal = SdkPrincipal.fromText(LEDGER);
const sdkConverted: Principal = principal(SdkPrincipal.fromText(LEDGER));
void [exactly, exactService, serviceDomain, reference, asText, unchecked, sdkDirect, sdkConverted];

// --- An independent model of the text form --------------------------------

const ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

/** Checksum ‖ id bytes of dash-grouped base32, with no canonicity check. */
function rawData(text: string): number[] {
  let buffer = 0;
  let bits = 0;
  const out: number[] = [];
  for (const char of text.split("-").join("")) {
    buffer = (buffer << 5) | ALPHABET.indexOf(char);
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((buffer >> bits) & 0xff);
    }
  }
  return out;
}

/** Dash-grouped lowercase base32 of arbitrary data, checksum not computed. */
function render(data: readonly number[]): string {
  let encoded = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of data) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      encoded += ALPHABET[(buffer >> bits) & 31];
    }
  }
  if (bits > 0) {
    encoded += ALPHABET[(buffer << (5 - bits)) & 31];
  }
  return encoded.match(/.{1,5}/g)?.join("-") ?? "";
}

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

const ID_29 = Uint8Array.from({ length: 29 }, (_, i) => i + 1);
const TEXT_29 = principalTextFromBytes(ID_29);
const TEXT_30 = principalTextFromBytes(Uint8Array.from({ length: 30 }, (_, i) => i + 1));
// The ledger's own bytes under a checksum with one bit flipped: every other
// property of the text (alphabet, grouping, length, padding) is canonical.
const CRC_MISMATCH = (() => {
  const data = rawData(LEDGER);
  data[0] ^= 0x01;
  return render(data);
})();

test("the independent text model agrees with the implementation", () => {
  // Without this, the refusal cases built from it would prove nothing.
  assert.strictEqual(render(rawData(LEDGER)), LEDGER);
  assert.strictEqual(render(rawData(TEXT_29)), TEXT_29);
  assert.strictEqual(TEXT_29.length, 63, "a 29-byte id renders to the 63-character maximum");
  assert.strictEqual(rawData(TEXT_30).length, 34, "the over-long probe really holds 30 id bytes");
  assert(CRC_MISMATCH !== LEDGER);
  assert.strictEqual(CRC_MISMATCH.length, LEDGER.length);
  assert.deepStrictEqual(rawData(CRC_MISMATCH).slice(4), rawData(LEDGER).slice(4));
});

// --- The canonical-text rules ----------------------------------------------

/** Whether `run` throws a `TypeError` — so a failure can name its case. */
function throwsTypeError(run: () => unknown): boolean {
  try {
    run();
  } catch (error) {
    return error instanceof TypeError;
  }
  return false;
}

const ACCEPTED: readonly [string, string][] = [
  ["the management canister (0-byte id)", "aaaaa-aa"],
  ["the anonymous principal (the single byte 0x04)", "2vxsx-fae"],
  ["the ledger canister (10-byte id)", LEDGER],
  ["a 29-byte id, the maximum", TEXT_29],
];

const REFUSED: readonly [string, string][] = [
  ["the empty string", ""],
  ["upper case", "AAAAA-AA"],
  ["one upper-case character", "aaaaA-aa"],
  ["an upper-cased canister id", LEDGER.toUpperCase()],
  ["missing dashes", "aaaaaaa"],
  ["missing dashes in a longer id", LEDGER.split("-").join("")],
  ["a misplaced dash", "aaaa-aaa"],
  ["a leading dash", "-aaaaa-aa"],
  ["a trailing dash", "aaaaa-aa-"],
  ["a doubled dash", "aaaaa--aa"],
  ["non-zero padding bits in the last character", "aaaaa-ab"],
  ["a checksum mismatch", CRC_MISMATCH],
  ["a 30-byte id, one over the maximum", TEXT_30],
  ["too short to hold a checksum", "aaaa-aa"],
  ["a character outside the alphabet", "aaaaa-a1"],
  ["base32 padding", "aaaaa-aa="],
  ["surrounding whitespace", " aaaaa-aa"],
  ["a trailing newline", "aaaaa-aa\n"],
  ["64 characters, one past the longest canonical text", "a".repeat(64)],
  ["a megabyte of text", "a".repeat(1_048_576)],
];

test("principal() accepts canonical text and returns it unchanged", () => {
  for (const [why, text] of ACCEPTED) {
    const value = principal(text);
    assert.strictEqual(value, text, why);
    assert.strictEqual(isPrincipal(text), true, why);
    assert.deepStrictEqual(validate(c.principal, value), { ok: true }, why);
    const encoded = encode(c.principal, value);
    assert(encoded.ok, `${why} must encode`);
    if (encoded.ok) {
      // Decoding always yields the canonical text, identical to the input.
      assert.deepStrictEqual(decode(c.principal, encoded.bytes), { ok: true, value: text }, why);
    }
  }
});

test("principal() refuses non-canonical text with TypeError, never repairs it", () => {
  for (const [why, text] of REFUSED) {
    assert.strictEqual(
      throwsTypeError(() => principal(text)),
      true,
      why,
    );
    // The same text inside an object with toText() is refused the same way.
    assert.strictEqual(
      throwsTypeError(() => principal({ toText: () => text })),
      true,
      why,
    );
    assert.strictEqual(isPrincipal(text), false, why);
    // Validate and encode refuse it too, with one code at one path.
    const validated = validate(c.principal as Schema<unknown>, text);
    assert(!validated.ok, `${why}: validate must refuse`);
    const encoded = encode(c.principal as Schema<unknown>, text);
    assert(!encoded.ok, `${why}: encode must refuse`);
    if (!validated.ok && !encoded.ok) {
      assert.strictEqual(validated.issues[0].code, "invalid_type", why);
      assert.strictEqual(encoded.issues[0].code, "invalid_type", why);
      assert.strictEqual(validated.issues[0].path, "$", why);
      assert.strictEqual(encoded.issues[0].path, "$", why);
    }
  }
  // Refusing is not canonicalizing: the upper-case spelling of a real
  // principal is an error, not the principal.
  assert.throws(() => principal("AAAAA-AA"), /not canonical principal text/);
  assert.throws(() => principal(LEDGER.toUpperCase()), TypeError);
  // A very long input is refused without echoing it back.
  assert.throws(() => principal("a".repeat(1_048_576)), /a 1048576-character string/);
});

test("principal() refuses what is neither text nor an object with toText()", () => {
  for (const input of [5, null, undefined, {}, [], { toText: "aaaaa-aa" }, 5n]) {
    assert.throws(() => principal(input as unknown as string), TypeError);
  }
  // A toText() that does not return a string.
  assert.throws(() => principal({ toText: () => 5 as unknown as string }), /returned number/);
  // isPrincipal is strings only: nothing with toText() is a Principal.
  for (const value of [
    { toText: () => "aaaaa-aa" },
    SdkPrincipal.fromText("aaaaa-aa"),
    5,
    null,
    undefined,
    ["aaaaa-aa"],
  ]) {
    assert.strictEqual(isPrincipal(value), false);
  }
});

test("the 29-byte boundary holds through principal(), validate, encode and decode", () => {
  assert.strictEqual(principal(TEXT_29), TEXT_29);
  const encoded = encode(c.principal, principal(TEXT_29));
  assert(encoded.ok);
  if (encoded.ok) {
    assert.deepStrictEqual(decode(c.principal, encoded.bytes), { ok: true, value: TEXT_29 });
  }
  assert.throws(() => principal(TEXT_30), TypeError);
  // On the wire, a 30-byte id is refused by the decoder itself.
  const long = [0x44, 0x49, 0x44, 0x4c, 0x00, 0x01, 0x68, 0x01, 30, ...new Array(30).fill(1)];
  const refused = decode(c.principal, Uint8Array.from(long));
  assert(!refused.ok);
  if (!refused.ok) {
    assert.strictEqual(refused.issues[0].code, "invalid_principal");
  }
});

test("decoding yields canonical text for every id length, matching the SDK", () => {
  const rand = mulberry32(0x187);
  for (let length = 0; length <= 29; length += 1) {
    for (let round = 0; round < 8; round += 1) {
      const id = Uint8Array.from({ length }, () => Math.floor(rand() * 256));
      const message = Uint8Array.from([
        0x44,
        0x49,
        0x44,
        0x4c,
        0x00,
        0x01,
        0x68,
        0x01,
        length,
        ...id,
      ]);
      const decoded = decode(c.principal, message);
      assert(decoded.ok);
      if (!decoded.ok) {
        continue;
      }
      // The SDK's own rendering of the same bytes is the reference.
      const sdkText = SdkPrincipal.fromUint8Array(id).toText();
      assert.strictEqual(decoded.value, sdkText);
      assert(isPrincipal(decoded.value));
      assert.strictEqual(principal(SdkPrincipal.fromUint8Array(id)), sdkText);
    }
  }
});

// --- Interop with the real SDK ---------------------------------------------

test("an SDK Principal converts through principal() to the same bytes as before", () => {
  const sdk = SdkPrincipal.fromText(LEDGER);
  const converted = principal(sdk);
  assert.strictEqual(converted, LEDGER);
  const encoded = encode(c.principal, converted);
  assert(encoded.ok);
  if (!encoded.ok) {
    return;
  }
  const hex = [...encoded.bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  assert.strictEqual(hex, LEDGER_MESSAGE_HEX);
  // The id on the wire is exactly the SDK's own byte form.
  assert.deepStrictEqual(encoded.bytes.slice(-10), sdk.toUint8Array());
  // The SDK's well-known principals convert to the canonical constants.
  assert.strictEqual(principal(SdkPrincipal.managementCanister()), "aaaaa-aa");
  assert.strictEqual(principal(SdkPrincipal.anonymous()), "2vxsx-fae");
  // And back out: a decoded principal is what the SDK parses.
  const decoded = decode(c.principal, encoded.bytes);
  assert(decoded.ok);
  if (decoded.ok) {
    const back = SdkPrincipal.fromText(decoded.value as Principal);
    assert.deepStrictEqual(back.toUint8Array(), sdk.toUint8Array());
    assert(!((decoded.value as unknown) instanceof SdkPrincipal));
  }
});

test("encode and validate are strict: an unconverted SDK Principal is refused", () => {
  const sdk = SdkPrincipal.fromText(LEDGER);
  const Account = c.record({ owner: c.principal });
  for (const [schema, value, path] of [
    [c.principal, sdk, "$"],
    [c.service({}), sdk, "$"],
    [c.func([], [], "query"), { principal: sdk, method: "m" }, "$.principal"],
    [Account, { owner: sdk }, "$.owner"],
  ] as const) {
    const validated = validate(schema as Schema<unknown>, value);
    const encoded = encode(schema as Schema<unknown>, value);
    assert(!validated.ok && !encoded.ok, "both must refuse an SDK instance");
    if (!validated.ok && !encoded.ok) {
      assert.strictEqual(validated.issues[0].code, "invalid_type");
      assert.strictEqual(encoded.issues[0].code, "invalid_type");
      assert.strictEqual(validated.issues[0].path, path);
      assert.strictEqual(encoded.issues[0].path, path);
    }
  }
  // Converted once at the boundary, the same value is accepted everywhere.
  assert.deepStrictEqual(validate(Account, { owner: principal(sdk) }), { ok: true });
  assert(encode(Account, { owner: principal(sdk) }).ok);
});
