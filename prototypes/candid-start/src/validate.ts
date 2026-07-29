// Structural validation of domain values against schema-core objects.
//
// This is a *prototype stand-in* for the issue #102 runtime (`validate` +
// `schemaFromContract` over Contract JSON). It follows #102's recorded
// constraints — modern domain shapes, `bigint` for the unbounded and 64-bit
// integers with no coercion, range-checked fixed-width integers, bounded
// depth and bounded work, path-addressed issues with stable codes, canonical
// field order for issue ordering — so the framework can swap in the #102
// implementation when it lands. Where the two diverge, #102 wins; this file
// makes no claim on that issue.
import type { Schema } from "./schema.ts";
import { isPrincipalLike } from "./principal.ts";
import {
  MAX_REC_DEPTH,
  ownEntry,
  PRIMITIVE_KINDS,
  resolveShape,
  shapeOf,
  type AnySchema,
  type SchemaShape,
} from "./walk.ts";

export interface ValidationIssue {
  /** Stable machine-readable code; message text is not stable surface. */
  readonly code: string;
  /** `$`-rooted path to the offending value, candid-core diagnostic style. */
  readonly path: string;
  readonly message: string;
}

export interface ValidationLimits {
  /** Maximum value nesting depth before failing closed. */
  readonly maxDepth: number;
  /** Maximum visited value nodes — bounds total validation work. */
  readonly maxWork: number;
  /** Maximum collected issues before reporting truncation and stopping. */
  readonly maxIssues: number;
}

export const DEFAULT_VALIDATION_LIMITS: ValidationLimits = {
  maxDepth: 64,
  maxWork: 1_000_000,
  maxIssues: 64,
};

export type ValidationResult =
  | { readonly ok: true }
  | { readonly ok: false; readonly issues: readonly ValidationIssue[] };

const NUMBER_RANGES: Record<string, readonly [number, number]> = {
  nat8: [0, 255],
  nat16: [0, 65_535],
  nat32: [0, 4_294_967_295],
  int8: [-128, 127],
  int16: [-32_768, 32_767],
  int32: [-2_147_483_648, 2_147_483_647],
};

const NAT64_MAX = (1n << 64n) - 1n;
const INT64_MIN = -(1n << 63n);
const INT64_MAX = (1n << 63n) - 1n;

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

type PathSegment = string | number;

function renderPath(segments: readonly PathSegment[]): string {
  let out = "$";
  for (const segment of segments) {
    if (typeof segment === "number") {
      out += `[${segment}]`;
    } else if (IDENTIFIER.test(segment)) {
      out += `.${segment}`;
    } else {
      out += `[${JSON.stringify(segment)}]`;
    }
  }
  return out;
}

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

class Walker {
  readonly issues: ValidationIssue[] = [];
  private readonly path: PathSegment[] = [];
  private readonly limits: ValidationLimits;
  private work = 0;
  private halted = false;

  constructor(limits: ValidationLimits) {
    this.limits = limits;
  }

  private issue(code: string, message: string): void {
    if (this.issues.length >= this.limits.maxIssues) {
      this.issues.push({
        code: "issue_limit_reached",
        path: renderPath(this.path),
        message: `stopped after ${this.limits.maxIssues} issues`,
      });
      this.halted = true;
      return;
    }
    this.issues.push({ code, path: renderPath(this.path), message });
  }

