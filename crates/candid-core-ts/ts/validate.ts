// Structural validation of JavaScript values against the schema core — the
// runtime half of issue #102, walking exactly the combinators `schema.ts`
// defines and the generated builders construct.
//
// # The error shape
//
// Every failure is a `ValidationIssue`: `{ code, path, message }` plus a
// `resource_limit` triple when a bound was hit — deliberately the serialized
// shape of candid-core's own `Diagnostic` (`{code, path, message,
// resource_limit?}`), so a consumer that already understands candid-core
// violations understands these. Codes are stable snake_case strings from the
// closed `ValidationCode` union; paths are `$`-rooted, with `.name` for
// ASCII-identifier-shaped keys and `["…"]`/`[3]` otherwise, matching the
// `$.declarations[3].name` style candid-core emits.
//
// # Bounded, fail-closed, no exceptions for control flow
//
// Validation never throws on any input value (a malformed *options* object
// is a programmer error and throws `TypeError`; see `validate`): malformed
// values produce issues, and hostile ones — cyclic objects, huge arrays, adversarially deep
// nesting — hit explicit limits that mirror candid-core's `Limits` defaults
// (`maxDepth` 256 like `max_value_depth`, `maxElements` 1_000_000). A limit
// failure is itself an issue (`resource_limit_exceeded`, resource
// `value_depth` or `value_elements`) and terminates the walk, so a truncated
// validation can never report `ok`. Depth counts schema traversal steps —
// `rec` unwrapping included, which is what makes a mis-built self-referential
// `rec` chain terminate — so linked-list-shaped data consumes depth per
// element, exactly as it does in candid-core's value domain. The walk keeps
// its work on an explicit stack rather than the host's (issue #192), so
// `maxDepth` is the only depth bound: raised, it admits a 100,000-level
// linked list, and the answer never depends on the engine or its JIT state.
// What can still exhaust the host stack is user code the walk calls — a
// getter, a Proxy trap, a rec thunk that itself recurses too deeply — and
// that is reported as `resource_limit_exceeded` with resource `stack` (the
// host ran out, not the value or the schema) rather than as an unreadable
// value.
//
// The no-throw guarantee holds even against values that fight inspection —
// own accessors that throw, Proxies with hostile traps, revoked Proxies: the
// entire walk runs behind one fail-closed choke point that converts any
// exception a value raises into a terminal `unreadable_value` issue at the
// path being examined. `maxElements` charges every traversal step, examined
// record keys included; it bounds the work this walker performs, not the
// engine's own key-list materialization, which JavaScript enumeration
// (`Object.keys` and `for..in` alike) pays in one linear step at loop entry.
//
// # Strictness decisions (fail closed, recorded on issue #102)
//
// - `nat`/`int`/`nat64`/`int64` require `bigint`; a `number` there is
//   rejected, never coerced. Fixed-width integers must be integral and in
//   range; 255 passes `nat8`, 256 does not.
// - Records reject unknown keys and require every declared key, including
//   `opt` fields: the domain shape is `T | null` with the property present.
//   Presence means an **own enumerable** property — the projection
//   `JSON.stringify`, spread, and structured clone all see. A non-enumerable
//   property neither satisfies a required field nor counts as unknown, so a
//   validated value never serializes to something the schema rejects.
// - An `opt` whose inner type admits `null` (`opt opt`, `opt null`,
//   `opt reserved`) is boxed: a present value is exactly `{ some: v }`,
//   strict like a record, with issues inside it at `$.….some`.
// - Tag-only variant arms (`null` payload) reject a present `value` key; the
//   one domain shape is `{ tag }`, not `{ tag, value: null }`.
// - `float32` accepts any JavaScript number: f32 representability is a codec
//   concern, and rejecting `0.1` here would fail values the domain types
//   deliberately admit. NaN and infinities are valid Candid floats.
// - `principal` (and a service value, and a func reference's `principal`)
//   is a `Principal`: a string holding canonical principal text, checked
//   with `isPrincipal` (issue #187). Anything else — a non-canonical
//   spelling, an object with `toText()`, an SDK `Principal` instance — is
//   `invalid_type`, the code and path the encoder reports for the same
//   value, so the two refuse exactly the same principals. No Principal
//   implementation is a runtime dependency; the text form is this package's.
// - Issue order is deterministic: schema fields in their object's enumeration
//   order (canonical Contract order for every key the generator emits; note
//   JavaScript hoists integer-like keys, which no `_id_`-conventional or
//   identifier key is), then unexpected value keys in value enumeration
//   order.
//
// # Result unwrapping
//
// `isResultSchema` and `unwrapResult` at the foot of the file are the second
// surface here (issue #151): a validated read of the `variant { ok; err }`
// convention, directed by the schema rather than by probing a decoded value
// for `ok`/`err` keys. They live in this module because unwrapping *is*
// validation plus one typed read, and because the root entry imports only
// the small principal-text module at runtime while the Contract loader (and
// the internal form-model builder) import *it* — so a validator dependency
// there would have arrived with `schemaFromContract` for consumers who never
// asked for one.

import type {
  AnySchema,
  AnyFieldSchema,
  FuncSchema,
  ServiceSchema,
  BlobSchema,
  FieldSchemas,
  Infer,
  OptSchema,
  PrimitiveSchema,
  RecordSchema,
  RecSchema,
  Schema,
  TupleSchema,
  UnitSchema,
  VariantSchema,
  VecSchema,
} from "./schema.ts";
import { isPrincipal, resolveSchema } from "./schema.ts";
import { checkOptions } from "./options.ts";

// The boxed-option rule `isBoxedOpt` states, applied to an inner node this
// walk has already resolved under its own budget. Module-local on purpose:
// the rule's published surface is `isBoxedOpt`, and calling it here would
// re-resolve the inner outside the walk's accounting. The cross-walker test
// in `tests/schema-types.test.ts` pins this copy to `isBoxedOpt`.
function admitsNull(node: { readonly kind: string; readonly primitive?: unknown }): boolean {
  return (
    node.kind === "opt" ||
    (node.kind === "primitive" && (node.primitive === "null" || node.primitive === "reserved"))
  );
}

