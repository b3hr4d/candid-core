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
  /** `call`, but passing `options` through untouched (no key added to it). */
  readonly bare?: (options: unknown) => Outcome;
  /** A limit whose `0` refuses the valid input, and the resource it reports. */
  readonly refusing: readonly [key: string, resource: string];
}

const entries: readonly Entry[] = [
  {
    name: "validate",
    refusing: ["maxElements", "value_elements"],
    limits: ["maxDepth", "maxElements", "maxIssues"],
    call: (options) => validate(c.nat, 5n, options as never),
  },
  {
    name: "unwrapResult",
    refusing: ["maxElements", "value_elements"],
    limits: ["maxDepth", "maxElements", "maxIssues"],
    call: (options) => unwrapResult(Result, { tag: "ok", value: 5n }, options as never),
  },
  {
    name: "encode",
    refusing: ["maxElements", "value_elements"],
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => encode(c.nat, 5n, options as never),
  },
  {
    name: "encodeArgs",
    refusing: ["maxElements", "value_elements"],
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => encodeArgs([c.nat], [5n], options as never),
  },
  {
    name: "decode",
    refusing: ["maxElements", "value_elements"],
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => decode(c.nat, natBytes, options as never),
  },
  {
    name: "decodeArgs",
    refusing: ["maxElements", "value_elements"],
    limits: ["maxBytes", "maxTypeTableEntries", "maxDepth", "maxElements", "maxNumericBytes"],
    call: (options) => decodeArgs([c.nat], natBytes, options as never),
  },
  {
    name: "schemaFromContract",
    refusing: ["maxTypeNodes", "type_nodes"],
    limits: ["maxTypeNodes", "maxFields", "maxDeclarations"],
    // No name table: field keys render `_id_`, and the options object reaches
    // the loader exactly as the test built it.
    bare: (options) => schemaFromContract(ledgerContract, options as never),
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

// ---------------------------------------------------------------------------
// One read per option (the review finding on this PR)
// ---------------------------------------------------------------------------
//
// The check and the walk used to read the caller's object separately, so a
// getter or Proxy answering `0` to the check and something else (`NaN`, a
// huge number) to the walk ran with a bound nobody checked. Now each known
// key is read exactly once, into a frozen snapshot that every entry point —
// and everything it calls — reads instead. Each test below uses the entry's
// `refusing` limit: `0` refuses the input, so the call must refuse even
// though every later read would have said "no bound".

/** The outcome's first resource, for a refused call. */
function resourceOf(outcome: Outcome): string | undefined {
  const issues = (
    outcome as {
      readonly issues?: readonly { readonly resource_limit?: { readonly resource: string } }[];
    }
  ).issues;
  return issues?.[0]?.resource_limit?.resource;
}

function runOf(entry: Entry): (options: unknown) => Outcome {
  return entry.bare ?? entry.call;
}

test("a getter that changes between reads is read once, and its first answer is the policy", () => {
  for (const entry of entries) {
    const [key, resource] = entry.refusing;
    for (const later of [Number.NaN, 1_000_000]) {
      let reads = 0;
      const options = Object.defineProperty({}, key, {
        enumerable: true,
        get(): number {
          reads += 1;
          return reads === 1 ? 0 : later;
        },
      });
      const outcome = runOf(entry)(options);
      assert.strictEqual(reads, 1, `${entry.name}: ${key} read once`);
      assert(!outcome.ok, `${entry.name}: the checked 0 is the bound`);
      assert.strictEqual(resourceOf(outcome), resource, entry.name);
    }
  }
});

test("a Proxy that changes between reads is read once per key", () => {
  for (const entry of entries) {
    const [key, resource] = entry.refusing;
    const gets = new Map<string | symbol, number>();
    let ownKeysCalls = 0;
    const options = new Proxy(
      {},
      {
        ownKeys(): (string | symbol)[] {
          ownKeysCalls += 1;
          return ownKeysCalls === 1 ? [key] : [key, "maxDeph"];
        },
        getOwnPropertyDescriptor(): PropertyDescriptor {
          return { value: undefined, enumerable: true, configurable: true, writable: true };
        },
        get(_target, property): unknown {
          const count = (gets.get(property) ?? 0) + 1;
          gets.set(property, count);
          return property === key ? (count === 1 ? 0 : Number.NaN) : undefined;
        },
      },
    );
    const outcome = runOf(entry)(options);
    assert.strictEqual(ownKeysCalls, 1, `${entry.name}: keys listed once`);
    for (const [property, count] of gets) {
      assert.strictEqual(count, 1, `${entry.name}: ${String(property)} read once`);
    }
    assert.strictEqual(gets.get(key), 1, `${entry.name}: ${key} read`);
    assert(!outcome.ok, `${entry.name}: the checked 0 is the bound`);
    assert.strictEqual(resourceOf(outcome), resource, entry.name);
  }
});

test("an options object that throws while being read is a TypeError with the cause", () => {
  const boom = new Error("boom");
  const throwing = (): never => {
    throw boom;
  };
  const revocable = Proxy.revocable({}, {});
  revocable.revoke();
  const hostile: readonly [string, (key: string) => unknown, boolean][] = [
    ["a throwing getter", (key) => Object.defineProperty({}, key, { get: throwing }), true],
    ["a Proxy whose get throws", () => new Proxy({}, { get: throwing }), true],
    ["a Proxy whose ownKeys throws", () => new Proxy({}, { ownKeys: throwing }), true],
    // A revoked Proxy fails the array check itself, with the engine's error.
    ["a revoked Proxy", () => revocable.proxy, false],
  ];
  for (const entry of entries) {
    for (const [name, make, ownCause] of hostile) {
      let thrown: unknown;
      try {
        runOf(entry)(make(entry.refusing[0]));
      } catch (error) {
        thrown = error;
      }
      assert(thrown instanceof TypeError, `${entry.name}, ${name}: a TypeError`);
      const error = thrown as TypeError & { readonly cause?: unknown };
      assert.strictEqual(
        error.message,
        `${entry.name}: the options object threw while being read`,
        `${entry.name}, ${name}`,
      );
      if (ownCause) {
        assert.strictEqual(error.cause, boom, `${entry.name}, ${name}: the cause is kept`);
      } else {
        assert(error.cause instanceof TypeError, `${entry.name}, ${name}: the engine's error`);
      }
    }
  }
});

test("unwrapResult hands validate the snapshot, not the caller's object", () => {
  // The nested call is where a second read used to hide: unwrapResult checked
  // the options, then validate's walk read them again.
  let reads = 0;
  const options = Object.defineProperty({}, "maxDepth", {
    enumerable: true,
    get(): number {
      reads += 1;
      return reads === 1 ? 0 : Number.NaN;
    },
  });
  const outcome = unwrapResult(
    c.variant({ ok: c.vec(c.nat), err: c.text }),
    {
      tag: "ok",
      value: [1n],
    },
    options,
  );
  assert.strictEqual(reads, 1);
  assert(!outcome.ok);
  const issues = (outcome as { readonly issues?: readonly { readonly resource_limit?: unknown }[] })
    .issues;
  assert.deepStrictEqual(issues?.[0]?.resource_limit, {
    resource: "value_depth",
    limit: 0,
    observed: 1,
  });
});
