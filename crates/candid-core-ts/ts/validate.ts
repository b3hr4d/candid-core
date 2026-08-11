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
// Validation never throws on any input value: malformed values produce
// issues, and hostile ones — cyclic objects, huge arrays, adversarially deep
// nesting — hit explicit limits that mirror candid-core's `Limits` defaults
// (`maxDepth` 256 like `max_value_depth`, `maxElements` 1_000_000). A limit
// failure is itself an issue (`resource_limit_exceeded`, resource
// `value_depth` or `value_elements`) and terminates the walk, so a truncated
// validation can never report `ok`. Depth counts schema traversal steps —
// `rec` unwrapping included, which is what makes a mis-built self-referential
// `rec` chain terminate — so linked-list-shaped data consumes depth per
// element, exactly as it does in candid-core's value domain.
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
// - Tag-only variant arms (`null` payload) reject a present `value` key; the
//   one domain shape is `{ tag }`, not `{ tag, value: null }`.
// - `float32` accepts any JavaScript number: f32 representability is a codec
//   concern, and rejecting `0.1` here would fail values the domain types
//   deliberately admit. NaN and infinities are valid Candid floats.
// - `principal` is checked structurally (an object with a `toText` function):
//   this harness deliberately has no runtime dependency on a Principal
//   implementation, and the type stub carries no brand to test for.
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
// validation plus one typed read, and because the root entry imports nothing
// at runtime while the actor factory, the form-model builder, and the
// Contract loader all import *it* — so a validator dependency there would
// have arrived with `formModel` and `schemaFromContract` for consumers who
// never asked for one.

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
import { resolveSchema } from "./schema.ts";

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

/** The `{resource, limit, observed}` triple, as candid-core serializes it. */
export interface ResourceLimitInfo {
  readonly resource: "value_depth" | "value_elements";
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

/** Bounds on one validation walk; each defaults to the `DEFAULT_MAX_*` below. */
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

/**
 * Validate `value` against `schema`. Never throws on any `value`; a schema
 * object that is not one this core constructs fails closed with
 * `unsupported_schema`.
 */
export function validate<T>(
  schema: Schema<T>,
  value: unknown,
  options: ValidateOptions = {},
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
  } catch {
    walk.issues.push({
      code: "unreadable_value",
      path: renderPath(path),
      message: "the value threw while being inspected",
    });
  }
  return walk.issues.length === 0 ? { ok: true } : { ok: false, issues: walk.issues };
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

class Walk {
  readonly issues: ValidationIssue[] = [];
  private readonly maxDepth: number;
  private readonly maxElements: number;
  private readonly maxIssues: number;
  private elements = 0;
  private halted = false;

  constructor(options: ValidateOptions) {
    this.maxDepth = options.maxDepth ?? DEFAULT_MAX_DEPTH;
    this.maxElements = options.maxElements ?? DEFAULT_MAX_ELEMENTS;
    this.maxIssues = options.maxIssues ?? DEFAULT_MAX_ISSUES;
  }

  visit(schema: AnyFieldSchema, value: unknown, path: PathSegment[], depth: number): void {
    if (this.halted || !this.step(path, depth)) {
      return;
    }
    const node = schema as SchemaNode;
    switch (node.kind) {
      case "primitive":
        this.primitive(node, value, path);
        return;
      case "opt":
        if (value !== null) {
          this.visit(node.inner, value, path, depth + 1);
        }
        return;
      case "vec":
        this.vec(node, value, path, depth);
        return;
      case "blob":
        if (!isUint8Array(value)) {
          this.issue("invalid_type", path, `expected a Uint8Array, got ${describe(value)}`);
        }
        return;
      case "unit":
        this.unit(value, path, depth);
        return;
      case "record":
        this.record(node, value, path, depth);
        return;
      case "tuple":
        this.tuple(node, value, path, depth);
        return;
      case "variant":
        this.variant(node, value, path, depth);
        return;
      case "func":
        this.func(value, path, depth);
        return;
      case "service":
        // A service value is the principal of a running service (issue
        // #104) — the same structural check the principal primitive uses.
        this.principalShaped(value, path, "a service value is a Principal");
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
        this.visit(body as AnySchema, value, path, depth + 1);
        return;
      }
      default:
        this.issue(
          "unsupported_schema",
          path,
          `unknown schema kind ${JSON.stringify((node as { kind: unknown }).kind)}`,
        );
    }
  }

  /** Charge one traversal step; false means the walk is over. */
  private step(path: PathSegment[], depth: number): boolean {
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
        this.principalShaped(
          value,
          path,
          `expected a Principal (an object with a toText method), got ${describe(value)}`,
        );
        return;
      default:
        this.issue("unsupported_schema", path, `unknown primitive ${JSON.stringify(name)}`);
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private vec(node: VecSchema<any>, value: unknown, path: PathSegment[], depth: number): void {
    if (!Array.isArray(value)) {
      this.issue("invalid_type", path, `expected an array, got ${describe(value)}`);
      return;
    }
    for (let index = 0; index < value.length && !this.halted; index += 1) {
      path.push(index);
      this.visit(node.inner, value[index], path, depth + 1);
      path.pop();
    }
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
    for (const [key, field] of Object.entries(node.fields)) {
      if (this.halted) {
        return;
      }
      path.push(key);
      if (!hasOwnEnumerable(value, key)) {
        this.issue("missing_field", path, "required field is missing");
      } else {
        this.visit(field, value[key], path, depth + 1);
      }
      path.pop();
    }
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
    for (let index = 0; index < node.elements.length && !this.halted; index += 1) {
      path.push(index);
      this.visit(node.elements[index], value[index], path, depth + 1);
      path.pop();
    }
  }

  private variant(
    node: VariantSchema<FieldSchemas>,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): void {
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
    } else {
      this.visit(arm.node, value.value, path, arm.depth + 1);
      path.pop();
    }
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

  private principalShaped(value: unknown, path: PathSegment[], message: string): void {
    if (
      (typeof value !== "object" && typeof value !== "function") ||
      value === null ||
      typeof (value as { toText?: unknown }).toText !== "function"
    ) {
      this.issue("invalid_type", path, message);
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
      this.principalShaped(
        value.principal,
        path,
        `expected a Principal (an object with a toText method), got ${describe(value.principal)}`,
      );
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
 * is not a result variant. That is a programmer error, and it is the same
 * treatment `resolveSchema` and `serviceMethods` give theirs.
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
  const checked = validate(schema as Schema<unknown>, value, options);
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
