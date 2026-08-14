// The minimal schema core the generated builders target — the seed of the
// Zod-style runtime recorded on issue #38. Combinators carry the structure a
// validator, codec, and form generator walk; this file defines shape and
// static inference, and `validate.ts` (issue #102) is the first walker. The
// node interfaces are exported for exactly that purpose: a walker narrows on
// `kind` instead of casting blindly, and the compiler checks the narrowing.
// `SchemaNode`, `resolveSchema`, and `serviceMethods` at the foot of the file
// are the rest of that contract — the rec-chain discipline and the service
// method table every walker needs, stated once instead of re-derived per
// consumer (issue #149).
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

declare const phantom: unique symbol;

/**
 * The principal value this runtime describes, delivers, and accepts — the
 * decoded-value contract, stated once:
 *
 * - **Decoded values carry exactly this surface.** The codec is
 *   self-contained by design (no runtime dependency on any Principal
 *   implementation), so decoding produces a minimal structural carrier of
 *   the canonical text — an object whose only member is `toText()`. It is
 *   *not* an `@icp-sdk/core` `Principal` instance: `instanceof` is `false`
 *   and class methods like `toUint8Array()` do not exist on it. Code that
 *   needs the class re-wraps: `Principal.fromText(value.toText())`.
 * - **SDK `Principal` instances are accepted on input.** Validation and
 *   encoding are structural — any object with a `toText()` method whose
 *   text is a canonical principal — and the SDK class satisfies that
 *   shape, so values you built with `Principal.fromText(…)` encode
 *   unchanged.
 *
 * @example
 * const owner: PrincipalValue = { toText: () => "ryjl3-tyaaa-aaaaa-aaaba-cai" };
 */
export interface PrincipalValue {
  toText(): string;
}

/**
 * The node every combinator produces: a `kind` tag a walker dispatches on,
 * plus a phantom carrier for the domain type `T`. `T` exists only in the
 * type system — a schema is plain inert data at runtime.
 *
 * Deliberately **invariant** (`in out T`). Generated modules annotate every
 * declaration as `export const X: Schema<X> = …`, so the compiler itself
 * proves the builder's inferred type is exactly the reviewed alias:
 * assignable in both directions, not merely compatible in one.
 *
 * @example
 * const Balance: Schema<bigint> = c.nat;
 */
export interface Schema<in out T> {
  readonly kind: string;
  /** Phantom carrier for `T`; never present at runtime. */
  readonly [phantom]?: (value: T) => T;
}

/**
 * The static type a schema describes — how a consumer names a generated
 * type without hand-writing it.
 *
 * @example
 * const Account = c.record({ owner: c.principal, balance: c.nat });
 * type Account = Infer<typeof Account>; // { owner: PrincipalValue; balance: bigint }
 */
export type Infer<S> = S extends Schema<infer T> ? T : never;

/**
 * A leaf node. `primitive` names the Candid type (`"nat"`, `"text"`, …) and
 * `T` is the JavaScript domain it describes.
 *
 * @example
 * c.nat64.primitive; // "nat64"
 */
export interface PrimitiveSchema<T> extends Schema<T> {
  readonly kind: "primitive";
  readonly primitive: string;
}

/**
 * `opt T`, whose domain is `T | null`. Absence is exactly `null`;
 * `undefined` is not a Candid value and is checked against `inner`.
 *
 * @example
 * c.opt(c.text).inner; // the `text` schema underneath
 */
export interface OptSchema<T> extends Schema<T | null> {
  readonly kind: "opt";
  readonly inner: Schema<T>;
}

/**
 * `vec T`, whose domain is `T[]`. Validation requires a real array — an
 * array-like or a bare iterable is not one.
 *
 * @example
 * c.vec(c.nat).inner; // the element schema
 */
export interface VecSchema<T> extends Schema<T[]> {
  readonly kind: "vec";
  readonly inner: Schema<T>;
}

/**
 * The anonymous `vec nat8`, whose domain is `Uint8Array` rather than
 * `number[]`. A distinct node because the byte path is worth taking
 * wholesale rather than one element at a time.
 *
 * @example
 * c.blob().kind; // "blob"
 */
export interface BlobSchema extends Schema<Uint8Array> {
  readonly kind: "blob";
}

/**
 * The empty Candid record, whose domain is `Record<string, never>` — a unit
 * value with no fields, not "any non-nullish thing".
 *
 * @example
 * c.unit().kind; // "unit"
 */
