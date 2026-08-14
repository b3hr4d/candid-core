// The issue #150 contract, proven against the real SDK: the type surface
// moved to the structural `PrincipalValue`, and the runtime did not move at
// all. Encode and validate accept a genuine `@icp-sdk/core` `Principal`
// instance exactly as before (it satisfies the structural shape), while a
// decoded principal is the minimal carrier the types now honestly describe —
// not a class instance.

import { test } from "node:test";
import assert from "node:assert/strict";

import { Principal } from "@icp-sdk/core/principal";

import { c, type PrincipalValue, type Schema } from "../schema.ts";
import { validate } from "../validate.ts";
import { encode, decode } from "../codec.ts";

const TEXT = "ryjl3-tyaaa-aaaaa-aaaba-cai";

// The exact-type pin: `Schema` is invariant, so this annotation compiles
// only if `c.principal` infers exactly `PrincipalValue` — assignable in both
// directions, the same proof the golden equality gate runs over generated
// modules.
const exactly: Schema<PrincipalValue> = c.principal;
void exactly;

// SDK instances are structurally assignable where the schema types demand a
// PrincipalValue — the encode-side half of the contract, at the type level.
const sdkInstanceIsAPrincipalValue: PrincipalValue = Principal.fromText(TEXT);
void sdkInstanceIsAPrincipalValue;

test("encode and validate accept a real SDK Principal instance", () => {
  const instance = Principal.fromText(TEXT);
  assert.deepStrictEqual(validate(c.principal, instance), { ok: true });
  const encoded = encode(c.principal, instance);
  assert(encoded.ok, "a real Principal instance must encode");
  if (!encoded.ok) {
    return;
  }
  // The bytes are identical to encoding the plain structural carrier: the
  // codec reads only `toText()`, so the two inputs are indistinguishable.
  const structural = encode(c.principal, { toText: () => TEXT });
  assert(structural.ok);
  if (structural.ok) {
    assert.deepStrictEqual(encoded.bytes, structural.bytes);
  }
  const back = decode(c.principal, encoded.bytes);
  assert(back.ok, "the encoded principal must decode");
  if (back.ok) {
    // `decode` hands back `unknown` by design; the decoded principal is the
    // structural carrier the schema types now honestly describe.
    const value = back.value as PrincipalValue;
    assert.strictEqual(value.toText(), TEXT);
    // Never an SDK class instance — exactly what the moved types say, and
    // what they lied about before.
    assert(!(value instanceof Principal));
  }
});
