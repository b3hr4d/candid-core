// The Rust-reference coercion agreement (issue #192, acceptance criterion 3;
// the harness the differential fuzz of #196 reuses).
//
// `tests/wire_vectors.rs` decodes every pinned `(wire types, bytes, expected
// types)` triple with the `candid` crate's `IDLArgs::from_bytes_with_types`
// and writes the verdict — accept with the decoded values, or reject — into
// `tests/goldens/wire/coercion.json`, beside the Contract the expected types
// come from. This file builds the expected schemas from that Contract with
// `schemaFromContract`, decodes the same bytes with `decodeArgs`, and holds
// the TypeScript decoder to every verdict: accept against accept with equal
// values, reject against reject. Values compare under one defined mapping
// (`domainJson` here, `domain` in the Rust test): integers of every width as
// decimal strings, floats as their shortest decimal, blobs as `{ blob: hex }`,
// everything else as the domain shape itself. Rejections compare as verdicts
// only; the reference's error text is recorded for readers, never compared.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { decodeArgs } from "../codec.ts";
import { schemaFromContract } from "../contract.ts";
import type { AnySchema } from "../schema.ts";

interface CoercionCase {
  readonly name: string;
  readonly expected: readonly string[];
  readonly hex: string;
  readonly verdict: "accept" | "reject";
  readonly values?: readonly unknown[];
}

interface CoercionGolden {
  readonly envelope: unknown;
  readonly cases: readonly CoercionCase[];
}

const golden = JSON.parse(
  readFileSync(new URL("../../tests/goldens/wire/coercion.json", import.meta.url), "utf8"),
) as CoercionGolden;

function fromHex(text: string): Uint8Array {
  const out = new Uint8Array(text.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = parseInt(text.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function toHex(bytes: Uint8Array): string {
  let text = "";
  for (const byte of bytes) {
    text += byte.toString(16).padStart(2, "0");
  }
  return text;
}

/** A decoded domain value under the mapping the Rust side emits. */
export function domainJson(value: unknown): unknown {
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return value;
  }
  if (typeof value === "bigint") {
    return value.toString();
  }
  if (typeof value === "number") {
    return Object.is(value, -0) ? "-0" : String(value);
  }
  if (value instanceof Uint8Array) {
    return { blob: toHex(value) };
  }
  if (Array.isArray(value)) {
    return value.map(domainJson);
  }
  if (typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value)) {
      out[key] = domainJson((value as Record<string, unknown>)[key]);
    }
    return out;
  }
  throw new Error(`no mapping for a ${typeof value}`);
}

/** The schemas the golden's Contract describes, by declaration name. */
export function coercionSchemas(): { readonly [name: string]: AnySchema } {
  const built = schemaFromContract(golden.envelope);
  if (!built.ok) {
    throw new Error(`the coercion Contract must load: ${JSON.stringify(built.issues)}`);
  }
  return built.schemas;
}

/**
 * One case's verdict from the TypeScript decoder, in the golden's terms:
 * `accept` with mapped values, or `reject`.
 */
export function tsVerdict(
  schemas: { readonly [name: string]: AnySchema },
  entry: CoercionCase,
): { readonly verdict: "accept" | "reject"; readonly values?: readonly unknown[] } {
  const expected = entry.expected.map((name) => {
    const schema = schemas[name];
    if (schema === undefined) {
      throw new Error(`${entry.name}: the Contract has no declaration ${name}`);
    }
    return schema;
  });
  const result = decodeArgs(expected, fromHex(entry.hex));
  return result.ok
    ? { verdict: "accept", values: result.values.map(domainJson) }
    : { verdict: "reject" };
}

test("the coercion golden carries at least 48 pinned triples", () => {
  assert(golden.cases.length >= 48, `${golden.cases.length} cases`);
  const verdicts = new Set(golden.cases.map((entry) => entry.verdict));
  assert.deepStrictEqual([...verdicts].sort(), ["accept", "reject"]);
});

test("decodeArgs agrees with the reference decoder on every coercion verdict", () => {
  const schemas = coercionSchemas();
  const disagreements: string[] = [];
  for (const entry of golden.cases) {
    const ours = tsVerdict(schemas, entry);
    const theirs =
      entry.verdict === "accept"
        ? { verdict: "accept", values: entry.values }
        : { verdict: "reject" };
    try {
      assert.deepStrictEqual(ours, theirs);
    } catch {
      disagreements.push(
        `${entry.name}: reference ${JSON.stringify(theirs)}, ours ${JSON.stringify(ours)}`,
      );
    }
  }
  assert.deepStrictEqual(disagreements, []);
});
