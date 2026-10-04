// A hostile *thrown value* cannot break the no-throw guarantee (issue #199).
//
// `validate`, `encode` and `decode` turn anything user code throws — a
// getter, a Proxy trap, a rec thunk — into an issue at one choke point per
// entry point. The codec's choke points must also tell the module's own
// control-flow exceptions apart from everything else, and they used to ask
// with `instanceof`, which reads the thrown value's prototype: for a Proxy
// that is a user-controlled `getPrototypeOf` trap, and a trap that throws
// raised its exception from inside the catch block, out of `encode` and
// `decode`. The rows below throw such values from every kind of user code
// the walkers call, on the value side, on the schema side and inside an
// `opt`, and require exactly the issue an ordinary thrown `Error` gets.

import { test } from "node:test";
import assert from "node:assert/strict";

import { decode, encode, type CodecIssue } from "../codec.ts";
import { validate, type ValidationIssue } from "../validate.ts";
import { c } from "../schema.ts";

type AnyIssue = ValidationIssue | CodecIssue;
type Result = { readonly ok: boolean; readonly issues?: readonly AnyIssue[] };

function bytesOf(result: ReturnType<typeof encode>): Uint8Array {
  if (!result.ok) {
    throw new Error("fixture must encode");
  }
  return result.bytes;
}

// Wire fixtures for the decode rows: `null`, and `opt record { a : nat8 }`
// holding a value, so decoding reaches the record's field inside the opt.
const nullBytes = bytesOf(encode(c.null, null));
const optRecordBytes = bytesOf(encode(c.opt(c.record({ a: c.nat8 })), { a: 1 }));

/** An object whose `a` getter runs `thrown`. */
function getter(thrown: () => never): { readonly a: number } {
  return {
    get a(): number {
      return thrown();
    },
  };
}

// Where user code runs, one row per entry point and position. `code` and
// `path` are what an ordinary thrown `Error` gets there; every row must give a
// hostile thrown value exactly the same issue.
type Row = {
  readonly name: string;
  readonly code: "unreadable_value" | "unsupported_schema";
  readonly path: string;
  readonly run: (thrown: () => never) => Result;
};

const rows: readonly Row[] = [
  // The value side: a getter.
  {
    name: "validate, value getter",
    code: "unreadable_value",
    path: "$.a",
    run: (thrown) => validate(c.record({ a: c.nat8 }), getter(thrown)),
  },
  {
    name: "encode, value getter",
    code: "unreadable_value",
    path: "$.a",
    run: (thrown) => encode(c.record({ a: c.nat8 }), getter(thrown)),
  },
  {
    name: "validate, value getter inside an opt",
    code: "unreadable_value",
    path: "$.a",
    run: (thrown) => validate(c.opt(c.record({ a: c.nat8 })), getter(thrown)),
  },
  {
    name: "encode, value getter inside an opt",
    code: "unreadable_value",
    path: "$.a",
    run: (thrown) => encode(c.opt(c.record({ a: c.nat8 })), getter(thrown)),
  },
  // The schema side: a rec thunk.
  {
    name: "validate, rec thunk",
    code: "unreadable_value",
    path: "$",
    run: (thrown) => validate(c.rec(thrown), null),
  },
  {
    name: "encode, rec thunk",
    code: "unreadable_value",
    path: "$",
    run: (thrown) => encode(c.rec(thrown), null),
  },
  {
    name: "decode, rec thunk",
    code: "unsupported_schema",
    path: "$",
    run: (thrown) => decode(c.rec(thrown), nullBytes),
  },
  {
    name: "validate, rec thunk inside an opt",
    code: "unreadable_value",
    path: "$",
    run: (thrown) => validate(c.opt(c.rec(thrown)), 1),
  },
  {
    name: "encode, rec thunk inside an opt",
    code: "unreadable_value",
    path: "$",
    run: (thrown) => encode(c.opt(c.rec(thrown)), null),
  },
  {
    name: "decode, rec thunk inside an opt",
    code: "unsupported_schema",
    path: "$",
    run: (thrown) => decode(c.opt(c.rec(thrown)), optRecordBytes),
  },
  // Thrown while an expected `opt` is decoding its constituent, so the
  // exception crosses the decoder's absorption point on its way out.
  {
    name: "decode, rec thunk below an opt that is decoding",
    code: "unsupported_schema",
    path: "$.a",
    run: (thrown) => decode(c.opt(c.record({ a: c.rec(thrown) })), optRecordBytes),
  },
];