export interface UnitSchema extends Schema<Record<string, never>> {
  readonly kind: "unit";
}

/**
 * A schema of *some* domain type, erased — the currency of code that holds
 * schemas without knowing what they describe, such as the runtime loader.
 *
 * `any`, deliberately: [`Schema`] is invariant, so a `Schema<unknown>` bound
 * would reject every concrete schema. It is the variance wildcard in
 * constraint position and never leaks into an inferred output type.
 *
 * @example
 * const byName: Record<string, AnySchema> = { Account: c.record({ id: c.nat }) };
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnySchema = Schema<any>;

/**
 * What a composite accepts as a child schema: [`AnySchema`] widened to
 * re-admit `Schema<never>`. `any` is assignable to every type but `never`,
 * so the invariant phantom would otherwise reject every schema whose domain
 * is `never` — the `empty` leaf, an empty variant, and anything resolving to
 * one.
 *
 * The widening lives here rather than in [`Schema`] itself, so `Infer` still
 * distributes to the right member and the generated-alias equality proof
 * keeps its full strength.
 *
 * @example
 * const args: readonly AnyFieldSchema[] = [c.text, c.empty];
 */
// The `@ts-expect-error` probes in `tests/schema-types.test.ts` pin both
// halves of that claim (issue #126): `Schema<never>` really is rejected by
// `AnySchema`, and the widening really does not reach `Schema<in out T>`.
export type AnyFieldSchema = AnySchema | Schema<never>;

/**
 * The field or arm map a `record` or `variant` is built from: schema key to
 * child schema.
 *
 * @example
 * const fields: FieldSchemas = { owner: c.principal, balance: c.nat };
 */
export type FieldSchemas = Record<string, AnyFieldSchema>;

type RecordInfer<F extends FieldSchemas> = { [K in keyof F]: Infer<F[K]> };

/**
 * A Candid `record`, whose domain is an object with one property per field.
 *
 * @example
 * c.record({ balance: c.nat }).fields.balance; // the field's own schema
 */
export interface RecordSchema<F extends FieldSchemas> extends Schema<RecordInfer<F>> {
  readonly kind: "record";
  readonly fields: F;
}

type TupleInfer<S extends readonly AnyFieldSchema[]> = {
  -readonly [I in keyof S]: Infer<S[I]>;
};

/**
 * A Candid record whose field ids are exactly `0..n-1`, whose domain is a
 * fixed-length JavaScript tuple rather than an object.
 *
 * @example
 * c.tuple([c.text, c.nat]).elements.length; // 2
 */
export interface TupleSchema<S extends readonly AnyFieldSchema[]> extends Schema<TupleInfer<S>> {
  readonly kind: "tuple";
  readonly elements: S;
}

// An arm with a `null` payload is a bare tag: Candid's `ok` and `ok : null`
// are the same variant arm, and `value: null` on every tag-only arm would be
// noise. Classification matches the emitter, validator, and codec, which all
// test the payload *node* (issue #127), in order:
// - a payload whose domain is `never` (`empty`, an empty variant) carries
//   `value` — its node is not `null`, so the runtime demands the field even
//   though nothing can inhabit it;
// - an `opt`-kind payload always carries `value` — `opt empty` infers
//   `null` without being the null primitive;
// - everything else is a bare tag exactly when its domain is `null`: the
//   `null` primitive, and a declared alias of `null`, whose generated
//   reference is annotated `Schema<null>` and whose node the runtime
//   resolves back to the null primitive.
// The one shape these rules cannot see through — a *declared* alias of
// `opt empty`, statically identical to a declared alias of `null` — is
// refused at generation (`TsGenError::AmbiguousVariantArm`); a hand-built
// `c.rec` thunk over any null-domain opt (`opt empty`, `opt null`) passed
// directly as an arm is likewise statically invisible (rec erases
// structure) and stays a documented limitation, classified correctly by
// the runtime either way.
type VariantInfer<A extends FieldSchemas> = {
  [K in keyof A & string]: [Infer<A[K]>] extends [never]
    ? { tag: K; value: Infer<A[K]> }
    : // The kind literal, not `OptSchema<any>`: the invariant inner makes
      // `OptSchema<never>` fail an `OptSchema<any>` test — the very #126
      // variance corner — while the kind is what the runtime itself tests.
      A[K] extends { kind: "opt" }
      ? { tag: K; value: Infer<A[K]> }
      : [Infer<A[K]>] extends [null]
        ? { tag: K }
        : { tag: K; value: Infer<A[K]> };
}[keyof A & string];

