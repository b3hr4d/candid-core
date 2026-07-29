import { test } from "node:test";
import assert from "node:assert/strict";
import { c } from "../src/schema.ts";
import { decodeBase64, encodeBase64, fromWire, toWire } from "../src/wire.ts";
import { validate } from "../src/validate.ts";
import { principalFromText } from "../src/principal.ts";
import type { Schema } from "../src/schema.ts";

function roundTrip<T>(schema: Schema<T>, value: T): T {
  const wire = toWire(schema, value);
  // The wire value must survive JSON itself, since that is the transport.
  const rehydrated: unknown = JSON.parse(JSON.stringify(wire));
  const decoded = fromWire(schema, rehydrated);
  const safe = (value: unknown): string =>
    JSON.stringify(value, (_key, entry: unknown) =>
      typeof entry === "bigint" ? `${entry}n` : entry,
    );
  assert.equal(decoded.ok, true, safe(decoded));
  if (!decoded.ok) {
    throw new Error("unreachable");
  }
  const checked = validate(schema, decoded.value);
  assert.equal(checked.ok, true, JSON.stringify(checked));
  return decoded.value;
}

test("bigints ride as decimal strings", () => {
  assert.equal(roundTrip(c.nat, 123456789012345678901234567890n), 123456789012345678901234567890n);
  assert.equal(roundTrip(c.int, -42n), -42n);
  assert.equal(toWire(c.nat64, 7n), "7");
});

test("floats ride as numbers; NaN and infinities as tokens", () => {
  assert.equal(roundTrip(c.float64, 1.5), 1.5);
  assert.equal(Number.isNaN(roundTrip(c.float64, Number.NaN)), true);
  assert.equal(roundTrip(c.float32, Number.NEGATIVE_INFINITY), Number.NEGATIVE_INFINITY);
  assert.equal(toWire(c.float64, Number.NaN), "NaN");
});

test("blobs ride as strict base64", () => {
  const bytes = new Uint8Array(256).map((_, index) => index);
  assert.deepEqual(roundTrip(c.blob(), bytes), bytes);
  assert.deepEqual(roundTrip(c.blob(), new Uint8Array()), new Uint8Array());
});

test("base64 decode is strict", () => {
  assert.deepEqual(decodeBase64(encodeBase64(new Uint8Array([0, 255, 7]))), new Uint8Array([0, 255, 7]));
  assert.equal(decodeBase64("AB"), null); // bad length
  assert.equal(decodeBase64("AB=A"), null); // interior padding
  assert.equal(decodeBase64("A==="), null); // over-padding
  assert.equal(decodeBase64("!@#$"), null); // alphabet
  assert.equal(decodeBase64("AB=="), null); // non-canonical trailing bits
  assert.deepEqual(decodeBase64("AA=="), new Uint8Array([0]));
});

test("principals ride as text", () => {
  const value = roundTrip(c.principal, principalFromText("aaaaa-aa"));
  assert.equal(value.toText(), "aaaaa-aa");
});

test("records, tuples, vec, unit, opt round-trip", () => {
  const schema = c.record({
    id: c.nat64,
    pair: c.tuple([c.text, c.bool]),
    tags: c.vec(c.text),
    unit: c.unit(),
    note: c.opt(c.text),
  });
  const value = {
    id: 9n,
    pair: ["x", true] as [string, boolean],
    tags: ["a", "b"],
    unit: {},
    note: null,
  };
  assert.deepEqual(roundTrip(schema, value), value);
  assert.deepEqual(
    roundTrip(schema, { ...value, note: "present" }).note,
    "present",
  );
});

test("variants ride as tag/value with null payloads bare", () => {
  const schema = c.variant({ ok: c.null, busy: c.nat32 });
  assert.deepEqual(toWire(schema, { tag: "ok" }), { tag: "ok" });
  assert.deepEqual(roundTrip(schema, { tag: "busy", value: 3 }), {
    tag: "busy",
    value: 3,
  });
});

test("reserved carries nothing across the wire", () => {
  assert.equal(toWire(c.reserved, { secrets: true }), null);
  const decoded = fromWire(c.reserved, { attacker: "shaped" });
  assert.equal(decoded.ok, true);
  if (decoded.ok) {
    assert.equal(decoded.value, null);
  }
});

test("wire decode fails closed with path-addressed issues", () => {
  const schema = c.record({ id: c.nat64, tags: c.vec(c.text) });
  const decoded = fromWire(schema, { id: 5, tags: ["a", 1] });
  assert.equal(decoded.ok, false);
  if (!decoded.ok) {
    assert.deepEqual(
      decoded.issues.map((issue) => [issue.code, issue.path]),
      [
        ["wire_expected_bigint", "$.id"],
        ["wire_expected_string", "$.tags[1]"],
      ],
    );
  }
  const extra = fromWire(schema, { id: "5", tags: [], bogus: 1 });
  assert.equal(extra.ok, false);
  if (!extra.ok) {
    assert.equal(extra.issues[0]?.code, "wire_unexpected_field");
  }
});

test("decoded values still go through domain validation for ranges", () => {
  const decoded = fromWire(c.nat64, "999999999999999999999999");
  assert.equal(decoded.ok, true);
  if (decoded.ok) {
    const checked = validate(c.nat64, decoded.value);
    assert.equal(checked.ok, false);
  }
});

test("variant tag from the prototype chain fails closed on the wire", () => {
  const schema = c.variant({ ok: c.null });
  const decoded = fromWire(schema, { tag: "hasOwnProperty" });
  assert.equal(decoded.ok, false);
  if (!decoded.ok) {
    assert.equal(decoded.issues[0]?.code, "wire_unknown_variant_tag");
  }
});

test("wire depth is bounded", () => {
  let schema = c.vec(c.nat8) as import("../src/walk.ts").AnySchema;
  let wire: unknown = [1];
  for (let index = 0; index < 80; index += 1) {
    schema = c.vec(schema);
    wire = [wire];
  }
  const decoded = fromWire(schema, wire);
  assert.equal(decoded.ok, false);
  if (!decoded.ok) {
    assert.equal(decoded.issues[0]?.code, "wire_depth_exceeded");
  }
});