  visit(schema: AnySchema, value: unknown, depth: number): void {
    if (this.halted) {
      return;
    }
    this.work += 1;
    if (this.work > this.limits.maxWork) {
      this.issue(
        "resource_limit_exceeded",
        `validation work exceeded ${this.limits.maxWork} visited nodes`,
      );
      this.halted = true;
      return;
    }
    if (depth > this.limits.maxDepth) {
      this.issue(
        "recursion_limit_exceeded",
        `value nesting exceeded ${this.limits.maxDepth} levels`,
      );
      this.halted = true;
      return;
    }

    const shape = shapeOf(schema);
    switch (shape.kind) {
      case "primitive":
        this.visitPrimitive(shape, value);
        return;
      case "opt":
        this.visitOpt(shape, value, depth);
        return;
      case "vec":
        this.visitVec(shape, value, depth);
        return;
      case "blob":
        if (!(value instanceof Uint8Array)) {
          this.issue(
            "expected_blob",
            `expected Uint8Array, got ${describe(value)}`,
          );
        }
        return;
      case "unit":
        this.visitUnit(value);
        return;
      case "record":
        this.visitRecord(shape, value, depth);
        return;
      case "tuple":
        this.visitTuple(shape, value, depth);
        return;
      case "variant":
        this.visitVariant(shape, value, depth);
        return;
      case "rec":
        if (typeof shape.body !== "function") {
          this.issue("unsupported_schema", "rec schema without a body thunk");
          return;
        }
        // The thunk is one indirection, not one value level: depth tracks the
        // *value*, and unbounded rec chains are cut off by maxWork.
        this.work += 1;
        this.visit(shape.body(), value, depth);
        return;
      default:
        this.issue(
          "unsupported_schema",
          `unknown schema kind \`${shape.kind}\` — failing closed`,
        );
        return;
    }
  }

  private visitPrimitive(shape: SchemaShape, value: unknown): void {
    const primitive = shape.primitive ?? "";
    switch (primitive) {
      case "null":
        if (value !== null) {
          this.issue("expected_null", `expected null, got ${describe(value)}`);
        }
        return;
      case "bool":
        if (typeof value !== "boolean") {
          this.issue(
            "expected_bool",
            `expected boolean, got ${describe(value)}`,
          );
        }
        return;
      case "text":
        if (typeof value !== "string") {
          this.issue("expected_text", `expected string, got ${describe(value)}`);
        }
        return;
      case "float32":
      case "float64":
        // Candid floats include NaN and the infinities; every JS number is a
        // valid float64 and representable-as-f32 is not checked (the value
        // representation is f64 either way).
        if (typeof value !== "number") {
          this.issue(
            "expected_number",
            `expected number for ${primitive}, got ${describe(value)}`,
          );
        }
        return;
      case "nat":
      case "int":
      case "nat64":
      case "int64":
        this.visitBigint(primitive, value);
        return;
      case "nat8":
      case "nat16":
      case "nat32":
      case "int8":
      case "int16":
      case "int32":
        this.visitFixedNumber(primitive, value);
        return;
      case "reserved":
        // Accepts anything, asserts nothing.
        return;
      case "empty":
        this.issue(
          "expected_empty",
          "`empty` is uninhabited; no value can satisfy it",
        );
        return;
      case "principal":
        if (!isPrincipalLike(value)) {
          this.issue(
            "expected_principal",
            `expected a Principal, got ${describe(value)}`,
          );
        }
        return;
      default:
        this.issue(
          "unsupported_schema",
          `unknown primitive \`${primitive}\` — failing closed`,
        );
        return;
    }
  }

  private visitBigint(primitive: string, value: unknown): void {
    if (typeof value !== "bigint") {
      // No coercion from number, by #102's recorded constraint: a number that
      // *looks* integral may already have lost precision.
      this.issue(
        "expected_bigint",
        `expected bigint for ${primitive}, got ${describe(value)}`,
      );
      return;
    }
    if (primitive === "nat" && value < 0n) {
      this.issue("bigint_out_of_range", "nat cannot be negative");
    } else if (primitive === "nat64" && (value < 0n || value > NAT64_MAX)) {
      this.issue("bigint_out_of_range", "nat64 out of [0, 2^64)");
    } else if (
      primitive === "int64" &&
      (value < INT64_MIN || value > INT64_MAX)
    ) {
      this.issue("bigint_out_of_range", "int64 out of [-2^63, 2^63)");
    }
  }

  private visitFixedNumber(primitive: string, value: unknown): void {
    if (typeof value !== "number") {
      this.issue(
        "expected_number",
        `expected number for ${primitive}, got ${describe(value)}`,
      );
      return;
    }
    if (!Number.isInteger(value)) {
      this.issue(
        "number_not_integer",
        `${primitive} requires an integer, got ${value}`,
      );
      return;
    }
    const range = NUMBER_RANGES[primitive];
    if (range === undefined) {
      this.issue("unsupported_schema", `no range for \`${primitive}\``);
      return;
    }
    const [low, high] = range;
    if (value < low || value > high) {
      this.issue(
        "int_out_of_range",
        `${primitive} out of [${low}, ${high}]: ${value}`,
      );
    }
  }

