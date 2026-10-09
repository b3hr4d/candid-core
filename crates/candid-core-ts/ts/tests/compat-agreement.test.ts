// The decoder's subtype relation against `checkCompatible`, the service-level
// compatibility check of `@candid-core/cli`.
//
// `tests/goldens/compat/agreement.json` is written by the Rust tests of the
// wasm crate (`crates/candid-core-wasm/tests/compatibility.rs`): hand-written
// cases and a seeded random campaign, each with its written and live
// Contracts and the check's verdict per written method. The two must agree:
// a service reference typed by the live service decodes at the written
// service exactly when the live service is a subtype of it, because decoding
// a reference is where this runtime runs `wireSubtypeOfSchema`. Each method is
// also decoded at a one-method service holding only it, so the verdicts are
// compared method by method, not only as a whole.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { decode, encode } from "../codec.ts";
import { schemaFromContract } from "../contract.ts";
import { c, principal, serviceMethods } from "../schema.ts";
import type { AnySchema } from "../schema.ts";

interface AgreementCase {
  readonly name: string;
  readonly compatible: boolean;
  /** Per written method: is the live method a subtype of the written one? */
  readonly methods: { readonly [method: string]: boolean };
  readonly written: unknown;
  readonly live: unknown;
}

const file = JSON.parse(
  readFileSync(new URL("../../tests/goldens/compat/agreement.json", import.meta.url), "utf8"),
) as { readonly cases: readonly AgreementCase[] };

function actorOf(contract: unknown, name: string): AnySchema {
  const built = schemaFromContract(contract);
  if (!built.ok) {
    throw new Error(`${name}: the Contract does not load: ${JSON.stringify(built.issues)}`);
  }
  assert.deepStrictEqual(built.omitted, [], `${name}: nothing is omitted`);
  if (built.actor === undefined) {
    throw new Error(`${name}: the Contract has no actor`);
  }
  return built.actor;
}

test("the decoder's subtype check agrees with checkCompatible, method by method", () => {
  assert(file.cases.length >= 120, `only ${file.cases.length} cases`);
  let compatible = 0;
  let incompatible = 0;
  for (const entry of file.cases) {
    const written = actorOf(entry.written, `${entry.name} (written)`);
    const live = actorOf(entry.live, `${entry.name} (live)`);
    const encoded = encode(live, principal("aaaaa-aa"));
    if (!encoded.ok) {
      throw new Error(`${entry.name}: the live reference does not encode`);
    }
    const whole = decode(written, encoded.bytes);
    assert.strictEqual(whole.ok, entry.compatible, `${entry.name}: the service-level verdict`);
    if (entry.compatible) {
      compatible += 1;
    } else {
      incompatible += 1;
    }
    const methods = serviceMethods(written);
    for (const [name, verdict] of Object.entries(entry.methods)) {
      const method = methods.get(name);
      if (method === undefined) {
        throw new Error(`${entry.name}: no written method ${name}`);
      }
      const one = c.service({ [name]: c.func(method.args, method.results, method.mode) });
      assert.strictEqual(
        decode(one, encoded.bytes).ok,
        verdict,
        `${entry.name}: method ${name} must decode exactly when it is compatible`,
      );
    }
  }
  assert(compatible >= 40 && incompatible >= 40, `${compatible} / ${incompatible}`);
});