/** Stable machine-readable failure codes. Closed: additions are API changes. */
export type ValidationCode =
  | "invalid_type"
  | "not_integer"
  | "out_of_range"
  | "missing_field"
  | "unexpected_field"
  | "unknown_tag"
  | "invalid_length"
  | "uninhabited_type"
  | "unsupported_schema"
  | "unreadable_value"
  | "resource_limit_exceeded";

/**
 * The `{resource, limit, observed}` triple, as candid-core serializes it.
 *
 * `stack` is the one resource with no candid-core counterpart: the host
 * JavaScript stack ran out mid-walk. The walks keep their own work on explicit
 * stacks, so no depth of value or schema causes it; what does is
 * user code a walk calls — a getter, a Proxy trap, a rec thunk — recursing
 * too deeply itself. Its `limit` is the call's effective `maxDepth` and its
 * `observed` the deepest depth the walk had reached when the engine refused,
 * which says where the walk was, not where the host's stack ends. Read
 * `resource`, not a comparison of the two numbers, to tell `stack` from
 * `value_depth`.
 */
export interface ResourceLimitInfo {
  readonly resource: "value_depth" | "value_elements" | "stack";
  readonly limit: number;
  readonly observed: number;
}

/**
 * One failure. Deliberately the serialized shape of candid-core's own
 * diagnostic, so a consumer that already reads those reads these.
 */
export interface ValidationIssue {
  readonly code: ValidationCode;
  /** `$`-rooted path to the offending value, candid-core style. */
  readonly path: string;
  readonly message: string;
  readonly resource_limit?: ResourceLimitInfo;
}

/**
 * Bounds on one validation walk; each defaults to the `DEFAULT_MAX_*` below.
 * Each is a non-negative safe integer or absent (`undefined`); any other value,
 * and any other key, makes the call throw `TypeError`.
 */
export interface ValidateOptions {
  /**
   * Maximum schema traversal depth, mirroring `Limits::max_value_depth`.
   * Every step — combinator descent and `rec` unwrap alike — consumes one.
   */
  readonly maxDepth?: number;
  /** Total traversal budget across the whole value, all branches included. */
  readonly maxElements?: number;
  /** Stop collecting after this many issues; the result is still not-ok. */
  readonly maxIssues?: number;
}

/**
 * A result, never an exception: `ok` discriminates, and a failure always
 * carries at least one issue.
 */
export type ValidateResult =
  { readonly ok: true } | { readonly ok: false; readonly issues: readonly ValidationIssue[] };

/** Default `maxDepth`, mirroring candid-core's `max_value_depth`. */
export const DEFAULT_MAX_DEPTH = 256;
/** Default `maxElements`: the whole-value traversal budget. */
export const DEFAULT_MAX_ELEMENTS = 1_000_000;
/** Default `maxIssues`: how many failures one walk collects before stopping. */
export const DEFAULT_MAX_ISSUES = 100;

/** Every `ValidateOptions` key; anything else in an options object throws. */
const VALIDATE_LIMIT_KEYS: readonly (keyof ValidateOptions)[] = [
  "maxDepth",
  "maxElements",
  "maxIssues",
];

/**
 * Validate `value` against `schema`. Never throws on any `value`; a schema
 * object that is not one this core constructs fails closed with
 * `unsupported_schema`.
 *
 * Throws `TypeError`, before reading the value, on an options object with an
 * unknown key or a limit that is not a non-negative safe integer: a
 * misspelled limit would silently apply the default, and a `NaN` one would
 * switch its bound off. `0` is a valid, fail-closed limit.
 */
export function validate<T>(
  schema: Schema<T>,
  value: unknown,
  options: ValidateOptions = {},
): ValidateResult {
  return validateWith(checkOptions("validate", options, VALIDATE_LIMIT_KEYS), schema, value);
}

function validateWith(
  options: ValidateOptions,
  schema: AnyFieldSchema,
  value: unknown,
): ValidateResult {
  const walk = new Walk(options);
  // The fail-closed choke point behind the no-throw guarantee: a value can
  // fight inspection — an own accessor that throws, a Proxy trap, a revoked
  // Proxy that makes even `Array.isArray` throw — and every such read
  // happens inside this one call. The walk mutates a single shared path
  // array, so the catch still knows exactly which value was being examined.
  const path: PathSegment[] = [];
  try {
    walk.visit(schema, value, path, 0);
  } catch (error) {
    // The one place a host stack overflow is caught for the whole walk: it is
    // a property of the host, not of the value or the schema, so it must not
    // reach the `unreadable_value` label below. The walk itself is iterative
    // (issue #192), so what overflows is user code it called — a getter, a
    // Proxy trap or a rec thunk that itself recursed too deeply.
    if (isStackExhaustion(error)) {
      walk.stackExhausted(path);
    } else {
      walk.issues.push({
        code: "unreadable_value",
        path: renderPath(path),
        message: "the value threw while being inspected",
      });
    }
  }
  return walk.issues.length === 0 ? { ok: true } : { ok: false, issues: walk.issues };
}

/**
 * True when `error` is a JavaScript engine reporting that its own call stack
 * ran out — the exception `validate`, `encode` and `decode` turn into a
 * `stack` resource failure rather than a value or schema problem.
 *
 * Deliberately not exported: no subpath of the package offers it. `codec.ts`
 * holds a private copy of this function, kept identical by hand; the two are
 * held in agreement by `tests/stack.test.ts`, which drives one shared table of
 * thrown values through `validate`, `encode` and `decode`.
 *
 * The engines disagree on both the class and the words, so the rule reads the
 * error's `name` and `message` and never `instanceof RangeError`:
 *
 * - V8 (Chrome, Node, Deno) and JavaScriptCore (Safari, Bun): a `RangeError`
 *   reading "Maximum call stack size exceeded".
 * - SpiderMonkey (Firefox): an `InternalError` — not a `RangeError` — reading
 *   "too much recursion".
 *
 * Any other `RangeError` (`Invalid array length`, a BigInt that is too big,
 * an oversized allocation) is not stack exhaustion and is refused, as is a
 * plain `Error` that merely quotes the words. The read is guarded: a thrown
 * value can be `null`, a primitive, or a Proxy whose traps throw, and none of
 * that may escape the choke point that called this.
 *
 * Limitation: a value's getter or a `rec` thunk can throw an error carrying
 * exactly this name and message, and it will be classified as stack
 * exhaustion. The walk still fails closed either way; only the label can be
 * forged.
 */
