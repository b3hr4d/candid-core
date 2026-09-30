// Issue #191: a declared primitive names only itself, and every `vec nat8` is
// a blob whatever its element is called. `type Byte = nat8` used to turn each
// `blob` in the interface into `number[]` — in the generator and, by the same
// rule, in `schemaFromContract` — so the two agreed with each other and were
// wrong the same way. That is why the generated-versus-loaded crosscheck
// alone cannot pin the fix: this suite states the verdicts outright, for the
// generated module and the loaded schema separately, and shows that the fix
// changes a value's shape and never its bytes.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { decode, encode } from "../codec.ts";
import { schemaFromContract, type FieldNameEntry } from "../contract.ts";
import { c, type AnySchema, type Schema } from "../schema.ts";
import { validate } from "../validate.ts";

import * as fidelity from "../../tests/goldens/fidelity.ts";

const goldens = new URL("../../tests/goldens/", import.meta.url);

function loaded(): { readonly [name: string]: AnySchema } {
  const contract = JSON.parse(
    readFileSync(new URL("fidelity.contract.json", goldens), "utf8"),
  ) as unknown;
  const names = JSON.parse(
    readFileSync(new URL("fidelity.names.json", goldens), "utf8"),
  ) as FieldNameEntry[];
  const built = schemaFromContract(contract, { names });
  assert(built.ok, "schemaFromContract must accept the fidelity golden");
  if (!built.ok) {
    throw new Error("unreachable");
  }
  return built.schemas;
}

const paths: ReadonlyArray<[string, { readonly [name: string]: unknown }]> = [
  ["generated", fidelity],
  ["loaded", loaded()],
];

function raw(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    raw: new Uint8Array([1, 2]),
    bytes: new Uint8Array([3, 4]),
    grid: [new Uint8Array([5]), new Uint8Array(0)],
    maybe: new Uint8Array([6]),
    one: 7,
    plain: 8,
    ...overrides,
  };
}

for (const [path, schemas] of paths) {
  test(`${path}: every blob position is a Uint8Array, behind a nat8 alias or not`, () => {
    const Raw = schemas.Raw as Schema<unknown>;
    assert.deepStrictEqual(validate(Raw, raw()), { ok: true });
    // `raw : blob` and `bytes : vec Byte` are one type, each also nested in
    // `vec`/`opt`; a number array is refused at every one of them.
    for (const field of ["raw", "bytes"]) {
      const refused = validate(Raw, raw({ [field]: [1, 2] }));
      assert(!refused.ok, `${path}: ${field} must refuse a number[]`);
    }
    assert(!validate(Raw, raw({ grid: [[5]] })).ok, `${path}: grid element refuses number[]`);
    assert(!validate(Raw, raw({ maybe: [6] })).ok, `${path}: opt payload refuses number[]`);
    // The alias itself is still a plain number, and a bare nat8 field too.
    assert.deepStrictEqual(validate(schemas.Byte as Schema<unknown>, 255), { ok: true });
    assert(!validate(schemas.Byte as Schema<unknown>, 256).ok);
    assert(!validate(Raw, raw({ one: 256 })).ok);
  });

  test(`${path}: one primitive, several names, no capture`, () => {
    // `Memo` beside a bare `nat64`; `Seconds` and `Millis` beside a bare
    // `int64`; the ICRC-1 `Tokens` and `BlockIndex`. Every position takes
    // exactly its primitive's domain, whichever alias spelled it.
    const ok = (name: string, value: unknown): boolean =>
      validate(schemas[name] as Schema<unknown>, value).ok;
    assert(ok("R", { a: 1n, b: 2n }) && !ok("R", { a: 1n, b: 2 }) && !ok("R", { a: 1, b: 2n }));
    assert(ok("Timing", { started: 1n, elapsed: 2n, raw: -3n }));
    assert(!ok("Timing", { started: 1n, elapsed: 2n, raw: 3 }));
    const account = { owner: "aaaaa-aa", subaccount: null };
    assert(ok("TransferArg", { to: account, amount: 1n, fee: 2n }));
    assert(ok("TransferArg", { to: account, amount: 1n, fee: null }));
    assert(!ok("TransferArg", { to: account, amount: 1, fee: null }));
    assert(ok("TransferResult", { tag: "Ok", value: 7n }));
    assert(ok("Owned", { by: "aaaaa-aa", alias: "2vxsx-fae" }));
    assert(!ok("Owned", { by: "aaaaa-aa", alias: 5 }));
  });
}

test("generated and loaded schemas write and read the same bytes", () => {
  const [generated, dynamic] = paths.map(([, schemas]) => schemas);
  const value = raw();
  const a = encode(generated.Raw as Schema<unknown>, value);
  const b = encode(dynamic.Raw as Schema<unknown>, value);
  assert(a.ok && b.ok);
  if (a.ok && b.ok) {
    assert.strictEqual(hex(a.bytes), hex(b.bytes));
    for (const schemas of [generated, dynamic]) {
      const back = decode(schemas.Raw as Schema<unknown>, a.bytes);
      assert(back.ok);
      if (back.ok) {
        const { raw: r, bytes, maybe } = back.value as Record<string, unknown>;
        assert(
          r instanceof Uint8Array && bytes instanceof Uint8Array && maybe instanceof Uint8Array,
        );
      }
    }
  }
});

test("the fix changes a value's shape and never its bytes", () => {
  // Before #191 a `vec Byte` (and every other blob once a `Byte` was
  // declared) was `c.vec(c.nat8)` and carried a `number[]`; now it is
  // `c.blob()` and carries a `Uint8Array`. Both schemas describe the wire
  // type `vec nat8`, so the same octets are written for the same elements
  // and either schema reads the other's message.
  const before = c.record({ bytes: c.vec(c.nat8), one: c.nat8 });
  const after = c.record({ bytes: c.blob(), one: c.nat8 });
  const oldValue = { bytes: [0, 1, 254, 255], one: 7 };
  const newValue = { bytes: new Uint8Array([0, 1, 254, 255]), one: 7 };
  const oldBytes = encode(before, oldValue);
  const newBytes = encode(after, newValue);
  assert(oldBytes.ok && newBytes.ok);
  if (!oldBytes.ok || !newBytes.ok) {
    return;
  }
  assert.strictEqual(hex(newBytes.bytes), hex(oldBytes.bytes), "identical bytes");
  const readOld = decode(before, newBytes.bytes);
  const readNew = decode(after, oldBytes.bytes);
  assert(readOld.ok && readNew.ok);
  if (readOld.ok && readNew.ok) {
    assert.deepStrictEqual(readOld.value, oldValue, "the old shape is a number[]");
    assert.deepStrictEqual(readNew.value, newValue, "the new shape is a Uint8Array");
  }
  // And the shapes are exclusive: neither schema takes the other's value.
  assert(!validate(after, oldValue).ok);
  assert(!validate(before, newValue).ok);
});

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
