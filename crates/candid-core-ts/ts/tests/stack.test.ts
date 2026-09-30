// Host stack exhaustion is a resource failure, not a value or schema problem.
//
// `validate`, `encode` and `decode` are recursive walkers. At the default
// `maxDepth` of 256 every one refuses with `value_depth` long before the
// engine's stack runs out, so overflow needs a raised `maxDepth` (or a
// hand-built schema deeper than any stack). When it does happen the engine
// throws, and the walkers' catch-all choke points used to label that
// `unreadable_value` (validate, encode) or `unsupported_schema` (decode) —
// claims about the value or the schema that are simply false. It is now
// `resource_limit_exceeded` with the new resource member `stack`.
//
// Nothing here depends on where a particular engine's stack limit falls: the
// inputs are hundreds of thousands of levels deep, orders of magnitude past
// any realistic stack, and the assertions are on the issue's shape — never on
// the depth an engine happened to reach.

import { test } from "node:test";
import assert from "node:assert/strict";

import { decode, encode, type CodecIssue, type CodecResourceLimitInfo } from "../codec.ts";
import { validate, type ResourceLimitInfo, type ValidationIssue } from "../validate.ts";
import { c, type AnySchema, type Schema } from "../schema.ts";

// Compile-time pins. Each is a `true` assertion the type checker must accept:
// the resource unions are closed and literal, so a consumer can switch on them
// exhaustively, and gaining a member is a type-level break these lines record.
type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
export const validateResourceUnion: Equal<
  ResourceLimitInfo["resource"],
  "value_depth" | "value_elements" | "stack"
> = true;
export const codecResourceUnion: Equal<
  CodecResourceLimitInfo["resource"],
  "bytes" | "type_table_entries" | "value_depth" | "value_elements" | "numeric_bytes" | "stack"
> = true;

// An exhaustive switch over the codec union: adding a member without handling
// it here is a compile error, which is what a consumer's own switch would see.
export function describeCodecResource(resource: CodecResourceLimitInfo["resource"]): string {
  switch (resource) {
    case "bytes":
    case "type_table_entries":
    case "value_depth":
    case "value_elements":
    case "numeric_bytes":
    case "stack":
      return resource;
    default: {
      const unreachable: never = resource;
      return unreachable;
    }
  }
}

type Nest = Nest[];
const Nested: Schema<Nest> = c.rec(() => c.vec(Nested));

// Far deeper than any realistic stack: each level costs two depth units and
// several engine frames, and the engines here hold thousands, not hundreds of
// thousands, of frames.
const LEVELS = 200_000;
const RAISED = { maxDepth: 1e9, maxElements: 1e9 };

function nestedValue(levels: number): Nest {
  let value: Nest = [];
  for (let level = 0; level < levels; level += 1) {
    value = [value];
  }
  return value;
}

/** `vec vec … vec` wire bytes: one type-table entry, `levels` single-element vecs. */
function nestedBytes(levels: number): Uint8Array {
  const head = [0x44, 0x49, 0x44, 0x4c, 0x01, 0x6d, 0x00, 0x01, 0x00];
  const bytes = new Uint8Array(head.length + levels + 1);
  bytes.set(head);
  bytes.fill(0x01, head.length, head.length + levels);
  bytes[head.length + levels] = 0x00;
  return bytes;
}

type AnyIssue = ValidationIssue | CodecIssue;

function only(result: { readonly ok: boolean; readonly issues?: readonly AnyIssue[] }): AnyIssue {
  assert.strictEqual(result.ok, false);
  assert.strictEqual(result.issues?.length, 1, "exactly one issue");
  return (result.issues as readonly AnyIssue[])[0];
}

function assertStack(issue: AnyIssue, limit: number): void {
  assert.strictEqual(issue.code, "resource_limit_exceeded");
  const info = issue.resource_limit;
  if (info === undefined) {
    throw new Error("a resource_limit_exceeded issue must carry its triple");
  }
  assert.strictEqual(info.resource, "stack");
  assert.strictEqual(info.limit, limit);
  // The walk got past the default depth before the engine refused. The exact
  // figure is engine-dependent and is deliberately not asserted.
  assert(Number.isInteger(info.observed));
  assert(info.observed > 256, `observed ${info.observed} should exceed the default depth`);
  assert(issue.path.startsWith("$"));
  // The message must not claim more than the triple can back up: the stack
  // ran out at a depth, and that depth is not always below the configured
  // maxDepth (see the type-table test below).
  assert(!issue.message.includes("before"), issue.message);
}