  private visitOpt(shape: SchemaShape, value: unknown, depth: number): void {
    const inner = shape.inner;
    if (inner === undefined) {
      this.issue("unsupported_schema", "opt schema without an inner schema");
      return;
    }
    // The collapsing-opt rule holds at runtime too: `T | null` cannot carry
    // `None` versus `Some(None)`, so an inner type that can itself be null
    // fails closed here even if construction-time checking was skipped.
    const innerShape = resolveShape(inner);
    if (
      innerShape === null ||
      innerShape.kind === "opt" ||
      (innerShape.kind === "primitive" &&
        (innerShape.primitive === "null" || innerShape.primitive === "reserved"))
    ) {
      this.issue(
        "unrepresentable_option",
        "opt of opt/null/reserved cannot be represented as `T | null`",
      );
      return;
    }
    if (value === null) {
      return;
    }
    this.visit(inner, value, depth + 1);
  }

  private visitVec(shape: SchemaShape, value: unknown, depth: number): void {
    const inner = shape.inner;
    if (inner === undefined) {
      this.issue("unsupported_schema", "vec schema without an inner schema");
      return;
    }
    if (!Array.isArray(value)) {
      this.issue("expected_array", `expected array, got ${describe(value)}`);
      return;
    }
    for (let index = 0; index < value.length; index += 1) {
      if (this.halted) {
        return;
      }
      this.path.push(index);
      this.visit(inner, value[index], depth + 1);
      this.path.pop();
    }
  }

  private visitUnit(value: unknown): void {
    if (
      typeof value !== "object" ||
      value === null ||
      Array.isArray(value) ||
      value instanceof Uint8Array
    ) {
      this.issue(
        "expected_unit",
        `expected the empty record {}, got ${describe(value)}`,
      );
      return;
    }
    const keys = Object.keys(value);
    if (keys.length > 0) {
      this.issue(
        "expected_unit",
        `the empty record has no fields; got key(s) ${keys
          .slice(0, 3)
          .map((key) => JSON.stringify(key))
          .join(", ")}`,
      );
    }
  }

  private visitRecord(shape: SchemaShape, value: unknown, depth: number): void {
    const fields = shape.fields;
    if (fields === undefined) {
      this.issue("unsupported_schema", "record schema without fields");
      return;
    }
    if (
      typeof value !== "object" ||
      value === null ||
      Array.isArray(value) ||
      value instanceof Uint8Array
    ) {
      this.issue("expected_record", `expected object, got ${describe(value)}`);
      return;
    }
    const record = value as Record<string, unknown>;
    // Schema key order is the canonical field order the generator emitted, so
    // issue ordering is deterministic.
    for (const key of Object.keys(fields)) {
      if (this.halted) {
        return;
      }
      const fieldSchema = fields[key];
      if (fieldSchema === undefined) {
        continue;
      }
      if (!Object.prototype.hasOwnProperty.call(record, key)) {
        this.path.push(key);
        this.issue("missing_field", "required field is absent");
        this.path.pop();
        continue;
      }
      this.path.push(key);
      this.visit(fieldSchema, record[key], depth + 1);
      this.path.pop();
    }
    for (const key of Object.keys(record)) {
      if (this.halted) {
        return;
      }
      if (!Object.prototype.hasOwnProperty.call(fields, key)) {
        this.path.push(key);
        this.issue("unexpected_field", "field is not part of the record type");
        this.path.pop();
      }
    }
  }

