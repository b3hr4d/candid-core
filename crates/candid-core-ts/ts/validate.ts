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
// # Strictness decisions (fail closed, recorded on issue #102)
//
// - `nat`/`int`/`nat64`/`int64` require `bigint`; a `number` there is
//   rejected, never coerced. Fixed-width integers must be integral and in
//   range; 255 passes `nat8`, 256 does not.
// - Records reject unknown keys and require every declared key, including
//   `opt` fields: the domain shape is `T | null` with the property present.
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

import type {
  AnySchema,
  BlobSchema,
  FieldSchemas,
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
  | "resource_limit_exceeded";

/** The `{resource, limit, observed}` triple, as candid-core serializes it. */
export interface ResourceLimitInfo {
  readonly resource: "value_depth" | "value_elements";
  readonly limit: number;
  readonly observed: number;
}

export interface ValidationIssue {
  readonly code: ValidationCode;
  /** `$`-rooted path to the offending value, candid-core style. */
  readonly path: string;
  readonly message: string;
  readonly resource_limit?: ResourceLimitInfo;
}

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

export type ValidateResult =
  | { readonly ok: true }
  | { readonly ok: false; readonly issues: readonly ValidationIssue[] };

export const DEFAULT_MAX_DEPTH = 256;
export const DEFAULT_MAX_ELEMENTS = 1_000_000;
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
  walk.visit(schema, value, [], 0);
  return walk.issues.length === 0
    ? { ok: true }
    : { ok: false, issues: walk.issues };
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
  if (value instanceof Uint8Array) {
    return "Uint8Array";
  }
  return typeof value;
}

function hasOwn(target: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(target, key);
}

/** A non-null object that is not one of the array-like domain shapes. */
function isPlainCandidate(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    !(value instanceof Uint8Array)
  );
}

const FIXED_WIDTH_RANGES: Record<
  string,
  readonly [min: number, max: number]
> = {
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

  visit(
    schema: AnySchema,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): void {
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
        if (!(value instanceof Uint8Array)) {
          this.issue(
            "invalid_type",
            path,
            `expected a Uint8Array, got ${describe(value)}`,
          );
        }
        return;
      case "unit":
        this.unit(value, path);
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
      case "rec": {
        const body: unknown = node.body();
        if (
          typeof body !== "object" ||
          body === null ||
          typeof (body as { kind?: unknown }).kind !== "string"
        ) {
          this.issue(
            "unsupported_schema",
            path,
            "a rec thunk did not produce a schema",
          );
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
  private primitive(
    node: PrimitiveSchema<any>,
    value: unknown,
    path: PathSegment[],
  ): void {
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
          this.issue(
            "invalid_type",
            path,
            `expected a bigint for ${name}, got ${describe(value)}`,
          );
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
          this.issue(
            "invalid_type",
            path,
            `expected a number for ${name}, got ${describe(value)}`,
          );
          return;
        }
        if (!Number.isInteger(value)) {
          this.issue("not_integer", path, `${name} requires an integer, got ${value}`);
          return;
        }
        const [min, max] = FIXED_WIDTH_RANGES[name];
        if (value < min || value > max) {
          this.issue(
            "out_of_range",
            path,
            `${name} must be in [${min}, ${max}], got ${value}`,
          );
        }
        return;
      }
      case "float32":
      case "float64":
        if (typeof value !== "number") {
          this.issue(
            "invalid_type",
            path,
            `expected a number for ${name}, got ${describe(value)}`,
          );
        }
        return;
      case "reserved":
        // Accepts anything, asserts nothing.
        return;
      case "empty":
        this.issue("uninhabited_type", path, "empty has no values");
        return;
      case "principal":
        if (
          (typeof value !== "object" && typeof value !== "function") ||
          value === null ||
          typeof (value as { toText?: unknown }).toText !== "function"
        ) {
          this.issue(
            "invalid_type",
            path,
            `expected a Principal (an object with a toText method), got ${describe(value)}`,
          );
        }
        return;
      default:
        this.issue(
          "unsupported_schema",
          path,
          `unknown primitive ${JSON.stringify(name)}`,
        );
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private vec(
    node: VecSchema<any>,
    value: unknown,
    path: PathSegment[],
    depth: number,
  ): void {
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

  private unit(value: unknown, path: PathSegment[]): void {
    if (!isPlainCandidate(value)) {
      this.issue(
        "invalid_type",
        path,
        `expected an empty record, got ${describe(value)}`,
      );
      return;
    }
    for (const key of Object.keys(value)) {
      if (this.halted) {
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
      if (!hasOwn(value, key)) {
        this.issue("missing_field", path, "required field is missing");
      } else {
        this.visit(field, value[key], path, depth + 1);
      }
      path.pop();
    }
    for (const key of Object.keys(value)) {
      if (this.halted) {
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
    if (!hasOwn(value, "tag")) {
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
    // generator decides it by the payload *node*.
    const arm = this.resolve(node.arms[tag], path, depth);
    if (arm === undefined) {
      return;
    }
    const tagOnly = arm.kind === "primitive" && arm.primitive === "null";
    if (tagOnly) {
      for (const key of Object.keys(value)) {
        if (this.halted) {
          return;
        }
        if (key !== "tag") {
          path.push(key);
          this.issue(
            "unexpected_field",
            path,
            "a null-payload arm is a bare { tag }",
          );
          path.pop();
        }
      }
      return;
    }
    path.push("value");
    if (!hasOwn(value, "value")) {
      this.issue("missing_field", path, "this arm carries a payload");
      path.pop();
    } else {
      this.visit(node.arms[tag], value.value, path, depth + 1);
      path.pop();
    }
    for (const key of Object.keys(value)) {
      if (this.halted) {
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
   * Unwrap `rec` chains to the structural node beneath, charging depth for
   * each hop. `undefined` means the walk halted or the schema is unusable.
   */
  private resolve(
    schema: AnySchema,
    path: PathSegment[],
    depth: number,
  ): SchemaNode | undefined {
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
    return node;
  }
}