function isStackExhaustion(error: unknown): boolean {
  try {
    if (typeof error !== "object" || error === null) {
      return false;
    }
    const { name, message } = error as { name?: unknown; message?: unknown };
    return (
      typeof message === "string" &&
      ((name === "RangeError" && message.startsWith("Maximum call stack size exceeded")) ||
        (name === "InternalError" && message.startsWith("too much recursion")))
    );
  } catch {
    return false;
  }
}

// The erased view a walker narrows on. `Schema<T>` is invariant, so `any` is
// the wildcard here for the same reason it is in `AnySchema` — it never leaks
// into `validate`'s signature.
/* eslint-disable @typescript-eslint/no-explicit-any */
type SchemaNode =
  | PrimitiveSchema<any>
  | OptSchema<any>
  | VecSchema<any>
  | BlobSchema
  | UnitSchema
  | RecordSchema<FieldSchemas>
  | TupleSchema<readonly AnySchema[]>
  | VariantSchema<FieldSchemas>
  | FuncSchema
  | ServiceSchema
  | RecSchema<any>;
/* eslint-enable @typescript-eslint/no-explicit-any */

type PathSegment = string | number;

// Mirrors the generator's `is_ts_property_identifier`: ASCII identifier shape
// renders `.key`, everything else renders bracketed and JSON-quoted.
const IDENTIFIER_KEY = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

function renderPath(segments: readonly PathSegment[]): string {
  let out = "$";
  for (const segment of segments) {
    if (typeof segment === "number") {
      out += `[${segment}]`;
    } else if (IDENTIFIER_KEY.test(segment)) {
      out += `.${segment}`;
    } else {
      out += `[${JSON.stringify(segment)}]`;
    }
  }
  return out;
}

/** A short honest description of a value for messages, never its contents. */
function describe(value: unknown): string {
  if (value === null) {
    return "null";
  }
  if (Array.isArray(value)) {
    return "array";
  }
  if (isUint8Array(value)) {
    return "Uint8Array";
  }
  return typeof value;
}

function hasOwn(target: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(target, key);
}

/**
 * Value-side presence: an own **enumerable** property — the projection
 * serialization and spread see. `hasOwn` stays for schema-side maps, which
 * this module builds itself.
 */
function hasOwnEnumerable(target: object, key: string): boolean {
  return Object.prototype.propertyIsEnumerable.call(target, key);
}

// The %TypedArray%.prototype[@@toStringTag] getter reads the internal
// [[TypedArrayName]] slot: it answers "Uint8Array" for a genuine Uint8Array
// from any realm (Buffer subclasses included), and `undefined` for prototype
// forgeries like `Object.create(Uint8Array.prototype)` — which `instanceof`
// gets wrong in both directions. `Symbol.toStringTag` on a plain object
// cannot spoof it.
const typedArrayTag = Object.getOwnPropertyDescriptor(
  Object.getPrototypeOf(Uint8Array.prototype) as object,
  Symbol.toStringTag,
)?.get;

function isUint8Array(value: unknown): value is Uint8Array {
  return typedArrayTag?.call(value) === "Uint8Array";
}

/** A non-null object that is not one of the array-like domain shapes. */
function isPlainCandidate(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" && value !== null && !Array.isArray(value) && !isUint8Array(value)
  );
}

const FIXED_WIDTH_RANGES: Record<string, readonly [min: number, max: number]> = {
  nat8: [0, 255],
  nat16: [0, 65_535],
  nat32: [0, 4_294_967_295],
  int8: [-128, 127],
  int16: [-32_768, 32_767],
  int32: [-2_147_483_648, 2_147_483_647],
};

const NAT64_MAX = 2n ** 64n - 1n;
const INT64_MIN = -(2n ** 63n);
const INT64_MAX = 2n ** 63n - 1n;

// The walk keeps its own work on an explicit stack (issue #192): a composite
// value becomes a frame recording where its loop stands, and the host call
// stack stays a few frames deep however deep the value nests, so `maxDepth`
// is the only depth bound. Each frame is the continuation of what used to be
// a recursive call — the loop index, and whether a child's path segment is
// still to be popped — and resuming it performs exactly the reads, charges,
// path pushes and pops, and issue checks the recursive walk performed, in
// the same order. Order is observable (a getter or Proxy trap sees every
// read, and the first issue's path is the result), so it is kept, down to
// reads that happen after the walk has halted.
/* eslint-disable @typescript-eslint/no-explicit-any */
type Frame =
  | {
      readonly kind: "vec";
      readonly node: VecSchema<any>;
      readonly value: unknown[];
      readonly depth: number;
      /** The element in flight; -1 before the first. */
      index: number;
    }
  | {
      readonly kind: "tuple";
      readonly node: TupleSchema<readonly AnySchema[]>;
      readonly value: unknown[];
      readonly depth: number;
      index: number;
    }
  | {
      readonly kind: "record";
      readonly node: RecordSchema<FieldSchemas>;
      readonly value: Record<string, unknown>;
      readonly entries: readonly (readonly [string, AnyFieldSchema])[];
      readonly depth: number;
      index: number;
      /** Whether the field in flight still has its path segment pushed. */
      pending: boolean;
    }
  | {
      /** A present boxed opt, after `some`: pop it, then check the keys. */
      readonly kind: "boxed";
      readonly value: Record<string, unknown>;
      readonly depth: number;
    }
  | {
      /** A variant, after its payload: pop `value`, then check the keys. */
      readonly kind: "variant";
      readonly value: Record<string, unknown>;
      readonly depth: number;
    };
/* eslint-enable @typescript-eslint/no-explicit-any */