  private visitTuple(shape: SchemaShape, value: unknown, depth: number): void {
    const elements = shape.elements;
    if (elements === undefined) {
      this.issue("unsupported_schema", "tuple schema without elements");
      return;
    }
    if (!Array.isArray(value)) {
      this.issue("expected_tuple", `expected array, got ${describe(value)}`);
      return;
    }
    if (value.length !== elements.length) {
      this.issue(
        "tuple_length_mismatch",
        `expected ${elements.length} element(s), got ${value.length}`,
      );
      return;
    }
    for (let index = 0; index < elements.length; index += 1) {
      if (this.halted) {
        return;
      }
      const element = elements[index];
      if (element === undefined) {
        continue;
      }
      this.path.push(index);
      this.visit(element, value[index], depth + 1);
      this.path.pop();
    }
  }

  private visitVariant(shape: SchemaShape, value: unknown, depth: number): void {
    const arms = shape.arms;
    if (arms === undefined) {
      this.issue("unsupported_schema", "variant schema without arms");
      return;
    }
    if (
      typeof value !== "object" ||
      value === null ||
      Array.isArray(value) ||
      value instanceof Uint8Array
    ) {
      this.issue(
        "expected_variant",
        `expected { tag } or { tag, value }, got ${describe(value)}`,
      );
      return;
    }
    const candidate = value as Record<string, unknown>;
    const tag = candidate["tag"];
    if (typeof tag !== "string") {
      this.issue("expected_variant", "variant value has no string `tag`");
      return;
    }
    const keys = Object.keys(candidate);
    const hasValue = Object.prototype.hasOwnProperty.call(candidate, "value");
    const extraKeys = keys.filter((key) => key !== "tag" && key !== "value");
    if (extraKeys.length > 0) {
      this.issue(
        "expected_variant",
        `variant value carries extra key(s) ${extraKeys
          .slice(0, 3)
          .map((key) => JSON.stringify(key))
          .join(", ")}`,
      );
      return;
    }
    // Own-property lookup: a tag of "constructor" or "__proto__" must miss,
    // not resolve through Object.prototype.
    const arm = ownEntry(arms, tag);
    if (arm === undefined) {
      this.issue("unknown_variant_tag", `no arm named ${JSON.stringify(tag)}`);
      return;
    }
    const armShape = resolveShape(arm);
    if (armShape === null) {
      this.issue("unsupported_schema", "variant arm schema does not resolve");
      return;
    }
    if (isNullDomain(armShape)) {
      // The arm's only inhabitant is `null`, so the type collapses it to a
      // tag-only `{ tag }` — `null` (`{ tag, value: null }` is a type error)
      // and `opt` of an uninhabited type (`opt empty`, whose sole value is
      // `None`) both land here, and the runtime agrees: no payload.
      if (hasValue) {
        this.path.push("value");
        this.issue(
          "variant_payload_unexpected",
          `arm ${JSON.stringify(tag)} carries no payload`,
        );
        this.path.pop();
      }
      return;
    }
    if (isUninhabited(armShape)) {
      // Type-level, `[never] extends [null]` puts a bare `empty` arm in the
      // tag-only branch — but selecting an uninhabited arm can never be a
      // valid Candid value, so the validator rejects it rather than mirroring
      // the type-level artifact. (This is the one variant case where the
      // runtime is deliberately stricter than the inferred type.)
      this.issue(
        "variant_arm_uninhabited",
        `arm ${JSON.stringify(tag)} has an uninhabited payload and cannot be selected`,
      );
      return;
    }
    if (!hasValue) {
      this.path.push("value");
      this.issue(
        "variant_payload_missing",
        `arm ${JSON.stringify(tag)} requires a payload`,
      );
      this.path.pop();
      return;
    }
    this.path.push("value");
    this.visit(arm, candidate["value"], depth + 1);
    this.path.pop();
  }
}

/** A schema no value inhabits: bare `empty`, or a variant with zero arms. */
function isUninhabited(shape: SchemaShape): boolean {
  if (shape.kind === "primitive") {
    return shape.primitive === "empty";
  }
  if (shape.kind === "variant") {
    return shape.arms !== undefined && Object.keys(shape.arms).length === 0;
  }
  return false;
}

/**
 * A schema whose only domain value is `null`: the `null` primitive, or an
 * `opt` of an uninhabited type (`opt empty` can only ever be `None`). These
 * are exactly the variant arms whose `Infer` collapses to `null`, so the
 * runtime's tag-only treatment matches the generated type.
 */