/**
 * A Candid `variant`, whose domain is a discriminated union on `tag`. An arm
 * with a `null` payload is a bare `{ tag }`; every other arm carries
 * `{ tag, value }`.
 *
 * @example
 * c.variant({ ok: c.nat, err: c.text }).arms.ok; // the `ok` payload schema
 */
export interface VariantSchema<A extends FieldSchemas> extends Schema<VariantInfer<A>> {
  readonly kind: "variant";
  readonly arms: A;
}

/**
 * A lazy indirection around another schema: `body` is a thunk resolved on
 * demand, which is what lets a schema refer to itself. The domain type is
 * the body's, unchanged — `rec` adds a hop, not a shape.
 *
 * @example
 * c.rec(() => c.nat).body(); // resolves the thunk to the `nat` schema
 */
export interface RecSchema<T> extends Schema<T> {
  readonly kind: "rec";
  readonly body: () => Schema<T>;
}

/**
 * A Candid `func` *value*: a reference to a method on some service, as the
 * `{ principal, method }` pair candid-core's value domain uses. Invoking one
 * is the actor layer's job (`callFunc` in `@candid-core/schema/actor`) — the
 * value itself stays inert data, so it round-trips validation and the codec
 * symmetrically. Validation is strict: both fields required, no other own
 * enumerable key, and the method name a non-empty string.
 *
 * @example
 * const nextPage: FuncValue = { principal: archive, method: "get_blocks" };
 */
export interface FuncValue {
  readonly principal: PrincipalValue;
  readonly method: string;
}

/**
 * Candid method modes; `update` is the unannotated default made explicit. An
 * actor sends `query` and `composite_query` down its transport's
 * non-replicated read path, and `update` and `oneway` down the replicated
 * one.
 *
 * @example
 * const mode: MethodMode = "composite_query";
 */
export type MethodMode = "update" | "query" | "composite_query" | "oneway";

/**
 * The signature lives in the node — args, results, mode — while the *value*
 * type is always [`FuncValue`]: what a func-typed field carries at runtime
 * is a reference, not a closure. The actor layer reads the node
 * structurally, which is why no generic parameters are needed here (and why
 * none would survive `c.rec`'s type erasure anyway).
 *
 * @example
 * c.func([c.principal], [c.nat], "query").mode; // "query"
 */
export interface FuncSchema extends Schema<FuncValue> {
  readonly kind: "func";
  readonly args: readonly AnyFieldSchema[];
  readonly results: readonly AnyFieldSchema[];
  readonly mode: MethodMode;
}

/**
 * A Candid `service` *type*. The value it describes is the principal of a
 * running service; the method map is what `createActor` walks to build a
 * typed call surface.
 *
 * @example
 * c.service({ balance: c.func([], [c.nat], "query") }).methods.balance;
 */
export interface ServiceSchema extends Schema<PrincipalValue> {
  readonly kind: "service";
  /**
   * `AnySchema`, not `FuncSchema`: the runtime loader wraps every method in
   * a lazy `rec` thunk (cyclic services exist), so walkers resolve each
   * method to its func node structurally, exactly as they do everywhere
   * else. Generated builders still pass `c.func(...)` directly.
   */
  readonly methods: { readonly [name: string]: AnySchema };
}

/* eslint-disable @typescript-eslint/no-explicit-any */
/**
 * Every node a schema can be, as one discriminated union on `kind` — what a
 * walker narrows over once it holds a schema whose domain type is erased.
 * A `switch` over this union with a `never`-typed default is exhaustive by
 * construction, so a combinator added here turns every such walk red until it
 * is handled.
 *
 * Every member's domain type is erased, composites included. [`Schema`] is
 * invariant, so a `RecordSchema` of *specific* fields is not assignable to
 * one of the general field map — the erasure is what lets a concrete builder
 * result be handed to a walker at all. It leaves this union exactly where
 * [`AnySchema`] sits: a `Schema<never>` — the `empty` leaf, an empty variant
 * — is not one of these erased members, which is why [`resolveSchema`] takes
 * the widened [`AnyFieldSchema`] and returns a node rather than demanding one.
 *
 * @example
 * const node: SchemaNode = c.record({ balance: c.nat });
 * if (node.kind === "record") node.fields; // narrowed to the field map
 */