test("stack exhaustion is resource_limit_exceeded {stack} from validate, encode and decode", () => {
  assertStack(only(validate(Nested, nestedValue(LEVELS), RAISED)), 1e9);
  assertStack(only(encode(Nested, nestedValue(LEVELS), RAISED)), 1e9);
  assertStack(only(decode(Nested, nestedBytes(LEVELS), RAISED)), 1e9);
  // With `maxDepth` raised far past any stack, the depth reached is below it.
  for (const result of [
    validate(Nested, nestedValue(LEVELS), RAISED),
    encode(Nested, nestedValue(LEVELS), RAISED),
    decode(Nested, nestedBytes(LEVELS), RAISED),
  ]) {
    assert((only(result).resource_limit?.observed ?? Infinity) < 1e9);
  }
});

test("the stack limit reports the configured maxDepth, whatever it is", () => {
  const options = { maxDepth: 5_000_000, maxElements: 1e9 };
  assertStack(only(validate(Nested, nestedValue(LEVELS), options)), 5_000_000);
  assertStack(only(encode(Nested, nestedValue(LEVELS), options)), 5_000_000);
  assertStack(only(decode(Nested, nestedBytes(LEVELS), options)), 5_000_000);
});

function deepVec(levels: number): AnySchema {
  let deep: AnySchema = c.nat8;
  for (let level = 0; level < levels; level += 1) {
    deep = c.vec(deep);
  }
  return deep;
}

test("a hand-built schema deeper than the stack overflows the encoder's type table", () => {
  // No `rec` anywhere: depth here is plain combinator nesting, which the type
  // table construction recurses through without any depth charge of its own.
  assertStack(only(encode(deepVec(LEVELS), [], RAISED)), 1e9);
});

test("the same schema at default limits is stack, with observed above the limit", () => {
  // The type table charges no depth for plain nested combinators, so the
  // default maxDepth of 256 never fires here and the engine's stack runs out
  // first. The issue must say so without claiming the stack ran out *before*
  // maxDepth: the depth reached is far past it. Only the ordering is pinned,
  // never the figure.
  const issue = only(encode(deepVec(LEVELS), []));
  assertStack(issue, 256);
  const observed = issue.resource_limit?.observed ?? 0;
  assert(observed > 256, `observed ${observed} is past the default limit of 256`);
});

test("at default limits the same inputs still fail with value_depth at 256", () => {
  const expected = { resource: "value_depth", limit: 256, observed: 257 };
  for (const result of [
    validate(Nested, nestedValue(LEVELS)),
    encode(Nested, nestedValue(LEVELS)),
    decode(Nested, nestedBytes(LEVELS)),
  ]) {
    const issue = only(result);
    assert.strictEqual(issue.code, "resource_limit_exceeded");
    assert.deepStrictEqual(issue.resource_limit, expected);
  }
});

// What a walker was doing when something threw, one row per entry point. Each
// installs `thrown` where user-supplied code runs: a value's getter (validate,
// encode) or a rec thunk (decode, where only the schema is user code).
type Attempt = { readonly name: string; readonly run: (thrown: () => never) => AnyIssue };

const attempts: readonly Attempt[] = [
  {
    name: "validate (value getter)",
    run: (thrown) =>
      only(
        validate(c.record({ a: c.nat8 }), {
          get a(): number {
            return thrown();
          },
        }),
      ),
  },
  {
    name: "encode (value getter)",
    run: (thrown) =>
      only(
        encode(c.record({ a: c.nat8 }), {
          get a(): number {
            return thrown();
          },
        }),
      ),
  },
  {
    name: "decode (rec thunk)",
    run: (thrown) => {
      const encoded = encode(c.null, null);
      if (!encoded.ok) {
        throw new Error("null must encode");
      }
      return only(
        decode(
          c.rec(() => thrown()),
          encoded.bytes,
        ),
      );
    },
  },
];

const notStack: ReadonlyArray<readonly [label: string, make: () => unknown]> = [
  ["a plain Error", () => new Error("x")],
  // Genuine engine-thrown RangeErrors that are not stack exhaustion.
  [
    "an invalid array length",
    () => {
      try {
        return new Array(-1);
      } catch (error) {
        return error;
      }
    },
  ],
  [
    "an invalid typed-array length",
    () => {
      try {
        return new Uint8Array(-1);
      } catch (error) {
        return error;
      }
    },
  ],
  [
    "a BigInt that is too big",
    () => {
      try {
        return 1n << (2n ** 40n);
      } catch (error) {
        return error;
      }
    },
  ],
  [
    "toFixed out of range",
    () => {
      try {
        return (1).toFixed(101);
      } catch (error) {
        return error;
      }
    },
  ],
  ["a hand-made RangeError", () => new RangeError("bad")],
  // The right words on the wrong class: only a RangeError/InternalError counts.
  ["a plain Error quoting the V8 words", () => new Error("Maximum call stack size exceeded")],
  ["a TypeError quoting the Firefox words", () => new TypeError("too much recursion")],
  // Each engine's words belong to its own class; crossing them is not a match.
  ["a RangeError with the Firefox words", () => new RangeError("too much recursion")],
  ["a bare string quoting the V8 words", () => "Maximum call stack size exceeded"],
  ["null", () => null],
  ["undefined", () => undefined],
  [
    "a thrown object whose name read throws",
    () =>
      new Proxy(
        {},
        {
          get() {
            throw new Error("hostile read");
          },
        },
      ),
  ],
];

