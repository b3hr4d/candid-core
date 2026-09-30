// Options are code, not input (issue #190): every public entry point that
// takes an options object refuses an unknown key or an invalid limit with a
// `TypeError` before it reads anything else, instead of silently applying a
// default (a misspelled key) or no bound at all (a `NaN` limit). `0` stays a
// defined fail-closed limit, as in the Rust `Limits` (ADR 0005). One table
// drives all seven entry points, so none can drift from the others.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { encode, encodeArgs, decode, decodeArgs } from "../codec.ts";
import { schemaFromContract, type FieldNameEntry } from "../contract.ts";
import { validate, unwrapResult } from "../validate.ts";
import { c, type Schema } from "../schema.ts";

const goldens = new URL("../../tests/goldens/", import.meta.url);
const ledgerContract = JSON.parse(
  readFileSync(new URL("ledger.contract.json", goldens), "utf8"),
) as unknown;
const ledgerNames = JSON.parse(
  readFileSync(new URL("ledger.names.json", goldens), "utf8"),
) as FieldNameEntry[];

const Result = c.variant({ ok: c.nat, err: c.text });
const natBytes = (() => {
  const encoded = encode(c.nat, 5n);
  assert(encoded.ok);
  return encoded.ok ? encoded.bytes : new Uint8Array(0);
})();

/** Anything with an `ok` discriminant: every entry point's result. */
type Outcome = { readonly ok: boolean };

interface Entry {
  readonly name: string;
  /** The entry point's limit keys, each a non-negative safe integer. */
  readonly limits: readonly string[];
  /** Call the entry point with a valid input and the given options. */
  readonly call: (options: unknown) => Outcome;
}

const entries: readonly Entry[] = [
  {
    name: "validate",
    limits: ["maxDepth", "maxElements", "maxIssues"],
    call: (options) => validate(c.nat, 5n, options as never),
  },
  {
    name: "unwrapResult",
    limits: ["maxDepth", "maxElements", "maxIssues"],
    call: (options) => unwrapResult(Result, { tag: "ok", value: 5n }, options as never),
  },
  {
    name: "encode",
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => encode(c.nat, 5n, options as never),
  },
  {
    name: "encodeArgs",
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => encodeArgs([c.nat], [5n], options as never),
  },
  {
    name: "decode",
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => decode(c.nat, natBytes, options as never),
  },
  {
    name: "decodeArgs",
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => decodeArgs([c.nat], natBytes, options as never),
  },
  {
    name: "schemaFromContract",
    limits: ["maxTypeNodes", "maxFields", "maxDeclarations"],
    // The options object itself gains `names`, so its other own keys —
    // non-enumerable and symbol ones included — reach the entry point as
    // written; a spread copy would drop some of them.
    call: (options) => {
      if (typeof options === "object" && options !== null && !Array.isArray(options)) {
        Object.defineProperty(options, "names", { value: ledgerNames, enumerable: true });
      }
      return schemaFromContract(ledgerContract, options as never);
    },
  },
];

/** Assert `run` throws a `TypeError` whose message names `entry` and `detail`. */
function throwsTypeError(run: () => unknown, entry: string, detail: string): void {
  let thrown: unknown;
  try {
    run();
  } catch (error) {
    thrown = error;
  }
  assert(thrown instanceof TypeError, `${entry} must throw a TypeError (${detail})`);
  const { message } = thrown as TypeError;
  assert(message.startsWith(`${entry}: `), `the message names ${entry}: ${message}`);
  assert(message.includes(detail), `the message mentions ${detail}: ${message}`);
}

test("every entry point accepts its own limits, absent or undefined", () => {
  for (const entry of entries) {
    assert(entry.call({}).ok, `${entry.name} with {}`);
    const undefinedValues = Object.fromEntries(entry.limits.map((key) => [key, undefined]));
    assert(entry.call(undefinedValues).ok, `${entry.name} with undefined limits`);
    const generous = Object.fromEntries(entry.limits.map((key) => [key, 1_000_000]));
    assert(entry.call(generous).ok, `${entry.name} with explicit limits`);
    const largest = Object.fromEntries(entry.limits.map((key) => [key, Number.MAX_SAFE_INTEGER]));
    assert(entry.call(largest).ok, `${entry.name} with MAX_SAFE_INTEGER limits`);
  }
});

test("an unknown option key throws TypeError from every entry point", () => {
  const misspelled: Record<string, string> = {
    validate: "maxDeph",
    unwrapResult: "maxIssue",
    encode: "maxDeph",
    encodeArgs: "maxElement",
    decode: "maxByte",
    decodeArgs: "max_depth",
    schemaFromContract: "maxTypeNodez",
  };
  for (const entry of entries) {
    const key = misspelled[entry.name];
    throwsTypeError(() => entry.call({ [key]: 0 }), entry.name, JSON.stringify(key));
    // A key another entry point knows is still unknown here: the codec has
    // no `maxIssues`, validate no `maxBytes`.
    const foreign = entry.limits.includes("maxBytes") ? "maxIssues" : "maxBytes";
    throwsTypeError(() => entry.call({ [foreign]: 1 }), entry.name, JSON.stringify(foreign));
    // Symbol and non-enumerable own keys are keys too.
    throwsTypeError(() => entry.call({ [Symbol("x")]: 1 }), entry.name, "unknown option");
    const hidden = Object.defineProperty({}, "maxDepht", { value: 1, enumerable: false });
    throwsTypeError(() => entry.call(hidden), entry.name, '"maxDepht"');
  }
});