/** What a walk continues with in the same loop: a node, a value, a depth. */
type Next =
  { readonly node: SchemaNode; readonly value: unknown; readonly depth: number } | undefined;

class Walk {
  readonly issues: ValidationIssue[] = [];
  private readonly maxDepth: number;
  private readonly maxElements: number;
  private readonly maxIssues: number;
  private elements = 0;
  private halted = false;
  /** The deepest depth `step` has charged: what a stack overflow reports. */
  private reached = 0;
  /** The explicit work stack: one frame per composite value in progress. */
  private readonly stack: Frame[] = [];

  constructor(options: ValidateOptions) {
    this.maxDepth = options.maxDepth ?? DEFAULT_MAX_DEPTH;
    this.maxElements = options.maxElements ?? DEFAULT_MAX_ELEMENTS;
    this.maxIssues = options.maxIssues ?? DEFAULT_MAX_ISSUES;
  }

  /** Walk `value` against `schema` to completion, or until something throws. */
  visit(schema: AnyFieldSchema, value: unknown, path: PathSegment[], depth: number): void {
    this.enter(schema, value, path, depth);
    const stack = this.stack;
    while (stack.length > 0) {
      this.resume(stack[stack.length - 1], path);
    }
  }

  /**
   * Begin one value: charge its step, check it, and either finish it on the
   * spot (a leaf, or a composite refused before any child) or push the frame
   * that walks its children. What were tail calls — a `rec` hop, an opt's
   * payload, a variant's payload (after pushing the frame that checks its
   * keys) — continue in this same loop, so `enter` never calls itself: the
   * host stack is `visit` → `resume` → `enter` → one helper, at any depth.
   */
  private enter(schema: AnyFieldSchema, value: unknown, path: PathSegment[], depth: number): void {
    let node = schema as SchemaNode;
    let at = depth;
    for (;;) {
      if (this.halted || !this.step(path, at)) {
        return;
      }
      switch (node.kind) {
        case "primitive":
          this.primitive(node, value, path);
          return;
        case "opt": {
          if (value === null) {
            return;
          }
          const next = this.opt(node, value, path, at);
          if (next === undefined) {
            return;
          }
          ({ node, value, depth: at } = next);
          continue;
        }
        case "vec":
          this.vec(node, value, path, at);
          return;
        case "blob":
          if (!isUint8Array(value)) {
            this.issue("invalid_type", path, `expected a Uint8Array, got ${describe(value)}`);
          }
          return;
        case "unit":
          this.unit(value, path, at);
          return;
        case "record":
          this.record(node, value, path, at);
          return;
        case "tuple":
          this.tuple(node, value, path, at);
          return;
        case "variant": {
          const next = this.variant(node, value, path, at);
          if (next === undefined) {
            return;
          }
          ({ node, value, depth: at } = next);
          continue;
        }
        case "func":
          this.func(value, path, at);
          return;
        case "service":
          // A service value is the principal of a running service (issue
          // #104) — the same check the principal primitive uses.
          this.principalText(value, path);
          return;
        case "rec": {
          const body: unknown = node.body();
          if (
            typeof body !== "object" ||
            body === null ||
            typeof (body as { kind?: unknown }).kind !== "string"
          ) {
            this.issue("unsupported_schema", path, "a rec thunk did not produce a schema");
            return;
          }
          node = body as SchemaNode;
          at += 1;
          continue;
        }
        default:
          this.issue(
            "unsupported_schema",
            path,
            `unknown schema kind ${JSON.stringify((node as { kind: unknown }).kind)}`,
          );
          return;
      }
    }
  }

  /**
   * Advance the frame on top of the stack by one child: start the next child
   * (which may push a frame of its own), or finish the frame and pop it.
   */
  private resume(frame: Frame, path: PathSegment[]): void {
    const stack = this.stack;
    switch (frame.kind) {
      case "vec": {
        const { node, value } = frame;
        for (;;) {
          if (frame.index >= 0) {
            path.pop();
          }
          frame.index += 1;
          if (!(frame.index < value.length && !this.halted)) {
            stack.pop();
            return;
          }
          path.push(frame.index);
          this.enter(node.inner, value[frame.index], path, frame.depth + 1);
          // A child that pushed a frame runs first; a leaf is already done,
          // and this loop carries on without a round trip through `visit`.
          if (stack[stack.length - 1] !== frame) {
            return;
          }
        }
      }
      case "tuple": {
        const { node, value } = frame;
        for (;;) {
          if (frame.index >= 0) {
            path.pop();
          }
          frame.index += 1;
          if (!(frame.index < node.elements.length && !this.halted)) {
            stack.pop();
            return;
          }
          path.push(frame.index);
          this.enter(node.elements[frame.index], value[frame.index], path, frame.depth + 1);
          if (stack[stack.length - 1] !== frame) {
            return;
          }
        }
      }
      case "record": {
        const { node, value, entries } = frame;
        for (;;) {
          if (frame.pending) {
            path.pop();
            frame.pending = false;
          }
          frame.index += 1;
          if (frame.index >= entries.length) {
            break;
          }
          if (this.halted) {
            stack.pop();
            return;
          }
          const entry = entries[frame.index];
          const key = entry[0];
          path.push(key);
          if (!hasOwnEnumerable(value, key)) {
            this.issue("missing_field", path, "required field is missing");
            path.pop();
            continue;
          }
          frame.pending = true;
          this.enter(entry[1], value[key], path, frame.depth + 1);
          if (stack[stack.length - 1] !== frame) {
            return;
          }
        }
        stack.pop();
        this.recordKeys(node, value, path, frame.depth);
        return;
      }
      case "boxed":
        path.pop();
        stack.pop();
        this.boxedKeys(frame.value, path, frame.depth);
        return;
      case "variant":
        path.pop();
        stack.pop();
        this.variantKeys(frame.value, path, frame.depth);
        return;
    }
  }