for (const attempt of attempts) {
  test(`${attempt.name}: errors that are not stack exhaustion keep their label`, () => {
    const expected = attempt.name.startsWith("decode") ? "unsupported_schema" : "unreadable_value";
    for (const [label, make] of notStack) {
      const issue = attempt.run(() => {
        // eslint-disable-next-line @typescript-eslint/only-throw-error
        throw make();
      });
      assert.strictEqual(issue.code, expected, `${label} must be ${expected}`);
      assert.strictEqual(issue.resource_limit, undefined, `${label} must carry no resource`);
    }
  });
}

// What each engine says, verbatim. These are forged by the test, so they hold
// on every host — the V8 words are what Node produces for real, checked below.
const stackShaped: ReadonlyArray<readonly [label: string, make: () => unknown]> = [
  ["V8", () => new RangeError("Maximum call stack size exceeded")],
  ["JavaScriptCore", () => new RangeError("Maximum call stack size exceeded.")],
  ["SpiderMonkey", () => Object.assign(new Error("too much recursion"), { name: "InternalError" })],
];

for (const attempt of attempts) {
  test(`${attempt.name}: each engine's stack-overflow error is reported as stack`, () => {
    for (const [engine, make] of stackShaped) {
      const issue = attempt.run(() => {
        // eslint-disable-next-line @typescript-eslint/only-throw-error
        throw make();
      });
      assert.strictEqual(issue.code, "resource_limit_exceeded", engine);
      assert.strictEqual(issue.resource_limit?.resource, "stack", engine);
      assert.strictEqual(issue.resource_limit?.limit, 256, engine);
      // The walk was a step or two in, nowhere near the configured depth.
      const observed = issue.resource_limit?.observed ?? -1;
      assert(Number.isInteger(observed) && observed >= 0 && observed < 256, engine);
    }
  });
}

test("a getter or thunk that itself overflows the stack is reported as stack", () => {
  // Residual user-code overflow, which stays a stack failure however the
  // walkers are implemented: this recursion is the caller's, and the host
  // really did run out. No walker depth is involved, so `observed` is small.
  // Not a tail call: JavaScriptCore implements proper tail calls, so a bare
  // `return runaway()` would loop forever there instead of overflowing.
  const runaway = (): never => {
    runaway();
    throw new Error("unreachable");
  };
  for (const attempt of attempts) {
    const issue = attempt.run(runaway);
    assert.strictEqual(issue.code, "resource_limit_exceeded", attempt.name);
    assert.strictEqual(issue.resource_limit?.resource, "stack", attempt.name);
  }
});

// Detection lives in two private copies, one in `validate.ts` and one in
// `codec.ts`, and neither is exported. So it cannot be unit-tested directly and
// it cannot be allowed to drift: this table is driven through all three entry
// points, and each must classify every row identically.
test("validate, encode and decode agree on what is stack exhaustion", () => {
  let real: unknown;
  const recurse = (): number => recurse() + 1;
  try {
    recurse();
  } catch (error) {
    real = error;
  }
  const spiderMonkey = (): unknown =>
    Object.assign(new Error("too much recursion"), { name: "InternalError" });

  const shared: ReadonlyArray<readonly [label: string, make: () => unknown, stack: boolean]> = [
    ...notStack.map(([label, make]) => [label, make, false] as const),
    ...stackShaped.map(([label, make]) => [`${label}-shaped`, make, true] as const),
    ["the real overflow this host's engine threw", () => real, true],
    // Cross-realm shape: name and message on a plain object, no shared prototype.
    [
      "a cross-realm-shaped plain object",
      () => ({ name: "RangeError", message: "Maximum call stack size exceeded" }),
      true,
    ],
    [
      "the SpiderMonkey words only inside a longer message",
      () =>
        Object.assign(spiderMonkey() as object, { message: "InternalError: too much recursion" }),
      false,
    ],
    ["a name with no message", () => ({ name: "RangeError" }), false],
    ["a message with no name", () => ({ message: "Maximum call stack size exceeded" }), false],
  ];

  for (const [label, make, stack] of shared) {
    const verdicts = attempts.map((attempt) => {
      const issue = attempt.run(() => {
        // eslint-disable-next-line @typescript-eslint/only-throw-error
        throw make();
      });
      return `${attempt.name}: ${issue.resource_limit?.resource === "stack"}`;
    });
    const expected = attempts.map((attempt) => `${attempt.name}: ${stack}`);
    assert.deepStrictEqual(verdicts, expected, label);
  }
});