export type SchemaNode =
  | PrimitiveSchema<any>
  | OptSchema<any>
  | VecSchema<any>
  | BlobSchema
  | UnitSchema
  | RecordSchema<any>
  | TupleSchema<any>
  | VariantSchema<any>
  | FuncSchema
  | ServiceSchema
  | RecSchema<any>;

/**
 * A [`SchemaNode`] that is not a `rec` — what [`resolveSchema`] returns. The
 * `rec` case is gone from the union rather than left unreachable, so a walk
 * over a resolved node stays exhaustive without a case that cannot happen.
 *
 * @example
 * const node: ResolvedNode = resolveSchema(c.rec(() => c.text));
 */
export type ResolvedNode = Exclude<SchemaNode, RecSchema<any>>;
/* eslint-enable @typescript-eslint/no-explicit-any */

/**
 * One method of a service: its name, its mode, and the schemas of its
 * arguments and results. The signature lives in the node rather than in the
 * type — `rec` erases it from the type — which is why introspection is how it
 * is read back.
 *
 * @example
 * const method: ServiceMethod = { name: "fee", mode: "query", args: [], results: [c.nat] };
 */
export interface ServiceMethod {
  readonly name: string;
  readonly mode: MethodMode;
  readonly args: readonly AnyFieldSchema[];
  readonly results: readonly AnyFieldSchema[];
}

function primitive<T>(name: string): PrimitiveSchema<T> {
  return { kind: "primitive", primitive: name };
}

/**
 * The schema builders. Every generated module targets this object, and it is
 * the whole construction surface: there is no class to instantiate and no
 * registry to populate — a schema is the plain data these return.
 *
 * @example
 * const Account = c.record({ owner: c.principal, balance: c.nat });
 */