function isNullDomain(shape: SchemaShape): boolean {
  if (shape.kind === "primitive") {
    return shape.primitive === "null";
  }
  if (shape.kind === "opt" && shape.inner !== undefined) {
    const inner = resolveShape(shape.inner);
    return inner !== null && isUninhabited(inner);
  }
  return false;
}

export function validate<T>(
  schema: Schema<T>,
  value: unknown,
  limits: ValidationLimits = DEFAULT_VALIDATION_LIMITS,
): ValidationResult {
  const walker = new Walker(limits);
  walker.visit(schema, value, 0);
  return walker.issues.length === 0
    ? { ok: true }
    : { ok: false, issues: walker.issues };
}

export interface SchemaCheckIssue {
  readonly code: string;
  /** Slash-joined structural path through the schema graph. */
  readonly path: string;
}

/**
 * Construction-time schema graph check: unknown kinds and collapsing options
 * (`opt` of `opt`/`null`/`reserved`, aliases included) fail closed before the
 * app ever serves a request, mirroring the generator's
 * `UnrepresentableOption` rule. Cycle-safe: each schema object is visited
 * once by identity, which terminates on generated graphs because recursion
 * always flows through the shared per-declaration `rec` constant. A straight
 * `rec` chain of distinct nodes deeper than the codec can resolve
 * (`MAX_REC_DEPTH`) is refused here rather than passing construction and then
 * failing on every wire call.
 */
export function checkSchema(schema: AnySchema): SchemaCheckIssue[] {
  const issues: SchemaCheckIssue[] = [];
  const visited = new Set<object>();

  const visit = (node: AnySchema, path: string, recRun: number): void => {
    if (visited.has(node)) {
      return;
    }
    visited.add(node);
    const shape = shapeOf(node);
    switch (shape.kind) {
      case "primitive":
        if (
          shape.primitive === undefined ||
          !isKnownPrimitive(shape.primitive)
        ) {
          issues.push({ code: "unsupported_schema", path });
        }
        return;
      case "blob":
      case "unit":
        return;
      case "opt": {
        if (shape.inner === undefined) {
          issues.push({ code: "unsupported_schema", path });
          return;
        }
        const inner = resolveShape(shape.inner);
        if (
          inner === null ||
          inner.kind === "opt" ||
          (inner.kind === "primitive" &&
            (inner.primitive === "null" || inner.primitive === "reserved"))
        ) {
          issues.push({ code: "unrepresentable_option", path });
          return;
        }
        // A structural node breaks the consecutive-`rec` chain: reset.
        visit(shape.inner, `${path}/opt`, 0);
        return;
      }
      case "vec":
        if (shape.inner === undefined) {
          issues.push({ code: "unsupported_schema", path });
          return;
        }
        visit(shape.inner, `${path}/vec`, 0);
        return;
      case "record":
      case "variant": {
        const table = shape.kind === "record" ? shape.fields : shape.arms;
        if (table === undefined) {
          issues.push({ code: "unsupported_schema", path });
          return;
        }
        for (const key of Object.keys(table)) {
          const child = table[key];
          if (child !== undefined) {
            visit(child, `${path}/${shape.kind}.${key}`, 0);
          }
        }
        return;
      }
      case "tuple":
        if (shape.elements === undefined) {
          issues.push({ code: "unsupported_schema", path });
          return;
        }
        shape.elements.forEach((element, index) => {
          visit(element, `${path}/tuple[${index}]`, 0);
        });
        return;
      case "rec":
        if (typeof shape.body !== "function") {
          issues.push({ code: "unsupported_schema", path });
          return;
        }
        if (recRun >= MAX_REC_DEPTH) {
          issues.push({ code: "rec_chain_too_deep", path });
          return;
        }
        visit(shape.body(), `${path}/rec`, recRun + 1);
        return;
      default:
        issues.push({ code: "unsupported_schema", path });
        return;
    }
  };

  visit(schema, "schema", 0);
  return issues;
}

function isKnownPrimitive(name: string): boolean {
  return PRIMITIVE_KINDS.has(name);
}