test("an invalid limit value throws TypeError from every entry point", () => {
  const invalid: readonly [unknown, string][] = [
    [Number.NaN, "NaN"],
    [-1, "-1"],
    [1.5, "1.5"],
    ["1", 'the string "1"'],
    [Number.POSITIVE_INFINITY, "Infinity"],
    [Number.MAX_SAFE_INTEGER + 1, String(Number.MAX_SAFE_INTEGER + 1)],
    [null, "null"],
    [1n, "a bigint"],
    [true, "a boolean"],
  ];
  for (const entry of entries) {
    for (const key of entry.limits) {
      for (const [value, shown] of invalid) {
        throwsTypeError(() => entry.call({ [key]: value }), entry.name, `${key} must be`);
        throwsTypeError(() => entry.call({ [key]: value }), entry.name, `got ${shown}`);
      }
    }
  }
});

test("an options argument that is not an object throws TypeError", () => {
  for (const entry of entries.filter((e) => e.name !== "schemaFromContract")) {
    for (const options of [null, [], 5, "maxDepth"]) {
      throwsTypeError(() => entry.call(options), entry.name, "options must be an object");
    }
  }
  throwsTypeError(
    () => schemaFromContract(ledgerContract, null as never),
    "schemaFromContract",
    "options must be an object",
  );
});

test("options are refused before the value is read", () => {
  // A getter that would record a read proves the order: the TypeError comes
  // first, and the value is never touched.
  let reads = 0;
  const value = {
    get a(): bigint {
      reads += 1;
      return 1n;
    },
  };
  const schema = c.record({ a: c.nat });
  assert.throws(() => validate(schema, value, { maxDepth: Number.NaN }), TypeError);
  assert.throws(() => encode(schema, value, { maxDepht: 1 } as never), TypeError);
  assert.strictEqual(reads, 0);
});

test("a NaN depth limit no longer switches the bound off", () => {
  // The issue #190 repro: under `maxDepth: NaN` every `depth > NaN` was
  // false, so validate accepted a value the default refuses.
  let schema: Schema<unknown> = c.nat as Schema<unknown>;
  let value: unknown = 1n;
  for (let level = 0; level < 300; level += 1) {
    schema = c.vec(schema) as Schema<unknown>;
    value = [value];
  }
  const refused = validate(schema, value);
  assert(!refused.ok);
  if (!refused.ok) {
    assert.deepStrictEqual(refused.issues[0].resource_limit, {
      resource: "value_depth",
      limit: 256,
      observed: 257,
    });
  }
  throwsTypeError(() => validate(schema, value, { maxDepth: Number.NaN }), "validate", "NaN");
  throwsTypeError(
    () => decode(c.nat, natBytes, { maxBytes: Number.NaN }),
    "decode",
    "maxBytes must be",
  );
});

test("zero is a valid limit and fails closed", () => {
  const cases: readonly [string, () => Outcome, string][] = [
    [
      "validate maxDepth",
      () => validate(c.record({ a: c.nat }), { a: 1n }, { maxDepth: 0 }),
      "value_depth",
    ],
    ["validate maxElements", () => validate(c.nat, 1n, { maxElements: 0 }), "value_elements"],
    [
      "encode maxDepth",
      () => encode(c.record({ a: c.nat }), { a: 1n }, { maxDepth: 0 }),
      "value_depth",
    ],
    ["encode maxElements", () => encode(c.nat, 1n, { maxElements: 0 }), "value_elements"],
    [
      "encode maxTypeTableEntries",
      () => encode(c.record({ a: c.nat }), { a: 1n }, { maxTypeTableEntries: 0 }),
      "type_table_entries",
    ],
    ["encode maxNumericBytes", () => encode(c.nat, 1n, { maxNumericBytes: 0 }), "numeric_bytes"],
    ["decode maxBytes", () => decode(c.nat, natBytes, { maxBytes: 0 }), "bytes"],
    ["decode maxElements", () => decode(c.nat, natBytes, { maxElements: 0 }), "value_elements"],
    [
      "schemaFromContract maxTypeNodes",
      () => schemaFromContract(ledgerContract, { names: ledgerNames, maxTypeNodes: 0 }),
      "type_nodes",
    ],
    [
      "schemaFromContract maxFields",
      () => schemaFromContract(ledgerContract, { names: ledgerNames, maxFields: 0 }),
      "fields",
    ],
    [
      "schemaFromContract maxDeclarations",
      () => schemaFromContract(ledgerContract, { names: ledgerNames, maxDeclarations: 0 }),
      "declarations",
    ],
  ];
  for (const [name, run, resource] of cases) {
    const outcome = run() as Outcome & {
      readonly issues?: readonly { readonly resource_limit?: { readonly resource: string } }[];
    };
    assert(!outcome.ok, `${name}: 0 must refuse`);
    assert.strictEqual(outcome.issues?.[0].resource_limit?.resource, resource, name);
  }
  // `maxIssues: 0` still reports one issue: a failure is never empty.
  const capped = validate(c.record({ a: c.nat, b: c.nat }), {}, { maxIssues: 0 });
  assert(!capped.ok);
  if (!capped.ok) {
    assert.strictEqual(capped.issues.length, 1);
  }
});