export const c = {
  /**
   * Candid `null`, the single-valued type. The one accepted value is literal
   * `null`; `undefined` is not a Candid value. Mostly seen as a variant arm,
   * where a `null` payload is what makes the arm a bare tag.
   *
   * @example
   * c.variant({ ok: c.null }); // { tag: "ok" }
   */
  null: primitive<null>("null"),

  /**
   * Candid `bool`, a JavaScript `boolean`.
   *
   * @example
   * c.record({ active: c.bool }); // { active: boolean }
   */
  bool: primitive<boolean>("bool"),

  /**
   * Candid `nat`: an unbounded non-negative integer, always `bigint`. A
   * `number` is rejected rather than coerced — `nat` exceeds 2^53 on the
   * wire, and a lossy bridge would corrupt values silently.
   *
   * @example
   * const balance: Infer<typeof c.nat> = 12_345_678_901_234_567_890n;
   */
  nat: primitive<bigint>("nat"),

  /**
   * Candid `int`: an unbounded signed integer, always `bigint`, for the same
   * reason `nat` is.
   *
   * @example
   * const delta: Infer<typeof c.int> = -42n;
   */
  int: primitive<bigint>("int"),

  /**
   * Candid `nat8`: an integral `number` in `[0, 255]`. A fractional value
   * fails with `not_integer`, one outside the range with `out_of_range`.
   *
   * @example
   * const byte: Infer<typeof c.nat8> = 255;
   */
  nat8: primitive<number>("nat8"),

  /**
   * Candid `nat16`: an integral `number` in `[0, 65_535]`.
   *
   * @example
   * const port: Infer<typeof c.nat16> = 8080;
   */
  nat16: primitive<number>("nat16"),

  /**
   * Candid `nat32`: an integral `number` in `[0, 4_294_967_295]` — the
   * widest unsigned Candid width whose whole range a JavaScript number holds
   * exactly, which is why it is `number` and `nat64` is not.
   *
   * @example
   * const height: Infer<typeof c.nat32> = 4_294_967_295;
   */
  nat32: primitive<number>("nat32"),

  /**
   * Candid `nat64`: a `bigint` in `[0, 2^64-1]`. Past 2^53 a `number` cannot
   * hold every value, so this width is `bigint` and a `number` is rejected.
   *
   * @example
   * const e8s: Infer<typeof c.nat64> = 100_000_000n;
   */
  nat64: primitive<bigint>("nat64"),

  /**
   * Candid `int8`: an integral `number` in `[-128, 127]`.
   *
   * @example
   * const offset: Infer<typeof c.int8> = -128;
   */
  int8: primitive<number>("int8"),

  /**
   * Candid `int16`: an integral `number` in `[-32_768, 32_767]`.
   *
   * @example
   * const trim: Infer<typeof c.int16> = -32_768;
   */
  int16: primitive<number>("int16"),

  /**
   * Candid `int32`: an integral `number` in
   * `[-2_147_483_648, 2_147_483_647]`.
   *
   * @example
   * const drift: Infer<typeof c.int32> = -2_147_483_648;
   */
  int32: primitive<number>("int32"),

  /**
   * Candid `int64`: a `bigint` in `[-2^63, 2^63-1]`, `bigint` for the same
   * 53-bit reason `nat64` is.
   *
   * @example
   * const nanos: Infer<typeof c.int64> = -9_223_372_036_854_775_808n;
   */
  int64: primitive<bigint>("int64"),

  /**
   * Candid `float32`, a JavaScript `number`. Validation accepts any number —
   * NaN and the infinities are valid Candid floats — but *encoding* is
   * exact-or-refuse: a value that is not `Math.fround`-exact fails with
   * `unrepresentable_float32` rather than rounding behind your back, so a
   * decode of an encode is structural identity.
   *
   * @example
   * const ratio: Infer<typeof c.float32> = Math.fround(0.1);
   */
  float32: primitive<number>("float32"),

  /**
   * Candid `float64`, a JavaScript `number` — the same double, so every
   * number encodes exactly.
   *
   * @example
   * const rate: Infer<typeof c.float64> = 0.1;
   */
  float64: primitive<number>("float64"),

  /**
   * Candid `text`, a JavaScript `string`. Candid text is a sequence of
   * Unicode scalar values, so encoding refuses a lone surrogate with
   * `invalid_text` — a string that validation, which checks only the
   * JavaScript type, lets through.
   *
   * @example
   * c.record({ memo: c.text }); // { memo: string }
   */
  text: primitive<string>("text"),

  /**
   * Candid `reserved`: the type every value inhabits. Validation asserts
   * nothing about it, which is exactly why its domain type is `unknown` — a
   * consumer must narrow before use.
   *
   * @example
   * const anything: Infer<typeof c.reserved> = { whatever: true };
   */
  reserved: primitive<unknown>("reserved"),

  /**
   * Candid `empty`: the uninhabited type, domain `never`. No value passes —
   * every one fails with `uninhabited_type` — so it appears as the
   * impossible arm of a variant, never as a field a caller can fill.
   *
   * @example
   * c.variant({ ok: c.text, never: c.empty });
   */
  empty: primitive<never>("empty"),

  /**
   * Candid `principal`, typed as the structural [`PrincipalValue`] — the
   * type surface telling the truth about what the self-contained codec
   * delivers. Validation is structural (an object carrying a `toText`
   * method), encoding parses `toText()` and refuses non-canonical text, and
   * an `@icp-sdk/core` `Principal` instance satisfies the shape, so SDK
   * values encode unchanged. See [`PrincipalValue`] for the full contract.
   *
   * @example
   * c.record({ owner: c.principal }); // { owner: PrincipalValue }
   */
  principal: primitive<PrincipalValue>("principal"),

  /**
   * Candid `opt T`, rendered as `T | null`: absence is exactly `null`, and
   * the property stays *present* on its record rather than going missing.
   *
   * Because `T | null` cannot tell `null` apart from a nested absent value,
   * an inner type that can itself be null fails closed wherever schemas are
   * derived — the generator refuses such a declaration, and the runtime
   * loader refuses the document with `unrepresentable_option`.
   *
   * @example
   * c.record({ memo: c.opt(c.text) }); // { memo: string | null }
   */
  opt<T>(inner: Schema<T>): OptSchema<T> {
    return { kind: "opt", inner };
  },

  /**
   * Candid `vec T`, a JavaScript `T[]`. Validation wants a real array: an
   * array-like or a bare iterable fails with `invalid_type`.
   *
   * @example
   * c.vec(c.nat32); // number[]
   */
  vec<T>(inner: Schema<T>): VecSchema<T> {
    return { kind: "vec", inner };
  },

  /**
   * Candid `blob` — the anonymous `vec nat8` — as a `Uint8Array` rather than
   * a `number[]`. The check reads the typed-array brand, so a `Uint8Array`
   * from another realm (a Node `Buffer` included) passes while a prototype
   * forgery does not.
   *
   * @example
   * c.record({ payload: c.blob() }); // { payload: Uint8Array }
   */
  blob(): BlobSchema {
    return { kind: "blob" };
  },

  /**
   * The empty Candid record — a unit value, not "any non-nullish thing":
   * the domain is `Record<string, never>`, and an own enumerable string key
   * on the value is an `unexpected_field`. A non-enumerable or symbol-keyed
   * property is invisible to that scan, for the same reason it is to a
   * record's — what `JSON.stringify` and spread see is what validates.
   *
   * @example
   * c.record({ ack: c.unit() }); // { ack: Record<string, never> }
   */
  unit(): UnitSchema {
    return { kind: "unit" };
  },

  /**
   * Candid `record`, keyed by field name. Strict in both directions: a
   * missing key is `missing_field` and an unknown one `unexpected_field`,
   * `opt` fields included — the domain shape is `T | null` with the property
   * there. Presence means an *own enumerable* property, the projection
   * `JSON.stringify`, spread, and structured clone all see, so a value that
   * validates never serializes into one that would not.
   *
   * A key carries its Candid label: the name hashes to the wire id, and a
   * field the interface left unnamed keys by the ecosystem's `_id_`
   * rendering instead.
   *
   * @example
   * c.record({ owner: c.principal, balance: c.nat });
   */
  record<F extends FieldSchemas>(fields: F): RecordSchema<F> {
    return { kind: "record", fields };
  },

  /**
   * A Candid record whose field ids are exactly `0..n-1`, as a fixed-length
   * JavaScript tuple. A value of the wrong length fails with
   * `invalid_length`. The `const` type parameter is what keeps element order
   * and arity in the inferred type instead of widening them to an array.
   *
   * @example
   * c.tuple([c.text, c.nat]); // [string, bigint]
   */
  tuple<const S extends readonly AnyFieldSchema[]>(elements: S): TupleSchema<S> {
    return { kind: "tuple", elements };
  },

  /**
   * Candid `variant`, as a discriminated union on `tag`. An arm whose
   * payload is `null` is a bare `{ tag }` — Candid's `ok` and `ok : null`
   * are the same arm, so a `value: null` on every tag-only arm would be
   * noise; every other arm is `{ tag, value }`. A tag that is not an arm
   * fails with `unknown_tag`.
   *
   * @example
   * c.variant({ ok: c.nat, err: c.text, pending: c.null });
   * // { tag: "ok"; value: bigint } | { tag: "err"; value: string } | { tag: "pending" }
   */
  variant<A extends FieldSchemas>(arms: A): VariantSchema<A> {
    return { kind: "variant", arms };
  },

  /**
   * A Candid `func` *type*: the node carries the signature — argument
   * schemas, result schemas, mode — while the value it describes is always
   * an inert [`FuncValue`] reference, never a closure. `callFunc` in
   * `@candid-core/schema/actor` is what invokes one.
   *
   * @example
   * c.func([c.principal], [c.nat], "query");
   */
  func(
    args: readonly AnyFieldSchema[],
    results: readonly AnyFieldSchema[],
    mode: MethodMode,
  ): FuncSchema {
    return { kind: "func", args, results, mode };
  },

  /**
   * A Candid `service` *type*. The value it describes is the principal of a
   * running service; the method map is what `createActor` in
   * `@candid-core/schema/actor` walks to build a typed call surface.
   *
   * @example
   * c.service({ balance: c.func([c.principal], [c.nat], "query") });
   */
  service(methods: { readonly [name: string]: AnySchema }): ServiceSchema {
    return { kind: "service", methods };
  },

  /**
   * Lazy indirection — how a schema refers to itself. The generator wraps
   * every declaration in `rec`, so canonical (name-sorted) emission order
   * can never hit a temporal-dead-zone reference, and recursion needs no
   * special casing at emission time. Each hop costs one traversal step
   * against a walk's depth and element budgets, which is what makes a
   * mis-built self-referential chain terminate instead of hanging.
   *
   * The explicit `Schema<T>` annotation is not optional decoration: it is
   * what breaks the circular inference, and it is the equality the compiler
   * then proves about the builder.
   *
   * @example
   * type List = { head: bigint; tail: List | null };
   * const List: Schema<List> = c.rec(() => c.record({ head: c.nat, tail: c.opt(List) }));
   */
  rec<T>(body: () => Schema<T>): RecSchema<T> {
    return { kind: "rec", body };
  },
};