  /** Charge one traversal step; false means the walk is over. */
  private step(path: PathSegment[], depth: number): boolean {
    if (depth > this.reached) {
      this.reached = depth;
    }
    if (depth > this.maxDepth) {
      this.resource("value_depth", this.maxDepth, depth, path);
      return false;
    }
    this.elements += 1;
    if (this.elements > this.maxElements) {
      this.resource("value_elements", this.maxElements, this.elements, path);
      return false;
    }
    return true;
  }

  private issue(code: ValidationCode, path: PathSegment[], message: string): void {
    if (this.halted) {
      return;
    }
    this.issues.push({ code, path: renderPath(path), message });
    if (this.issues.length >= this.maxIssues) {
      this.halted = true;
    }
  }

  private resource(
    resource: ResourceLimitInfo["resource"],
    limit: number,
    observed: number,
    path: PathSegment[],
  ): void {
    if (this.halted) {
      return;
    }
    this.issues.push({
      code: "resource_limit_exceeded",
      path: renderPath(path),
      message: `${resource} limit ${limit} exceeded (observed ${observed})`,
      resource_limit: { resource, limit, observed },
    });
    // A bound failure is terminal: continuing would report `ok`-shaped
    // partial results for a value that was never fully examined.
    this.halted = true;
  }

  /**
   * Record that the host stack ran out mid-walk. Not routed through
   * `resource`: that method drops the issue when the walk has halted, and a
   * caught exception must always leave a trace.
   */
  stackExhausted(path: PathSegment[]): void {
    this.issues.push({
      code: "resource_limit_exceeded",
      path: renderPath(path),
      message:
        `the host stack was exhausted at depth ${this.reached} (the configured maxDepth ` +
        `is ${this.maxDepth}); use a shallower input or schema, or a host with a larger stack`,
      resource_limit: { resource: "stack", limit: this.maxDepth, observed: this.reached },
    });
    this.halted = true;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private primitive(node: PrimitiveSchema<any>, value: unknown, path: PathSegment[]): void {
    const name = node.primitive;
    switch (name) {
      case "null":
        if (value !== null) {
          this.issue("invalid_type", path, `expected null, got ${describe(value)}`);
        }
        return;
      case "bool":
        if (typeof value !== "boolean") {
          this.issue("invalid_type", path, `expected a boolean, got ${describe(value)}`);
        }
        return;
      case "text":
        if (typeof value !== "string") {
          this.issue("invalid_type", path, `expected a string, got ${describe(value)}`);
        }
        return;
      case "nat":
      case "int":
      case "nat64":
      case "int64": {
        if (typeof value !== "bigint") {
          // `number` is deliberately rejected rather than coerced: these
          // types exceed 2^53 on the wire and a lossy bridge would corrupt.
          this.issue("invalid_type", path, `expected a bigint for ${name}, got ${describe(value)}`);
          return;
        }
        if (name === "nat" && value < 0n) {
          this.issue("out_of_range", path, `nat must be non-negative, got ${value}n`);
        } else if (name === "nat64" && (value < 0n || value > NAT64_MAX)) {
          this.issue("out_of_range", path, `nat64 must be in [0, 2^64-1], got ${value}n`);
        } else if (name === "int64" && (value < INT64_MIN || value > INT64_MAX)) {
          this.issue("out_of_range", path, `int64 must be in [-2^63, 2^63-1], got ${value}n`);
        }
        return;
      }
      case "nat8":
      case "nat16":
      case "nat32":
      case "int8":
      case "int16":
      case "int32": {
        if (typeof value !== "number") {
          this.issue("invalid_type", path, `expected a number for ${name}, got ${describe(value)}`);
          return;
        }
        if (!Number.isInteger(value)) {
          this.issue("not_integer", path, `${name} requires an integer, got ${value}`);
          return;
        }
        const [min, max] = FIXED_WIDTH_RANGES[name];
        if (value < min || value > max) {
          this.issue("out_of_range", path, `${name} must be in [${min}, ${max}], got ${value}`);
        }
        return;
      }
      case "float32":
      case "float64":
        if (typeof value !== "number") {
          this.issue("invalid_type", path, `expected a number for ${name}, got ${describe(value)}`);
        }
        return;
      case "reserved":
        // Accepts anything, asserts nothing.
        return;
      case "empty":
        this.issue("uninhabited_type", path, "empty has no values");
        return;
      case "principal":
        this.principalText(value, path);
        return;
      default:
        this.issue("unsupported_schema", path, `unknown primitive ${JSON.stringify(name)}`);
    }
  }

  /**
   * A present `opt` value. Whether it is boxed is decided here, on the
   * resolved inner node (the `isBoxedOpt` rule), never on the schema object: the
   * inner is resolved once, charging each rec hop exactly as visiting it
   * would, and the resolved node is what validates the payload — so an
   * unboxed opt's accounting is unchanged. A boxed value is strict like a
   * record: a plain object whose only own enumerable key is `some`.
   *
   * Returns what the caller's loop visits next: the value itself at the
   * resolved inner node, or — for a boxed value, after pushing the frame that
   * finishes it — its `some` payload.
   */
  private opt(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    node: OptSchema<any>,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): Next {
    const inner = this.resolve(node.inner, path, depth + 1);
    if (inner === undefined) {
      return undefined;
    }
    if (!admitsNull(inner.node)) {
      return { node: inner.node, value, depth: inner.depth };
    }
    if (!isPlainCandidate(value)) {
      this.issue(
        "invalid_type",
        path,
        `expected null or { some: … } for an opt whose inner type admits null, got ${describe(value)}`,
      );
      return undefined;
    }
    path.push("some");
    if (!hasOwnEnumerable(value, "some")) {
      this.issue("missing_field", path, "a present boxed opt carries { some }");
      path.pop();
      this.boxedKeys(value, path, depth);
      return undefined;
    }
    this.stack.push({ kind: "boxed", value, depth });
    return { node: inner.node, value: value.some, depth: inner.depth };
  }

  /** A boxed opt's keys, after `some`: anything but `some` is unexpected. */
  private boxedKeys(value: Record<string, unknown>, path: PathSegment[], depth: number): void {
    for (const key of Object.keys(value)) {
      if (this.halted || !this.step(path, depth)) {
        return;
      }
      if (key !== "some") {
        path.push(key);
        this.issue("unexpected_field", path, "a present boxed opt is exactly { some }");
        path.pop();
      }
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private vec(node: VecSchema<any>, value: unknown, path: PathSegment[], depth: number): void {
    if (!Array.isArray(value)) {
      this.issue("invalid_type", path, `expected an array, got ${describe(value)}`);
      return;
    }
    this.stack.push({ kind: "vec", node, value, depth, index: -1 });
  }

  private unit(value: unknown, path: PathSegment[], depth: number): void {
    if (!isPlainCandidate(value)) {
      this.issue("invalid_type", path, `expected an empty record, got ${describe(value)}`);
      return;
    }
    for (const key of Object.keys(value)) {
      // Examined keys are traversal work: charge them so a hostile value
      // with millions of keys fails closed at the configured budget.
      if (this.halted || !this.step(path, depth)) {
        return;
      }
      path.push(key);
      this.issue("unexpected_field", path, "the empty record has no fields");
      path.pop();
    }
  }

  private record(
    node: RecordSchema<FieldSchemas>,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): void {
    if (!isPlainCandidate(value)) {
      this.issue("invalid_type", path, `expected a record, got ${describe(value)}`);
      return;
    }
    // Fields in the schema object's enumeration order; the frame then walks
    // them one at a time (`resume`) and ends with `recordKeys`.
    const entries = Object.entries(node.fields);
    this.stack.push({ kind: "record", node, value, entries, depth, index: -1, pending: false });
  }

  /** A record's keys, after its fields: every key not a field is unexpected. */
  private recordKeys(
    node: RecordSchema<FieldSchemas>,
    value: Record<string, unknown>,
    path: PathSegment[],
    depth: number,
  ): void {
    for (const key of Object.keys(value)) {
      // Examined keys are traversal work: charge them so a hostile value
      // with millions of keys fails closed at the configured budget.
      if (this.halted || !this.step(path, depth)) {
        return;
      }
      // Own-key membership, not `in`: a value key like "toString" must not
      // pass because it exists on the fields object's prototype.
      if (!hasOwn(node.fields, key)) {
        path.push(key);
        this.issue("unexpected_field", path, "field is not part of this record");
        path.pop();
      }
    }
  }

  private tuple(
    node: TupleSchema<readonly AnySchema[]>,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): void {
    if (!Array.isArray(value)) {
      this.issue("invalid_type", path, `expected a tuple array, got ${describe(value)}`);
      return;
    }
    if (value.length !== node.elements.length) {
      this.issue(
        "invalid_length",
        path,
        `expected ${node.elements.length} elements, got ${value.length}`,
      );
      return;
    }
    this.stack.push({ kind: "tuple", node, value, depth, index: -1 });
  }

  /** A variant value; returns its payload for the caller's loop to visit. */
  private variant(
    node: VariantSchema<FieldSchemas>,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): Next {
    if (!isPlainCandidate(value)) {
      this.issue("invalid_type", path, `expected a variant, got ${describe(value)}`);
      return;
    }
    path.push("tag");
    if (!hasOwnEnumerable(value, "tag")) {
      this.issue("missing_field", path, "a variant value carries a tag");
      path.pop();
      return;
    }
    const tag = value.tag;
    if (typeof tag !== "string") {
      this.issue("invalid_type", path, `expected a string tag, got ${describe(tag)}`);
      path.pop();
      return;
    }
    if (!hasOwn(node.arms, tag)) {
      this.issue("unknown_tag", path, `${JSON.stringify(tag)} is not an arm of this variant`);
      path.pop();
      return;
    }
    path.pop();
    // Arm classification must see through `rec`: a dynamically built arm
    // wraps its payload schema in a lazy thunk, and `{ tag }` versus
    // `{ tag, value }` is decided by the resolved payload, exactly as the
    // generator decides it by the payload *node*. The resolved node is then
    // what validates the payload, so each rec hop is charged exactly once —
    // the documented one-extra-element-per-hop model.
    const arm = this.resolve(node.arms[tag], path, depth);
    if (arm === undefined) {
      return;
    }
    const tagOnly = arm.node.kind === "primitive" && arm.node.primitive === "null";
    if (tagOnly) {
      for (const key of Object.keys(value)) {
        if (this.halted || !this.step(path, depth)) {
          return;
        }
        if (key !== "tag") {
          path.push(key);
          this.issue("unexpected_field", path, "a null-payload arm is a bare { tag }");
          path.pop();
        }
      }
      return;
    }
    path.push("value");
    if (!hasOwnEnumerable(value, "value")) {
      this.issue("missing_field", path, "this arm carries a payload");
      path.pop();
      this.variantKeys(value, path, depth);
      return undefined;
    }
    this.stack.push({ kind: "variant", value, depth });
    return { node: arm.node, value: value.value, depth: arm.depth + 1 };
  }

  /** A variant's keys, after its payload: anything but `tag`/`value` is unexpected. */
  private variantKeys(value: Record<string, unknown>, path: PathSegment[], depth: number): void {
    for (const key of Object.keys(value)) {
      if (this.halted || !this.step(path, depth)) {
        return;
      }
      if (key !== "tag" && key !== "value") {
        path.push(key);
        this.issue("unexpected_field", path, "a variant value is { tag, value }");
        path.pop();
      }
    }
  }

  /**
   * A `Principal`: a string holding canonical principal text. The encoder
   * applies the same two checks with the same code, path, and messages.
   */
  private principalText(value: unknown, path: PathSegment[]): void {
    if (typeof value !== "string") {
      this.issue(
        "invalid_type",
        path,
        `expected a Principal (canonical principal text), got ${describe(value)}`,
      );
    } else if (!isPrincipal(value)) {
      this.issue(
        "invalid_type",
        path,
        "expected a Principal, got a string that is not canonical principal text",
      );
    }
  }

  /**
   * A func value is `{ principal, method }` — inert reference data, the
   * modern shape recorded on issue #104. Strict like records: both keys
   * required (own enumerable), the method non-empty, nothing else present.
   */
  private func(value: unknown, path: PathSegment[], depth: number): void {
    if (!isPlainCandidate(value)) {
      this.issue(
        "invalid_type",
        path,
        `expected a func reference ({ principal, method }), got ${describe(value)}`,
      );
      return;
    }
    path.push("principal");
    if (!hasOwnEnumerable(value, "principal")) {
      this.issue("missing_field", path, "a func reference names a principal");
    } else {
      this.principalText(value.principal, path);
    }
    path.pop();
    path.push("method");
    if (!hasOwnEnumerable(value, "method")) {
      this.issue("missing_field", path, "a func reference names a method");
    } else if (typeof value.method !== "string" || value.method.length === 0) {
      this.issue("invalid_type", path, "a method name is a non-empty string");
    }
    path.pop();
    for (const key of Object.keys(value)) {
      if (this.halted || !this.step(path, depth)) {
        return;
      }
      if (key !== "principal" && key !== "method") {
        path.push(key);
        this.issue("unexpected_field", path, "a func reference is { principal, method }");
        path.pop();
      }
    }
  }

  /**
   * Unwrap `rec` chains to the structural node beneath, charging depth once
   * per hop; the returned depth is where the unwrapping ended, so the caller
   * continues from it instead of re-walking the chain. `undefined` means the
   * walk halted or the schema is unusable.
   */
  private resolve(
    schema: AnyFieldSchema,
    path: PathSegment[],
    depth: number,
  ): { node: SchemaNode; depth: number } | undefined {
    let node = schema as SchemaNode;
    let hops = depth;
    while (node.kind === "rec") {
      hops += 1;
      if (!this.step(path, hops)) {
        return undefined;
      }
      const body: unknown = node.body();
      if (
        typeof body !== "object" ||
        body === null ||
        typeof (body as { kind?: unknown }).kind !== "string"
      ) {
        this.issue("unsupported_schema", path, "a rec thunk did not produce a schema");
        return undefined;
      }
      node = body as SchemaNode;
    }
    return { node, depth: hops };
  }
}

// Result unwrapping — issue #151. `variant { ok : T; err : E }` is the
// universal canister result convention, and the ecosystem unwraps it by
// probing a decoded *value* for `ok`/`err` keys: a guess that misfires on any
// record legitimately carrying those fields, and that cannot type the error
// payload because a value knows nothing about the arms it does not inhabit.
// Whether a method's reply *is* a result, and what each arm carries, is a
// schema fact — so it is read off the schema here.
//
// The recognised set is two conventions rather than four spellings: Motoko's
// `Result.Result` emits `ok`/`err`, Rust's candid derive emits `Ok`/`Err`, and
// each is matched as a *pair*, in either arm order, with no third arm. A mixed
// pair is what no generator produces, so admitting one would be unwrapping on
// coincidence — the failure this helper replaces. Decisions recorded on the
// issue, alongside the bare-tag payload (`null`, the arm's own type) and this
// module as the export home.
const RESULT_PAIRS: readonly (readonly [ok: string, err: string])[] = [
  ["ok", "err"],
  ["Ok", "Err"],
];

/** The two arm names a result schema carries, in this schema's spelling. */
interface ResultArms {
  readonly okTag: string;
  readonly errTag: string;
}

/**
 * Classify a schema as a result variant, or `undefined` if it is not one.
 * Own enumerable arm names, which is the projection every walk in this
 * package that *enumerates* arms uses — the codec's type table and the form
 * model; a name reached only through the arms object's prototype is not an
 * arm, and neither is a third one. The two walks that test arm *membership*
 * for a tag, this file's own and the codec's, use `hasOwn`, so a hand-built
 * arms map carrying a non-enumerable arm is a schema the classifier and the
 * validator would read differently. Nothing builds one — the builders take an
 * object literal and the Contract loader assigns into a null-prototype map —
 * and the unwrapper fails closed on the difference, reporting such a tag as
 * an issue rather than as an arm.
 */
function resultArms(schema: AnyFieldSchema): ResultArms | undefined {
  const node = resolveSchema(schema);
  if (node.kind !== "variant") {
    return undefined;
  }
  const names = Object.keys(node.arms);
  if (names.length !== 2) {
    return undefined;
  }
  for (const [okTag, errTag] of RESULT_PAIRS) {
    if (names.includes(okTag) && names.includes(errTag)) {
      return { okTag, errTag };
    }
  }
  return undefined;
}

/**
 * The static type a result schema's ok arm carries: the arm's own domain
 * type, or `null` when the arm is a bare tag — a bare arm's payload is Candid
 * `null`, whose one JavaScript value is `null`. `never` when the schema
 * describes no such arm.
 *
 * It reads one arm and nothing else: whether a schema is a result *at all* is
 * [`isResultSchema`]'s answer, and the exactly-two-arms rule lives there, so
 * this alias still names a payload for a variant that has an ok arm among
 * several.
 *
 * @example
 * const Transfer = c.variant({ ok: c.nat, err: c.text });
 * type Balance = ResultOk<typeof Transfer>; // bigint
 */
export type ResultOk<S> = ArmPayload<Extract<Infer<S>, { tag: "ok" } | { tag: "Ok" }>>;

/**
 * The static type a result schema's err arm carries, on the same rule as
 * [`ResultOk`] — which is the half a value-probing unwrapper cannot type at
 * all.
 *
 * @example
 * const Transfer = c.variant({ ok: c.nat, err: c.text });
 * type Failure = ResultErr<typeof Transfer>; // string
 */
export type ResultErr<S> = ArmPayload<Extract<Infer<S>, { tag: "err" } | { tag: "Err" }>>;

// A bare-tag arm is `{ tag }` with no `value` property, and its payload type
// is the `null` its arm schema describes.
type ArmPayload<A> = A extends { value: infer V } ? V : null;

/**
 * What [`unwrapResult`] returns: the ok arm, the err arm, or the value not
 * being of this schema at all — that last one carrying exactly the `issues`
 * a failed [`validate`] call carries.
 *
 * Every member declares all three keys, the ones it does not hold as optional
 * `undefined`, so `ok`, `error`, and `issues` can each be read and narrowed on
 * directly rather than through a type guard.
 *
 * @example
 * const outcome: UnwrapResult<bigint, string> = { ok: true, value: 5n };
 * outcome.issues === undefined; // readable on every member, no type guard
 */
export type UnwrapResult<T, E> =
  | {
      readonly ok: true;
      readonly value: T;
      readonly error?: undefined;
      readonly issues?: undefined;
    }
  | {
      readonly ok: false;
      readonly value?: undefined;
      readonly error: E;
      readonly issues?: undefined;
    }
  | {
      readonly ok: false;
      readonly value?: undefined;
      readonly error?: undefined;
      readonly issues: readonly ValidationIssue[];
    };

/**
 * Does this schema describe the `variant { ok; err }` result convention?
 * True for a schema that resolves — through the `rec` indirections every
 * generated declaration and every runtime-loaded edge arrives wrapped in — to
 * a variant whose arms are **exactly** an ok arm and an err arm, spelled
 * either `ok`/`err` or `Ok`/`Err`, in either order. A record carrying those
 * field names is not a result; neither is a variant that adds a third arm,
 * nor one mixing the two spellings.
 *
 * Resolution is the shared one, so this raises the `TypeError`s it does: on
 * something that is not a schema object — including the mistake this helper
 * exists to prevent, a decoded *value* passed where its schema belongs — on a
 * `kind` this package does not define, and on a `rec` chain that runs past the
 * hop limit instead of terminating. All three are programmer errors; a schema
 * this package can read is answered `true` or `false`, never by an exception.
 *
 * @example
 * isResultSchema(c.variant({ Ok: c.nat, Err: c.text })); // true
 * isResultSchema(c.record({ ok: c.nat, err: c.text })); // false
 */
export function isResultSchema(schema: AnyFieldSchema): boolean {
  return resultArms(schema) !== undefined;
}

/**
 * Read a result value into `{ ok: true, value }` or `{ ok: false, error }`,
 * both typed from the schema's own arms. An err arm is a value, not an
 * exception: nothing here throws for one, and nothing throws for a malformed
 * value either — that comes back as `{ ok: false, issues }`, the same issue
 * list [`validate`] produces, which is what makes the typed payload above an
 * honest claim rather than a cast.
 *
 * A bare-tag arm — `variant { ok; err : text }` — unwraps to `null`, the
 * single value of the Candid `null` its arm describes.
 *
 * The payload is read after that check rather than during it, so the value
 * handed back is the checked one for every value that is inert data — which
 * every decoded one is. A live object whose accessors answer differently on
 * successive reads can hand back a payload the check never saw; the tag is
 * re-read and re-checked, so such a value still cannot land outside this
 * schema's own two arms.
 *
 * Throws `TypeError`, eagerly and before touching the value, on a schema that
 * is not a result variant, and on options `validate` refuses (an unknown key,
 * or a limit that is not a non-negative safe integer). Both are programmer
 * errors, and it is the same treatment `resolveSchema` and `serviceMethods`
 * give theirs.
 *
 * @example
 * const Transfer = c.variant({ ok: c.nat, err: c.text });
 * const outcome = unwrapResult(Transfer, { tag: "ok", value: 5n });
 * outcome.ok === true; // and outcome.value is 5n, typed bigint
 */
export function unwrapResult<S extends AnyFieldSchema>(
  schema: S,
  value: unknown,
  options: ValidateOptions = {},
): UnwrapResult<ResultOk<S>, ResultErr<S>> {
  const arms = resultArms(schema);
  if (arms === undefined) {
    throw new TypeError("unwrapResult needs a result variant schema");
  }
  const checked = validateWith(
    checkOptions("unwrapResult", options, VALIDATE_LIMIT_KEYS),
    schema,
    value,
  );
  if (!checked.ok) {
    return { ok: false, issues: checked.issues };
  }
  // The walk above passed, so the value is a plain object carrying one of
  // this variant's tags, and the arm's own shape decided whether a `value`
  // key is there: a bare-tag arm rejects one, every other arm demands it as
  // an own enumerable property. Reading that presence is therefore reading
  // the classification validation just made — and its absence yields `null`,
  // the arm's payload type, rather than the `undefined` a blind `.value`
  // read produces. It also keeps a non-enumerable `value` property, which
  // the walk never saw, out of an unwrapped payload.
  //
  // These reads happen after the walk rather than inside it, so a value
  // whose accessors answer differently on a second read is the one shape
  // that can hand back a payload the walk did not examine — in either
  // direction, since the arm the second read names is the arm the payload is
  // reported under. The tag is re-checked below to bound that: a drifted tag
  // outside the pair is an issue rather than a classification, so such a
  // value can never leave this schema's own two arms. Decoded values are
  // inert data and cannot do any of it; both directions are pinned in
  // tests/result.test.ts as the limitation they are.
  let tag: unknown;
  let payload: unknown = null;
  let shown = "";
  let reading = "$.tag";
  try {
    tag = (value as { tag: unknown }).tag;
    reading = "$.value";
    if (hasOwnEnumerable(value as object, "value")) {
      payload = (value as { value: unknown }).value;
    }
    if (tag !== arms.okTag && tag !== arms.errTag) {
      // Describing the tag *reads* it: `describe` asks `Array.isArray`
      // first, which a revoked Proxy answers by throwing. So the message for
      // the drifted-tag return below is built here, inside the guard, rather
      // than at the return itself.
      reading = "$.tag";
      shown = typeof tag === "string" ? JSON.stringify(tag) : describe(tag);
    }
  } catch {
    return {
      ok: false,
      issues: [
        {
          code: "unreadable_value",
          path: reading,
          message: "the value threw while being inspected",
        },
      ],
    };
  }
  if (tag === arms.okTag) {
    return { ok: true, value: payload as ResultOk<S> };
  }
  if (tag === arms.errTag) {
    return { ok: false, error: payload as ResultErr<S> };
  }
  return {
    ok: false,
    issues: [
      {
        code: "unknown_tag",
        path: "$.tag",
        message: `${shown} is not this result's ok or err arm`,
      },
    ],
  };
}