// Thrown values that make `instanceof` throw. The last one's trap throws
// exactly what an engine throws when its stack runs out: the classification
// is of the value that was thrown, so the trap's own exception must neither
// escape nor be what gets reported.
const hostiles: ReadonlyArray<readonly [label: string, make: () => object]> = [
  [
    "a Proxy whose getPrototypeOf trap throws",
    () =>
      new Proxy(
        {},
        {
          getPrototypeOf(): never {
            throw new Error("trap");
          },
        },
      ),
  ],
  [
    "a revoked Proxy",
    () => {
      const { proxy, revoke } = Proxy.revocable({}, {});
      revoke();
      return proxy;
    },
  ],
  [
    "a Proxy whose getPrototypeOf trap throws a stack-shaped RangeError",
    () =>
      new Proxy(
        {},
        {
          getPrototypeOf(): never {
            throw new RangeError("Maximum call stack size exceeded");
          },
        },
      ),
  ],
];

/** Run `row` with `make()` thrown, failing the test if anything escapes. */
function attempt(row: Row, make: () => unknown, label: string): Result {
  try {
    return row.run(() => {
      // eslint-disable-next-line @typescript-eslint/only-throw-error
      throw make();
    });
  } catch (error) {
    throw new Error(`${row.name}: ${label} escaped as ${String(error)}`);
  }
}

test("each hostile fixture really makes instanceof throw", () => {
  for (const [label, make] of hostiles) {
    assert.throws(() => make() instanceof Error, label);
  }
});

test("an ordinary thrown Error gets each row's pinned issue", () => {
  for (const row of rows) {
    const result = attempt(row, () => new Error("ordinary"), "an ordinary Error");
    assert.deepStrictEqual(
      result,
      {
        ok: false,
        issues: [
          {
            code: row.code,
            path: row.path,
            message:
              row.code === "unreadable_value"
                ? "the value threw while being inspected"
                : "the schema threw while being traversed",
          },
        ],
      },
      row.name,
    );
  }
});

test("a thrown value that makes instanceof throw gets exactly an ordinary Error's issue", () => {
  for (const row of rows) {
    const ordinary = attempt(row, () => new Error("ordinary"), "an ordinary Error");
    for (const [label, make] of hostiles) {
      assert.deepStrictEqual(attempt(row, make, label), ordinary, `${row.name}: ${label}`);
    }
  }
});

test("a stack-shaped thrown value is stack exhaustion even when instanceof would throw", () => {
  // The stack classification reads only `name` and `message`, ahead of and
  // independently of the "is this ours?" check: a value that carries an
  // engine's stack-overflow words is `stack` whatever its prototype trap does.
  const make = (): object =>
    new Proxy(
      { name: "RangeError", message: "Maximum call stack size exceeded" },
      {
        getPrototypeOf(): never {
          throw new Error("trap");
        },
      },
    );
  for (const row of rows) {
    const result = attempt(row, make, "a stack-shaped hostile Proxy");
    assert.strictEqual(result.ok, false, row.name);
    assert.strictEqual(result.issues?.length, 1, row.name);
    const issue = (result.issues as readonly AnyIssue[])[0];
    assert.strictEqual(issue.code, "resource_limit_exceeded", row.name);
    assert.strictEqual(issue.resource_limit?.resource, "stack", row.name);
    assert.strictEqual(issue.path, row.path, row.name);
  }
});