// Reading a schema back — issue #149. Both walkers in this package (the actor
// factory and the form-model builder) resolved rec chains privately before
// this, in two copies carrying the same bound and the same two messages; a
// consumer holding the exported node interfaces had to write a third against
// a discipline nothing documented. One implementation lives here now, and
// both walkers call it.
const REC_HOP_LIMIT = 256;

// The kinds `ResolvedNode` claims to cover, as a record rather than a bare
// list: a combinator added to the union and forgotten here is a compile
// error, so the runtime check and the return type cannot drift apart. The
// lookup goes through a Set because `"toString" in {}` answers true and a
// method name is not a schema kind.
const RESOLVED_KINDS: { readonly [K in ResolvedNode["kind"]]: true } = {
  primitive: true,
  opt: true,
  vec: true,
  blob: true,
  unit: true,
  record: true,
  tuple: true,
  variant: true,
  func: true,
  service: true,
};
const KNOWN_KINDS: ReadonlySet<string> = new Set(Object.keys(RESOLVED_KINDS));

/**
 * Resolve a schema to the node underneath its `rec` indirections: follow
 * `body()` until a non-`rec` node appears, then return that node.
 *
 * This is the walking discipline every schema consumer needs, and the reason
 * the node interfaces above are exported at all. Generated modules wrap
 * *every* declaration in `c.rec`, and building schemas from a Contract
 * document at runtime adds a hop at each edge, so a schema reached by name is
 * a `rec` far more often than not: reading `.kind` off one directly sees
 * `"rec"` and learns nothing about the type.
 *
 * Bounded and fail-closed, because a cycle is the shape recursion naturally
 * produces: a chain that never terminates throws `TypeError` after 256 hops
 * rather than hanging. So does anything that is not a schema object, and so
 * does a `kind` this package does not define — [`Schema`] requires only that
 * `kind` be *a* string, so the returned node is checked against the kinds
 * [`ResolvedNode`] actually covers rather than asserted to be one of them.
 * All three are programmer errors; a malformed *value* is what `validate`
 * reports on, and it never throws.
 *
 * @example
 * resolveSchema(c.rec(() => c.nat)).kind; // "primitive"
 */
