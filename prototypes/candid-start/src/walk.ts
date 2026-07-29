// A structural view over schema-core objects. The core exports only `c`,
// `Schema`, and `Infer` on purpose; every walker in this package reads the
// runtime representation through this one file, so a representation change in
// the core has exactly one place to break. The coupling is intra-repository
// and versioned together — this is the same trade the goldens make.
import type { Schema } from "./schema.ts";

// `any` for the same reason the schema core uses it: `Schema<in out T>` is
// invariant, so `Schema<unknown>` would reject every concrete schema. The
// wildcard never leaks into an inferred output type.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnySchema = Schema<any>;

export interface SchemaShape {
  readonly kind: string;
  readonly primitive?: string;
  readonly inner?: AnySchema;
  readonly fields?: Record<string, AnySchema>;
  readonly elements?: readonly AnySchema[];
  readonly arms?: Record<string, AnySchema>;
  readonly body?: () => AnySchema;
}

export function shapeOf(schema: AnySchema): SchemaShape {
  return schema as unknown as SchemaShape;
}

/**
 * Resolve through `rec` indirections to the underlying structural node.
 * Bounded: a `rec` chain longer than the budget returns `null` rather than
 * recursing forever on a pathological hand-built schema. Generated schemas
 * are exactly one `rec` deep.
 */
export function resolveShape(
  schema: AnySchema,
  budget: number = 32,
): SchemaShape | null {
  let shape = shapeOf(schema);
  let remaining = budget;
  while (shape.kind === "rec") {
    if (remaining <= 0 || typeof shape.body !== "function") {
      return null;
    }
    remaining -= 1;
    shape = shapeOf(shape.body());
  }
  return shape;
}

/** Own-property lookup that cannot be confused by the prototype chain. */
export function ownEntry<T>(
  table: Record<string, T>,
  key: string,
): T | undefined {
  return Object.prototype.hasOwnProperty.call(table, key)
    ? table[key]
    : undefined;
}

export const PRIMITIVE_KINDS: ReadonlySet<string> = new Set([
  "null",
  "bool",
  "nat",
  "int",
  "nat8",
  "nat16",
  "nat32",
  "nat64",
  "int8",
  "int16",
  "int32",
  "int64",
  "float32",
  "float64",
  "text",
  "reserved",
  "empty",
  "principal",
]);
