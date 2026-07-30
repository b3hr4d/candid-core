// The minimal schema core the generated builders target — the seed of the
// Zod-style runtime recorded on issue #38. Combinators carry the structure a
// validator, codec, and form generator walk; this file defines shape and
// static inference, and `validate.ts` (issue #102) is the first walker. The
// node interfaces are exported for exactly that purpose: a walker narrows on
// `kind` instead of casting blindly, and the compiler checks the narrowing.
//
// `Schema<in out T>` is deliberately invariant. The generator annotates every
// declaration as `export const X: Schema<X> = …`, so the TypeScript compiler
// itself proves, on every golden, that the builder's inferred type is exactly
// the reviewed alias — assignable in both directions, not merely compatible in
// one. A mapping regression in either the emitter or this core turns the
// golden type-check red.
//
// This file lives inside the crate's type-check harness for now, imported as
// `@candid-core/schema` via tsconfig paths. Extracting it into a published npm
// package is its own future slice: npm names are as permanent as crates.io
// names, and the name is not yet decided.

import type { Principal } from "@icp-sdk/core/principal";

declare const phantom: unique symbol;

export interface Schema<in out T> {
  readonly kind: string;
  /** Phantom carrier for `T`; never present at runtime. */
  readonly [phantom]?: (value: T) => T;
}

/** The static type a schema describes: `Infer<typeof X>`. */
export type Infer<S> = S extends Schema<infer T> ? T : never;

export interface PrimitiveSchema<T> extends Schema<T> {
  readonly kind: "primitive";
  readonly primitive: string;
}

export interface OptSchema<T> extends Schema<T | null> {
  readonly kind: "opt";
  readonly inner: Schema<T>;
}

export interface VecSchema<T> extends Schema<T[]> {
  readonly kind: "vec";
  readonly inner: Schema<T>;
}

export interface BlobSchema extends Schema<Uint8Array> {
  readonly kind: "blob";
}

export interface UnitSchema extends Schema<Record<string, never>> {
  readonly kind: "unit";
}

// `any`, deliberately: `Schema<T>` is invariant, so a `Schema<unknown>` bound
// would reject every concrete schema. `any` is the variance wildcard in
// constraint position and never leaks into inferred output types.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnySchema = Schema<any>;
export type FieldSchemas = Record<string, AnySchema>;

type RecordInfer<F extends FieldSchemas> = { [K in keyof F]: Infer<F[K]> };

export interface RecordSchema<F extends FieldSchemas> extends Schema<RecordInfer<F>> {
  readonly kind: "record";
  readonly fields: F;
}

type TupleInfer<S extends readonly AnySchema[]> = {
  -readonly [I in keyof S]: Infer<S[I]>;
};

export interface TupleSchema<S extends readonly AnySchema[]>
  extends Schema<TupleInfer<S>> {
  readonly kind: "tuple";
  readonly elements: S;
}

// An arm with a `null` payload is a bare tag: Candid's `ok` and `ok : null`
// are the same variant arm, and `value: null` on every tag-only arm would be
// noise. The `[…] extends [null]` guard keeps union payloads out of the
// tag-only branch.
type VariantInfer<A extends FieldSchemas> = {
  [K in keyof A & string]: [Infer<A[K]>] extends [null]
    ? { tag: K }
    : { tag: K; value: Infer<A[K]> };
}[keyof A & string];

export interface VariantSchema<A extends FieldSchemas>
  extends Schema<VariantInfer<A>> {
  readonly kind: "variant";
  readonly arms: A;
}

export interface RecSchema<T> extends Schema<T> {
  readonly kind: "rec";
  readonly body: () => Schema<T>;
}

/**
 * A Candid `func` *value*: a reference to a method on some service. The
 * modern domain shape recorded on issue #104, mirroring candid-core's
 * `HostValueKind::Func { principal, method }`. Invoking one is the actor
 * layer's job (`callFunc` in `actor.ts`) — the value itself stays inert
 * data, so it round-trips validation and the codec symmetrically.
 */
export interface FuncValue {
  readonly principal: Principal;
  readonly method: string;
}

/** Candid method modes; `update` is the unannotated default made explicit. */
export type MethodMode = "update" | "query" | "composite_query" | "oneway";

/**
 * The signature lives in the node — args, results, mode — while the *value*
 * type is always [`FuncValue`]: what a func-typed field carries at runtime
 * is a reference, not a closure. The actor layer reads the node
 * structurally, which is why no generic parameters are needed here (and why
 * none would survive `c.rec`'s type erasure anyway).
 */
export interface FuncSchema extends Schema<FuncValue> {
  readonly kind: "func";
  readonly args: readonly AnySchema[];
  readonly results: readonly AnySchema[];
  readonly mode: MethodMode;
}

/**
 * A Candid `service` *value* is the principal of a running service (issue
 * #104); the method map is what `createActor` walks to build a call surface.
 */
export interface ServiceSchema extends Schema<Principal> {
  readonly kind: "service";
  /**
   * `AnySchema`, not `FuncSchema`: the dynamic loader wraps every method in
   * a lazy `rec` thunk (cyclic services exist), so walkers resolve each
   * method to its func node structurally, exactly as they do everywhere
   * else. Generated builders still pass `c.func(...)` directly.
   */
  readonly methods: { readonly [name: string]: AnySchema };
}

function primitive<T>(name: string): PrimitiveSchema<T> {
  return { kind: "primitive", primitive: name };
}

export const c = {
  null: primitive<null>("null"),
  bool: primitive<boolean>("bool"),
  nat: primitive<bigint>("nat"),
  int: primitive<bigint>("int"),
  nat8: primitive<number>("nat8"),
  nat16: primitive<number>("nat16"),
  nat32: primitive<number>("nat32"),
  nat64: primitive<bigint>("nat64"),
  int8: primitive<number>("int8"),
  int16: primitive<number>("int16"),
  int32: primitive<number>("int32"),
  int64: primitive<bigint>("int64"),
  float32: primitive<number>("float32"),
  float64: primitive<number>("float64"),
  text: primitive<string>("text"),
  reserved: primitive<unknown>("reserved"),
  empty: primitive<never>("empty"),
  principal: primitive<Principal>("principal"),

  opt<T>(inner: Schema<T>): OptSchema<T> {
    return { kind: "opt", inner };
  },

  vec<T>(inner: Schema<T>): VecSchema<T> {
    return { kind: "vec", inner };
  },

  blob(): BlobSchema {
    return { kind: "blob" };
  },

  /** The empty Candid record — a unit value, not "any non-nullish thing". */
  unit(): UnitSchema {
    return { kind: "unit" };
  },

  record<F extends FieldSchemas>(fields: F): RecordSchema<F> {
    return { kind: "record", fields };
  },

  tuple<const S extends readonly AnySchema[]>(
    elements: S,
  ): TupleSchema<S> {
    return { kind: "tuple", elements };
  },

  variant<A extends FieldSchemas>(arms: A): VariantSchema<A> {
    return { kind: "variant", arms };
  },

  func(
    args: readonly AnySchema[],
    results: readonly AnySchema[],
    mode: MethodMode,
  ): FuncSchema {
    return { kind: "func", args, results, mode };
  },

  service(methods: { readonly [name: string]: AnySchema }): ServiceSchema {
    return { kind: "service", methods };
  },

  /**
   * Lazy indirection. The generator wraps every declaration in `rec`, so
   * canonical (name-sorted) emission order can never hit a temporal-dead-zone
   * reference, and recursion needs no special casing at emission time.
   */
  rec<T>(body: () => Schema<T>): RecSchema<T> {
    return { kind: "rec", body };
  },
};