export function resolveSchema(schema: AnyFieldSchema): ResolvedNode {
  let node: unknown = schema;
  for (let hops = 0; hops < REC_HOP_LIMIT; hops += 1) {
    if (
      typeof node !== "object" ||
      node === null ||
      typeof (node as { kind?: unknown }).kind !== "string"
    ) {
      throw new TypeError("not a schema object");
    }
    const kind = (node as { kind: string }).kind;
    if (kind !== "rec") {
      if (!KNOWN_KINDS.has(kind)) {
        throw new TypeError(`unknown schema kind ${JSON.stringify(kind)}`);
      }
      return node as ResolvedNode;
    }
    node = (node as { body(): unknown }).body();
  }
  throw new TypeError("rec chain exceeds the depth limit");
}

/**
 * The method table of a service schema, keyed by method name in declaration
 * order — the first step of a wire debugger, a devtools panel, or a hook
 * generator. It is the same walk the actor factory performs to build its
 * methods, so what a table says and what an actor dispatches cannot disagree.
 *
 * Every method is resolved through [`resolveSchema`], because a method is not
 * reliably a `func` node directly: schemas built from a Contract document at
 * runtime wrap each one in a lazy `rec` thunk, since cyclic services exist.
 * A `ReadonlyMap` rather than an object, so a method named `__proto__` is an
 * ordinary entry and iteration order is the service's own.
 *
 * Throws `TypeError` on a schema that is not a service, and on a method that
 * does not resolve to a func — both programmer errors, both raised eagerly
 * rather than at the call that would have used the method.
 *
 * @example
 * const table = serviceMethods(c.service({ fee: c.func([], [c.nat], "query") }));
 * table.get("fee")?.mode; // "query"
 */
export function serviceMethods(service: AnyFieldSchema): ReadonlyMap<string, ServiceMethod> {
  const node = resolveSchema(service);
  if (node.kind !== "service") {
    throw new TypeError("serviceMethods needs a service schema");
  }
  const table = new Map<string, ServiceMethod>();
  for (const name of Object.keys(node.methods)) {
    const func = resolveSchema(node.methods[name]);
    if (func.kind !== "func") {
      throw new TypeError(`method ${JSON.stringify(name)} is not a func schema`);
    }
    table.set(name, { name, mode: func.mode, args: func.args, results: func.results });
  }
  return table;
}
