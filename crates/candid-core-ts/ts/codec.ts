// The TypeScript-native Candid binary codec over the schema runtime — the
// issue #103 slice: encode domain values to Candid wire bytes and decode wire
// bytes to domain values, with `@icp-sdk/core`'s agent (or any transport)
// used only to move the resulting bytes.
//
// # Message shape
//
// A Candid message is an *argument sequence*: `DIDL` magic, a type table of
// composite types, the argument type list, then the concatenated values.
// `encodeArgs`/`decodeArgs` are the primary API; `encode`/`decode` wrap the
// one-argument case. Blob is emitted as `vec nat8` (the wire has no blob
// opcode), tuples as records with ids `0..n-1`, unit as the empty record.
//
// # Field ids
//
// Schema objects carry rendered keys, not numeric label ids; the wire needs
// ids. Both directions derive them by the recorded convention (issue #103):
// `_N_` keys are the numeric id N, every other key is the Candid label hash
// of its UTF-8 bytes. `schemaFromContract` enforces at load time that a name
// table is hash-consistent, so a schema built from a Contract cannot carry a
// key whose derived id differs from the document's — see `ts/labels.ts`.
// Two keys of one node deriving the same id fail closed (`duplicate_field_id`).
//
// # Decoding is coercion, exactly as specified
//
// Decoding is schema-directed: the expected schema drives interpretation and
// the wire type table is checked against it by the Candid coercion relation.
// The asymmetry is the spec's, not ours: an expected `opt` *absorbs* content
// mismatches to `null` (constituent mismatch, reserved, absurd pairs — never
// a hard error at the content level), while an unknown variant tag or a
// missing non-optional record field is a hard error. Extra wire record
// fields and extra trailing arguments are skipped, charging the same budgets
// as decoded values; wire `func`/`service`/future types are skippable
// wherever skipping is legal, and a value of such a type at a position the
// expected schema actually needs fails closed (`type_mismatch`) — no schema
// kind maps to them in this slice. A wire `nat` value is accepted at
// expected `int` (`nat <: int`); the reverse is a hard error.
//
// # Boxed options
//
// An expected `opt` whose inner node admits `null` — `opt opt T`, `opt null`,
// `opt reserved` — carries a present value as `{ some: v }` in both
// directions, so `None`, `Some(None)`, and `Some(Some(x))` stay three values;
// every other opt is `T | null`. The box is a domain shape only: wire bytes
// and the coercion rules are exactly those of the unboxed walk.
//
// # Strictness decisions (recorded on issue #103)
//
// - Overlong (non-minimal) LEB128/SLEB128 is rejected on decode
//   (`overlong_leb128`) — the issue mandates it; the spec's strict-inverse
//   reading supports it as an interpretation.
// - `float32` encoding is exact-or-refuse: a number that is not
//   `Math.fround`-exact fails (`unrepresentable_float32`, NaN allowed), so
//   `decode(encode(v))` is structural identity, never a silent rounding.
// - Text is Unicode scalar values: encode refuses lone surrogates
//   (`invalid_text`), decode refuses malformed UTF-8 (`invalid_utf8`),
//   overlong forms included.
// - Principals are self-contained: the canonical text form (lowercase base32
//   of CRC-32 ‖ bytes, dash-grouped by five) is implemented in
//   `principal-text.ts`, with no runtime dependency on any Principal class.
//   A principal's domain value is that text as a branded `Principal` string
//   (issue #187): decode returns the canonical text for the primitive, a
//   func reference's `principal` and a service reference alike, and encode
//   accepts exactly the strings `validate` accepts — a string holding
//   canonical text — refusing anything else with validate's code and path
//   (`invalid_type`); there is no `{ toText }` duck-typing.
//   Opaque reference forms (tag byte 0) and any external reference sequence
//   are unsupported in this slice: decode fails closed, and input with bytes
//   remaining after the value section is refused (`trailing_bytes`).
// - Encode performs its own complete validation walk — the checks mirror
//   `validate`, but reading each property exactly once, so a hostile getter
//   cannot pass validation and then hand the emitter a different value.
//
// # Bounded, fail-closed, no exceptions for control flow
//
// Nothing here throws on any value or byte string: hostile values produce
// issues via the same choke-point pattern as `validate` (`unreadable_value`),
// and malformed or adversarial bytes produce issues with explicit budgets in
// candid-core's `Limits` spirit: `maxBytes` caps input size up front,
// `maxTypeTableEntries` mirrors `max_type_nodes`, `maxDepth`/`maxElements`
// mirror the validate walk (every decoded element, skipped value, and rec hop
// charges the same
// budget — a zero-byte-per-element wire vector cannot decode more than
// `maxElements` values), and `maxNumericBytes` caps a single unbounded
// `nat`/`int` encoding. Decode stops at the first hard error: the wire
// format cannot be resynchronized after one, so the issue list is short by
// design. Every walk — the encoder's type table and value walk, the decoder's
// value, skip and reference-subtyping walks — keeps its work on an explicit
// stack rather than the host's (issue #192), so the configured limits are the
// only bounds: with `maxDepth` raised, a 100,000-level value encodes and
// decodes. The encoder's type table charges `maxDepth` for Candid nesting
// depth — each combinator level at which it opens an entry, never a `rec`
// hop — so any type the candid-core compiler accepts encodes through its
// generated module at the default limits, and a hand-built static schema
// deeper than the limit is refused with `value_depth` like a deep value.
// What can still
// exhaust the host stack is user code a walk calls — a getter, a Proxy trap,
// a rec thunk that itself recurses too deeply: that exception is caught at
// the same choke points, ahead of their catch-alls, and reported as
// `resource_limit_exceeded` with resource `stack` — never as a value or
// schema problem.
//
// Options are the exception to "never throws", deliberately (issue #190):
// they are code, not input. An unknown key, or a limit that is not a
// non-negative safe integer (`NaN`, negative, fractional, a string,
// `Infinity`), throws `TypeError` before anything is read, rather than being
// ignored — a misspelled limit silently applied the default, and a `NaN` one
// switched its bound off. `0` stays a defined fail-closed limit.
//
// # Determinism
//
// Identical inputs produce identical bytes, values, and issues: nothing
// depends on time, environment, or map iteration order. Encoded bytes are
// also independent of how the schema objects were built (issue #190,
// clarifying the claim recorded on #103). The walk's identity-keyed type
// table is rewritten into a structural canonical form (`typetable.ts`)
// before it is written, so a generated module, `schemaFromContract`, and a
// hand-built schema for the same Candid types — however they share or
// duplicate nodes, and in whatever key order a record's schema or value
// spells its fields — produce the same message. Repeated structure is
// written once; a recursive type is canonical per knot, and every schema
// generated from or loaded from a Contract has one knot per recursive node
// (see `typetable.ts` for the one case left unminimised). No result depends
// on the host's stack either: the walks are iterative (issue #192), so an
// input succeeds or fails by the configured limits alone, the same on every
// engine and every call — the only exception being user code (a getter, a
// rec thunk) that overflows the stack by itself.

import type { AnyFieldSchema, AnySchema, Principal, Schema } from "./schema.ts";
import { fieldIdOfKey, utf8BytesStrict, utf8Decode } from "./labels.ts";
import { checkOptions } from "./options.ts";
import { principalBytesFromText, principalTextFromBytes } from "./principal-text.ts";
import { canonicalTypeTable, type TableEntry } from "./typetable.ts";

// The principal text form's two directions stay part of this entry point's
// surface; their one implementation lives in the internal module the root
// entry's `principal`/`isPrincipal` and the validator share.
export { principalBytesFromText, principalTextFromBytes };

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
export type CodecCode =
  // Shared with validate, same meanings, produced by the encode walk.
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
  | "resource_limit_exceeded"
  // Codec-specific.
  | "duplicate_field_id"
  | "unrepresentable_float32"
  | "invalid_text"
  | "invalid_principal"
  | "invalid_magic"
  | "malformed_type_table"
  | "overlong_leb128"
  | "invalid_utf8"
  | "invalid_tag_byte"
  | "truncated"
  | "trailing_bytes"
  | "type_mismatch"
  | "unknown_variant_tag";

/** The `{resource, limit, observed}` triple a bound failure carries. */
export interface CodecResourceLimitInfo {
  /**
   * `stack` means the host JavaScript stack ran out mid-walk. The walks keep
   * their work on explicit stacks, so no depth of message, value or schema causes
   * it; user code a walk calls — a getter, a Proxy trap, a rec thunk —
   * recursing too deeply itself does. `limit` is the effective `maxDepth` and
   * `observed` the deepest depth the walk had reached, which says where the
   * walk was, not where the host's stack ends. Read `resource`, not a
   * comparison of the two numbers, to tell `stack` from `value_depth`.
   */
  readonly resource:
    "bytes" | "type_table_entries" | "value_depth" | "value_elements" | "numeric_bytes" | "stack";
  readonly limit: number;
  readonly observed: number;
}

/** One encode or decode failure, in `validate`'s issue shape. */
export interface CodecIssue {
  readonly code: CodecCode;
  /** `$`-rooted path into the argument sequence, `$args[i]` per argument. */
  readonly path: string;
  readonly message: string;
  readonly resource_limit?: CodecResourceLimitInfo;
}

/** Wire bytes, or the issues that stopped the encoder. Never an exception. */
export type EncodeResult =
  | { readonly ok: true; readonly bytes: Uint8Array }
  | { readonly ok: false; readonly issues: readonly CodecIssue[] };

/** One decoded value per expected argument, or the issues that refused them. */
export type DecodeResult =
  | { readonly ok: true; readonly values: readonly unknown[] }
  | { readonly ok: false; readonly issues: readonly CodecIssue[] };

/**
 * Explicit budgets for one codec call; each defaults to a `DEFAULT_MAX_*`.
 * Each is a non-negative safe integer or absent (`undefined`); any other value,
 * and any other key, makes the call throw `TypeError`.
 */
export interface CodecOptions {
  /** Input size ceiling for decode, in bytes. */
  readonly maxBytes?: number;
  /**
   * Wire type table entry cap, mirroring `Limits::max_type_nodes`. Decode
   * refuses a wire table claiming more entries. Encode charges one entry per
   * distinct composite schema node its walk meets, before repeated structure
   * is merged, so the table it writes is never larger than this.
   */
  readonly maxTypeTableEntries?: number;
  /**
   * Traversal depth cap, mirroring `Limits::max_value_depth`. Encode's
   * type-table walk charges it too, for Candid nesting depth: once per
   * combinator level at which the table gains an entry, never for a `rec`
   * hop, so a static schema nested deeper than this is refused.
   */
  readonly maxDepth?: number;
  /** Traversal element budget, mirroring validate's accounting. */
  readonly maxElements?: number;
  /** Byte cap for one unbounded `nat`/`int` encoding. */
  readonly maxNumericBytes?: number;
}

/** Default `maxBytes`: 10 MiB of input a decode will look at. */
export const DEFAULT_MAX_BYTES = 10_485_760;
/** Default `maxTypeTableEntries`. */
export const DEFAULT_MAX_TYPE_TABLE_ENTRIES = 100_000;
/** Default `maxDepth`, the same bound `validate` uses. */
export const DEFAULT_MAX_DEPTH = 256;
/** Default `maxElements`, the same bound `validate` uses. */
export const DEFAULT_MAX_ELEMENTS = 1_000_000;
/** Default `maxNumericBytes`: 1 MiB for one unbounded `nat`/`int`. */
export const DEFAULT_MAX_NUMERIC_BYTES = 1_048_576;

// ---------------------------------------------------------------------------
// Shared internals
// ---------------------------------------------------------------------------

// The erased walker union. validate.ts keeps its copy private by design; the
// schema.ts header blesses walkers narrowing on `kind` with their own union.
interface PrimitiveNode {
  readonly kind: "primitive";
  readonly primitive: string;
}
interface OptNode {
  readonly kind: "opt";
  readonly inner: AnySchema;
}
interface VecNode {
  readonly kind: "vec";
  readonly inner: AnySchema;
}
interface BlobNode {
  readonly kind: "blob";
}
interface UnitNode {
  readonly kind: "unit";
}
interface RecordNode {
  readonly kind: "record";
  readonly fields: { readonly [key: string]: AnySchema };
}
interface TupleNode {
  readonly kind: "tuple";
  readonly elements: readonly AnySchema[];
}
interface VariantNode {
  readonly kind: "variant";
  readonly arms: { readonly [key: string]: AnySchema };
}
interface RecNode {
  readonly kind: "rec";
  body(): AnySchema;
}
interface FuncNode {
  readonly kind: "func";
  readonly args: readonly AnySchema[];
  readonly results: readonly AnySchema[];
  readonly mode: "update" | "query" | "composite_query" | "oneway";
}
interface ServiceNode {
  readonly kind: "service";
  readonly methods: { readonly [name: string]: AnySchema };
}
type SchemaNode =
  | PrimitiveNode
  | OptNode
  | VecNode
  | BlobNode
  | UnitNode
  | RecordNode
  | TupleNode
  | VariantNode
  | FuncNode
  | ServiceNode
  | RecNode;

/** The nat8 element schema blob abbreviates, for the subtype relation. */
const NAT8_NODE: PrimitiveNode = { kind: "primitive", primitive: "nat8" };

/** Annotation byte per mode; `update` is the unannotated default. */
const MODE_ANNOTATION: { readonly [mode: string]: number } = {
  update: 0,
  query: 1,
  oneway: 2,
  composite_query: 3,
};

type PathSegment = string | number;

// Identical rendering to validate.ts, so one value produces one path text
// across the whole runtime — plus the argument root: the first segment of an
// args-level walk is `args[i]`, rendered as `$args[i]`.
function renderPath(path: readonly PathSegment[]): string {
  let text = "$";
  for (let index = 0; index < path.length; index += 1) {
    const segment = path[index];
    if (index === 0 && typeof segment === "string" && /^args\[[0-9]+\]$/.test(segment)) {
      text += segment;
    } else if (typeof segment === "number") {
      text += `[${segment}]`;
    } else if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(segment)) {
      text += `.${segment}`;
    } else {
      text += `[${JSON.stringify(segment)}]`;
    }
  }
  return text;
}

function hasOwn(target: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(target, key);
}

function hasOwnEnumerable(target: object, key: string): boolean {
  return Object.prototype.propertyIsEnumerable.call(target, key);
}

const typedArrayTag = Object.getOwnPropertyDescriptor(
  Object.getPrototypeOf(Uint8Array.prototype) as object,
  Symbol.toStringTag,
)?.get;

function isUint8Array(value: unknown): value is Uint8Array {
  return typedArrayTag?.call(value) === "Uint8Array";
}

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

// The module's own control-flow exceptions, `Halt` and `CoercionMismatch`,
// are recognised by identity, never by `instanceof` (issue #199). The catch
// sites that ask "is this ours?" see whatever user code threw — a getter, a
// Proxy trap, a rec thunk — and `instanceof` reads the thrown value's
// prototype, which for a Proxy is a user-controlled `getPrototypeOf` trap: a
// trap that throws raised its exception from inside the catch block and
// escaped the no-throw guarantee. A `WeakSet` lookup is keyed by the object
// itself and runs no user code, so a thrown value that is not one of ours —
// hostile or not — is simply "not ours" and reaches the catch-all. Each
// constructor registers its own instance; neither class is exported, so
// nothing else is ever registered.
const halts = new WeakSet<object>();
const mismatches = new WeakSet<object>();

/** A checked halt: issues past this point must not be recorded. */
class Halt extends Error {
  constructor() {
    super();
    halts.add(this);
  }
}

/** True only for a `Halt` this module threw; reads nothing from `error`. */
function isHalt(error: unknown): boolean {
  return typeof error === "object" && error !== null && halts.has(error);
}

/** True only for a `CoercionMismatch` this module threw; reads nothing from `error`. */
function isCoercionMismatch(error: unknown): error is CoercionMismatch {
  return typeof error === "object" && error !== null && mismatches.has(error);
}

/**
 * True when `error` is an engine reporting that its own call stack ran out:
 * a `RangeError` reading "Maximum call stack size exceeded" (V8,
 * JavaScriptCore) or an `InternalError` reading "too much recursion"
 * (SpiderMonkey, which does not throw a `RangeError`). Matches on `name` and
 * `message`, never `instanceof`; any other error, and any thrown value that
 * cannot be read safely, is not stack exhaustion.
 *
 * A private copy of `isStackExhaustion` in `validate.ts`, which carries the
 * full rationale. Neither is exported; keep the two identical.
 * `tests/stack.test.ts` asserts they agree through the entry points.
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

/** The issue both walkers record when the engine reports its stack exhausted. */
function stackIssue(path: readonly PathSegment[], limit: number, reached: number): CodecIssue {
  return {
    code: "resource_limit_exceeded",
    path: renderPath(path),
    message:
      `the host stack was exhausted at depth ${reached} (the configured maxDepth ` +
      `is ${limit}); use a shallower input or schema, or a host with a larger stack`,
    resource_limit: { resource: "stack", limit, observed: reached },
  };
}

interface Limits {
  readonly maxBytes: number;
  readonly maxTypeTableEntries: number;
  readonly maxDepth: number;
  readonly maxElements: number;
  readonly maxNumericBytes: number;
}

/** Every `CodecOptions` key; anything else in an options object throws. */
const CODEC_LIMIT_KEYS: readonly (keyof CodecOptions)[] = [
  "maxBytes",
  "maxTypeTableEntries",
  "maxDepth",
  "maxElements",
  "maxNumericBytes",
];

/**
 * The effective limits of one call, after `checkOptions` has refused unknown
 * keys and invalid values with a `TypeError` naming `entry`.
 */
function limitsOf(entry: string, options: CodecOptions): Limits {
  const checked = checkOptions(entry, options, CODEC_LIMIT_KEYS) as CodecOptions;
  return {
    maxBytes: checked.maxBytes ?? DEFAULT_MAX_BYTES,
    maxTypeTableEntries: checked.maxTypeTableEntries ?? DEFAULT_MAX_TYPE_TABLE_ENTRIES,
    maxDepth: checked.maxDepth ?? DEFAULT_MAX_DEPTH,
    maxElements: checked.maxElements ?? DEFAULT_MAX_ELEMENTS,
    maxNumericBytes: checked.maxNumericBytes ?? DEFAULT_MAX_NUMERIC_BYTES,
  };
}

// Wire type opcodes, verified against the spec's own table. `principal` is
// -24 — the -18..-23 block is the composites, not a primitive continuation.
const OP = {
  null: -1,
  bool: -2,
  nat: -3,
  int: -4,
  nat8: -5,
  nat16: -6,
  nat32: -7,
  nat64: -8,
  int8: -9,
  int16: -10,
  int32: -11,
  int64: -12,
  float32: -13,
  float64: -14,
  text: -15,
  reserved: -16,
  empty: -17,
  opt: -18,
  vec: -19,
  record: -20,
  variant: -21,
  func: -22,
  service: -23,
  principal: -24,
} as const;

const PRIMITIVE_OPCODES: { readonly [name: string]: number } = {
  null: OP.null,
  bool: OP.bool,
  nat: OP.nat,
  int: OP.int,
  nat8: OP.nat8,
  nat16: OP.nat16,
  nat32: OP.nat32,
  nat64: OP.nat64,
  int8: OP.int8,
  int16: OP.int16,
  int32: OP.int32,
  int64: OP.int64,
  float32: OP.float32,
  float64: OP.float64,
  text: OP.text,
  reserved: OP.reserved,
  empty: OP.empty,
  principal: OP.principal,
};

const FIXED_WIDTH: {
  readonly [name: string]: { bytes: number; signed: boolean };
} = {
  nat8: { bytes: 1, signed: false },
  nat16: { bytes: 2, signed: false },
  nat32: { bytes: 4, signed: false },
  int8: { bytes: 1, signed: true },
  int16: { bytes: 2, signed: true },
  int32: { bytes: 4, signed: true },
};
const FIXED_RANGES: { readonly [name: string]: readonly [number, number] } = {
  nat8: [0, 255],
  nat16: [0, 65_535],
  nat32: [0, 4_294_967_295],
  int8: [-128, 127],
  int16: [-32_768, 32_767],
  int32: [-2_147_483_648, 2_147_483_647],
};
const NAT64_MAX = 18_446_744_073_709_551_615n;
const INT64_MIN = -9_223_372_036_854_775_808n;
const INT64_MAX = 9_223_372_036_854_775_807n;

// ---------------------------------------------------------------------------
// LEB128
// ---------------------------------------------------------------------------

function writeLebNumber(out: number[], value: number): void {
  let v = value;
  do {
    let byte = v % 128;
    v = Math.floor(v / 128);
    if (v > 0) {
      byte |= 0x80;
    }
    out.push(byte);
  } while (v > 0);
}

function writeSlebBig(out: number[], value: bigint): void {
  let v = value;
  for (;;) {
    const byte = Number(v & 0x7fn);
    v >>= 7n;
    const signBit = (byte & 0x40) !== 0;
    if ((v === 0n && !signBit) || (v === -1n && signBit)) {
      out.push(byte);
      return;
    }
    out.push(byte | 0x80);
  }
}

/** Lexicographic comparison of raw byte sequences. */
function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const shorter = Math.min(a.length, b.length);
  for (let i = 0; i < shorter; i += 1) {
    if (a[i] !== b[i]) {
      return a[i] - b[i];
    }
  }
  return a.length - b.length;
}

/** Bit length of a non-negative bigint, via one hex rendering. */
function bitLength(value: bigint): number {
  if (value === 0n) {
    return 0;
  }
  const hex = value.toString(16);
  return (hex.length - 1) * 4 + (32 - Math.clz32(parseInt(hex[0], 16)));
}

/** Minimal LEB128 group count for an unsigned value. */
function lebGroupCount(value: bigint): number {
  return value === 0n ? 1 : Math.ceil(bitLength(value) / 7);
}

/** Minimal SLEB128 group count for a signed value. */
function slebGroupCount(value: bigint): number {
  const magnitude = value >= 0n ? value : -value - 1n;
  return Math.max(1, Math.ceil((bitLength(magnitude) + 1) / 7));
}

/**
 * Emit exactly `groups` 7-bit groups of the unsigned image, least
 * significant first, by recursive halving — the shift-per-group loop the
 * plain writers use is quadratic in the value's size, which would turn a
 * caller-supplied astronomical bigint into a CPU sink even under the byte
 * cap. Continuation bits are stamped afterward.
 */
function emitLebGroups(out: number[], value: bigint, groups: number): void {
  if (groups === 1) {
    out.push(Number(value & 0x7fn));
    return;
  }
  const half = groups >> 1;
  const shift = BigInt(7 * half);
  emitLebGroups(out, value & ((1n << shift) - 1n), half);
  emitLebGroups(out, value >> shift, groups - half);
}

function writeLebGroups(out: number[], unsignedImage: bigint, groups: number): void {
  const start = out.length;
  emitLebGroups(out, unsignedImage, groups);
  for (let i = start; i < out.length - 1; i += 1) {
    out[i] |= 0x80;
  }
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/**
 * Strip the `$args[0]` root from a one-argument walk, so the single-value
 * API's paths read exactly like `validate`'s.
 */
function rerootIssues(issues: readonly CodecIssue[]): readonly CodecIssue[] {
  return issues.map((issue) =>
    issue.path.startsWith("$args[0]")
      ? { ...issue, path: `$${issue.path.slice("$args[0]".length)}` }
      : issue,
  );
}

/**
 * Encode one value; the message is the one-argument sequence.
 *
 * Never throws on any value. Throws `TypeError` on an options object with an
 * unknown key or a limit that is not a non-negative safe integer — options
 * are code, and a misspelled or `NaN` limit would otherwise apply a policy
 * nobody asked for.
 */
export function encode<T>(
  schema: Schema<T>,
  value: unknown,
  options: CodecOptions = {},
): EncodeResult {
  const result = encodeWith(limitsOf("encode", options), [schema as AnyFieldSchema], [value]);
  return result.ok ? result : { ok: false, issues: rerootIssues(result.issues) };
}

/**
 * Encode an argument sequence to Candid wire bytes.
 *
 * The bytes depend only on the values and on the Candid types the schemas
 * describe, never on how the schema objects were built: a generated module,
 * `schemaFromContract`, and a hand-built schema for the same types produce
 * the same message (see "Determinism" in this module's header). Options are
 * checked as `encode` documents.
 */
export function encodeArgs(
  schemas: readonly AnyFieldSchema[],
  values: readonly unknown[],
  options: CodecOptions = {},
): EncodeResult {
  return encodeWith(limitsOf("encodeArgs", options), schemas, values);
}

function encodeWith(
  limits: Limits,
  schemas: readonly AnyFieldSchema[],
  values: readonly unknown[],
): EncodeResult {
  const encoder = new Encoder(limits);
  const path: PathSegment[] = [];
  try {
    if (schemas.length !== values.length) {
      encoder.fail("invalid_length", path, `${schemas.length} schemas for ${values.length} values`);
    }
    const provisionalRefs = schemas.map((schema, index) => {
      path.push(`args[${index}]` as string);
      const ref = encoder.typeRef(schema as SchemaNode, path, 0);
      path.pop();
      return ref;
    });
    const body: number[] = [];
    for (let index = 0; index < schemas.length; index += 1) {
      path.push(`args[${index}]` as string);
      encoder.value(schemas[index] as SchemaNode, values[index], body, path, 0);
      path.pop();
    }
    // The table the walk built is keyed by schema-object identity; the one
    // written is its structural canonical form, so equal types written
    // through differently built schemas produce equal bytes.
    const { table, roots: typeRefs } = canonicalTypeTable(encoder.table, provisionalRefs);
    // Element-by-element appends, never spreads: `push(...big)` routes the
    // whole array through the engine's argument list and throws RangeError
    // past ~124k elements — a well-formed large message, not a hostile one.
    const out: number[] = [0x44, 0x49, 0x44, 0x4c];
    writeLebNumber(out, table.length);
    for (const entry of table) {
      for (const byte of entry) {
        out.push(byte);
      }
    }
    writeLebNumber(out, typeRefs.length);
    for (const ref of typeRefs) {
      writeSlebBig(out, BigInt(ref));
    }
    encoder.rope.flattenInto(body, out);
    return { ok: true, bytes: Uint8Array.from(out) };
  } catch (error) {
    if (isStackExhaustion(error)) {
      // A property of the host, not of the value: checked before the
      // catch-all so it is never labelled a value problem. The walks are
      // iterative (issue #192); what overflows is user code they called.
      encoder.stackExhausted(path);
    } else if (!isHalt(error)) {
      // The fail-closed choke point: a hostile value that throws while being
      // read becomes an issue, never an escaping exception.
      encoder.issues.push({
        code: "unreadable_value",
        path: renderPath(path),
        message: "the value threw while being inspected",
      });
    }
    return { ok: false, issues: encoder.issues };
  }
}

/**
 * Builds one provisional `TableEntry`: literal bytes go to `bytes`, and a
 * type reference either lands there too (a negative primitive opcode) or
 * closes the current byte run and is recorded as a table index.
 */
class EntryBuilder {
  bytes: number[] = [];
  private readonly segments: number[][] = [];
  private readonly refs: number[] = [];

  ref(reference: number): void {
    if (reference < 0) {
      writeSlebBig(this.bytes, BigInt(reference));
      return;
    }
    this.segments.push(this.bytes);
    this.refs.push(reference);
    this.bytes = [];
  }

  finish(): TableEntry {
    this.segments.push(this.bytes);
    return { segments: this.segments, refs: this.refs };
  }
}

/**
 * A record's fields are written into their own buffers and appended to the
 * record's output in wire (id) order. Copying each field's bytes into its
 * parent is what the recursive encoder did, and nested records then copied
 * their whole subtree once per level: quadratic in nesting depth, which only
 * became reachable once depth stopped being bounded by the host stack. So a
 * field whose bytes are few is still copied, and one that is large — or that
 * already links others — is linked instead: recorded as "insert this buffer
 * here" and resolved once, iteratively, when the message is assembled. The
 * bytes are identical either way; copying stays bounded by a constant per
 * field, so encoding is linear in the output.
 */
class Rope {
  /** Per buffer, its links as flat pairs: offset in that buffer, then the buffer inserted there. */
  private readonly links = new Map<number[], (number | number[])[]>();

  /** Append `child`'s bytes (and whatever it links) to `parent`. */
  append(parent: number[], child: number[]): void {
    const nested = this.links.get(child);
    if (nested === undefined && child.length <= COPY_LIMIT) {
      for (let i = 0; i < child.length; i += 1) {
        parent.push(child[i]);
      }
      return;
    }
    let links = this.links.get(parent);
    if (links === undefined) {
      links = [];
      this.links.set(parent, links);
    }
    links.push(parent.length, child);
  }

  /** Copy `root` into `out` with every link resolved in place, iteratively. */
  flattenInto(root: number[], out: number[]): void {
    // Parallel stacks: the buffer being copied, the next byte, the next link.
    const buffers: number[][] = [root];
    const positions: number[] = [0];
    const nextLinks: number[] = [0];
    while (buffers.length > 0) {
      const top = buffers.length - 1;
      const buffer = buffers[top];
      const links = this.links.get(buffer);
      const linked = links !== undefined && nextLinks[top] < links.length;
      const until = linked ? (links[nextLinks[top]] as number) : buffer.length;
      for (let i = positions[top]; i < until; i += 1) {
        out.push(buffer[i]);
      }
      positions[top] = until;
      if (linked) {
        const child = links[nextLinks[top] + 1] as number[];
        nextLinks[top] += 2;
        buffers.push(child);
        positions.push(0);
        nextLinks.push(0);
      } else {
        buffers.pop();
        positions.pop();
        nextLinks.pop();
      }
    }
  }
}

/** The byte count up to which `Rope.append` copies a buffer rather than linking it. */
const COPY_LIMIT = 64;

/** The height of a type-table entry that is still being built (see `Encoder.heights`). */
const OPEN = -1;

/** A `typeStart` answer meaning "an entry was opened; its index comes later". */
const PENDING: unique symbol = Symbol("pending");

/**
 * One type-table entry under construction — the continuation of what was a
 * recursive `typeRef` call. `step` is where its children stand (per kind:
 * the next element, field or method, or for a func 0 = arguments, 1 =
 * results); `awaiting` means a child's entry is being built on top of it and
 * its index is to be recorded on resume.
 */
interface TypeFrame {
  readonly node: Exclude<SchemaNode, RecNode>;
  readonly index: number;
  readonly entry: EntryBuilder;
  readonly at: number;
  step: number;
  awaiting: boolean;
  /** A func frame's loop body is in flight (see `typeRef`'s catch). */
  inBody: boolean;
  /** How many levels below this entry its deepest descendant entry sits, so far. */
  height: number;
  /** The `rec` objects this entry was reached through, active while it is open. */
  recs: SchemaNode[] | undefined;
  fields?: { key: string; id: number; schema: AnySchema }[];
  methods?: { name: string; bytes: Uint8Array }[];
  iterator?: Iterator<AnySchema>;
}

/**
 * One composite value being encoded — the continuation of what was a
 * recursive `value` call: the child in flight, and for a record the field
 * buffers written so far; `boxed` and `variant` are the key checks that
 * follow a payload.
 */
type ValueFrame =
  | {
      readonly kind: "vec";
      readonly node: VecNode;
      readonly value: unknown[];
      readonly length: number;
      index: number;
      readonly depth: number;
      readonly out: number[];
    }
  | {
      readonly kind: "tuple";
      readonly node: TupleNode;
      readonly value: unknown[];
      index: number;
      readonly depth: number;
      readonly out: number[];
    }
  | {
      readonly kind: "record";
      readonly node: RecordNode;
      readonly value: Record<string, unknown>;
      readonly fields: readonly { key: string; id: number; schema: AnySchema }[];
      readonly keys: readonly string[];
      index: number;
      readonly buffers: Map<string, number[]>;
      /** The field in flight, whose buffer is recorded (and path popped) on resume. */
      pendingKey: string | undefined;
      pendingBuffer: number[] | undefined;
      readonly depth: number;
      readonly out: number[];
    }
  | { readonly kind: "boxed"; readonly value: Record<string, unknown>; readonly depth: number }
  | { readonly kind: "variant"; readonly value: Record<string, unknown>; readonly depth: number };

class Encoder {
  readonly issues: CodecIssue[] = [];
  /**
   * The provisional table: one entry per distinct composite schema node, in
   * first-visit order, keyed by identity. `canonicalTypeTable` turns it into
   * the structural table actually written.
   */
  readonly table: TableEntry[] = [];
  private readonly memo = new Map<SchemaNode, number>();
  private elements = 0;
  /** The deepest depth charged so far: what a stack overflow reports. */
  private reached = 0;
  /** The type-table walk's explicit stack, and the index a finished frame hands back. */
  private readonly typeStack: TypeFrame[] = [];
  private typeResult = 0;
  /**
   * Per provisional entry, how many levels below it its deepest descendant
   * entry sits (`OPEN` while the entry is still being built), so a reuse of
   * the entry can be charged for the depth it spans (`chargeReuse`).
   */
  private readonly heights: number[] = [];
  /** The `rec` chain the last `resolveType` followed, head first. */
  private readonly chain: SchemaNode[] = [];
  /** `rec` objects that entries open on the path were reached through, with counts. */
  private readonly activeRecs = new Map<SchemaNode, number>();
  /** Whether the last memo answer of `typeStart` was a back edge (see `reuse`). */
  private cut = false;
  /** The value walk's explicit stack. */
  private readonly valueStack: ValueFrame[] = [];
  /** Where records link large field buffers rather than copy them. */
  readonly rope = new Rope();

  private readonly limits: Limits;

  constructor(limits: Limits) {
    this.limits = limits;
  }

  /** Record that the host stack ran out mid-walk (see `encodeWith`'s catch). */
  stackExhausted(path: readonly PathSegment[]): void {
    this.issues.push(stackIssue(path, this.limits.maxDepth, this.reached));
  }

  fail(
    code: CodecCode,
    path: readonly PathSegment[],
    message: string,
    resource_limit?: CodecResourceLimitInfo,
  ): never {
    this.issues.push(
      resource_limit === undefined
        ? { code, path: renderPath(path), message }
        : { code, path: renderPath(path), message, resource_limit },
    );
    throw new Halt();
  }

  step(path: readonly PathSegment[], depth: number): void {
    if (depth > this.reached) {
      this.reached = depth;
    }
    if (depth > this.limits.maxDepth) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `value_depth limit ${this.limits.maxDepth} exceeded`,
        {
          resource: "value_depth",
          limit: this.limits.maxDepth,
          observed: depth,
        },
      );
    }
    this.elements += 1;
    if (this.elements > this.limits.maxElements) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `value_elements limit ${this.limits.maxElements} exceeded`,
        {
          resource: "value_elements",
          limit: this.limits.maxElements,
          observed: this.elements,
        },
      );
    }
  }

  /** Resolve rec chains to a structural node, charging depth per hop. */
  resolve(
    schema: SchemaNode,
    path: readonly PathSegment[],
    depth: number,
  ): { node: Exclude<SchemaNode, RecNode>; depth: number } {
    let node = schema;
    let hops = depth;
    while (node.kind === "rec") {
      hops += 1;
      this.step(path, hops);
      const body: unknown = node.body();
      if (
        typeof body !== "object" ||
        body === null ||
        typeof (body as { kind?: unknown }).kind !== "string"
      ) {
        this.fail("unsupported_schema", path, "a rec thunk did not produce a schema");
      }
      node = body as SchemaNode;
    }
    return { node: node as Exclude<SchemaNode, RecNode>, depth: hops };
  }

  /**
   * Rec unwrapping for type-table construction. Never element-charged: the
   * table is type-graph work bounded by its own entry cap, and charging it
   * against `maxElements` made encode's element accounting diverge from
   * validate's value-walk accounting. Never depth-charged either (issue #192,
   * see `chargeTypeDepth`): a `rec` is an alias or a lazy edge, an
   * indirection rather than a level of Candid nesting, so the returned depth
   * is the one passed in. What bounds a chain of thunks is its own cap: more
   * than `maxDepth` consecutive hops resolving one reference (256 by default;
   * a generated module needs one or two, a Contract-loaded schema one) fail
   * closed with `value_depth`, `observed` being the chain's length.
   */
  private resolveType(
    schema: SchemaNode,
    path: readonly PathSegment[],
    depth: number,
  ): { node: Exclude<SchemaNode, RecNode>; depth: number } {
    let node = schema;
    let hops = 0;
    if (depth > this.reached) {
      this.reached = depth;
    }
    this.chain.length = 0;
    while (node.kind === "rec") {
      this.chain.push(node);
      hops += 1;
      if (hops > this.limits.maxDepth) {
        this.fail(
          "resource_limit_exceeded",
          path,
          `value_depth limit ${this.limits.maxDepth} exceeded`,
          {
            resource: "value_depth",
            limit: this.limits.maxDepth,
            observed: hops,
          },
        );
      }
      const body: unknown = node.body();
      if (
        typeof body !== "object" ||
        body === null ||
        typeof (body as { kind?: unknown }).kind !== "string"
      ) {
        this.fail("unsupported_schema", path, "a rec thunk did not produce a schema");
      }
      node = body as SchemaNode;
    }
    return { node: node as Exclude<SchemaNode, RecNode>, depth };
  }

  /**
   * Issue #192, decision D1 as the maintainer settled it — the one place it
   * is made. The type-table walk charges `maxDepth` for Candid nesting depth
   * only: one unit per combinator level at which it opens an entry (the
   * argument's own type at 0, its children at 1, and so on), and nothing for
   * `rec` hops, which are aliases and lazy edges rather than nesting
   * (`resolveType` bounds those by chain instead). That is the count the
   * candid-core compiler bounds with `max_type_depth` (default 256, the
   * default `maxDepth` here), where an alias adds no depth either, so any
   * type the compiler accepts encodes through its generated module at the
   * default limits, however it is split into declarations. A hand-built
   * schema nested deeper — no `rec` needed — is refused with `value_depth`
   * at the first entry past the limit (Candid depth 257 by default), as ADR
   * 0005 asks of graph work, rather than walked to any depth. Primitives open
   * no entry and are not charged. An entry the memo answers is charged too,
   * at the position that reuses it (`reuse`).
   */
  private chargeTypeDepth(path: readonly PathSegment[], depth: number): void {
    if (depth > this.limits.maxDepth) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `value_depth limit ${this.limits.maxDepth} exceeded`,
        {
          resource: "value_depth",
          limit: this.limits.maxDepth,
          observed: depth,
        },
      );
    }
  }

  /**
   * The wire type reference for a schema: a negative primitive opcode or a
   * *provisional* type-table index (see `table`). Identity memoization here
   * bounds the walk and ties recursive knots; it no longer decides the bytes,
   * which come from the structural rewrite in `canonicalTypeTable`. Entries
   * are memoized by node identity — including the
   * *unresolved* rec node, which is the stable anchor: a generated
   * `c.rec(() => …)` body builds fresh combinator objects on every call, so
   * memoizing only the resolved node would never see the cycle and the
   * table would grow until the entry cap. Memoizing the rec object itself
   * (reserved before its children are walked) is what ties the knot.
   *
   * The walk keeps its work on an explicit stack of `TypeFrame`s (issue
   * #192): an entry being built is a frame, and a child reference that needs
   * an entry of its own pushes one and hands its index back through
   * `typeResult`. Entries are reserved, written and completed in the same
   * pre-order, with the same schema reads, as the recursive walk this
   * replaces, so the provisional table is identical.
   */
  typeRef(schema: SchemaNode, path: readonly PathSegment[], depth: number): number {
    const ref = this.typeStart(schema, path, depth);
    if (ref !== PENDING) {
      return ref;
    }
    const stack = this.typeStack;
    try {
      while (stack.length > 0) {
        this.resumeType(stack[stack.length - 1], path);
      }
    } catch (error) {
      // A throw out of the body of `for (const arg of node.args)` closed
      // that loop's iterator on its way out; a func frame whose child was in
      // flight does the same, innermost first, and the original exception
      // still propagates. (A throw from `next()` itself closes nothing.)
      for (let i = stack.length - 1; i >= 0; i -= 1) {
        const frame = stack[i];
        if (frame.inBody && frame.iterator !== undefined) {
          try {
            frame.iterator.return?.();
          } catch {
            // The loop's own exception wins, as it does for `for…of`.
          }
        }
      }
      stack.length = 0;
      throw error;
    }
    return this.typeResult;
  }

  /**
   * One `typeRef` up to its children: answer from the memo or a primitive
   * opcode, or reserve the entry, write its opening bytes, push its frame and
   * answer `PENDING` — the index arrives in `typeResult` when the frame ends.
   */
  private typeStart(
    schema: SchemaNode,
    path: readonly PathSegment[],
    depth: number,
  ): number | typeof PENDING {
    this.cut = false;
    const known = this.memo.get(schema);
    if (known !== undefined) {
      this.reuse(path, depth, known, this.activeRecs.has(schema));
      return known;
    }
    const { node, depth: at } = this.resolveType(schema, path, depth);
    if (node.kind === "primitive") {
      const opcode = PRIMITIVE_OPCODES[node.primitive];
      if (opcode === undefined) {
        this.fail(
          "unsupported_schema",
          path,
          `unknown primitive ${JSON.stringify(node.primitive)}`,
        );
      }
      this.memo.set(schema, opcode);
      return opcode;
    }
    const existing = this.memo.get(node);
    if (existing !== undefined) {
      this.reuse(path, at, existing, false);
      this.memo.set(schema, existing);
      return existing;
    }
    this.chargeTypeDepth(path, at);
    const index = this.table.length;
    if (index >= this.limits.maxTypeTableEntries) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `type_table_entries limit ${this.limits.maxTypeTableEntries} exceeded`,
        {
          resource: "type_table_entries",
          limit: this.limits.maxTypeTableEntries,
          observed: index + 1,
        },
      );
    }
    this.memo.set(node, index);
    this.memo.set(schema, index);
    this.table.push({ segments: [], refs: [] });
    this.heights.push(OPEN);
    const recs = this.chain.length === 0 ? undefined : this.chain.slice();
    if (recs !== undefined) {
      for (const rec of recs) {
        this.activeRecs.set(rec, (this.activeRecs.get(rec) ?? 0) + 1);
      }
    }
    const entry = new EntryBuilder();
    const frame: TypeFrame = {
      node,
      index,
      entry,
      at,
      step: 0,
      awaiting: false,
      inBody: false,
      height: 0,
      recs,
    };
    switch (node.kind) {
      case "opt": {
        writeSlebBig(entry.bytes, BigInt(OP.opt));
        break;
      }
      case "vec": {
        writeSlebBig(entry.bytes, BigInt(OP.vec));
        break;
      }
      case "blob": {
        writeSlebBig(entry.bytes, BigInt(OP.vec));
        writeSlebBig(entry.bytes, BigInt(OP.nat8));
        break;
      }
      case "unit": {
        writeSlebBig(entry.bytes, BigInt(OP.record));
        writeLebNumber(entry.bytes, 0);
        break;
      }
      case "tuple": {
        writeSlebBig(entry.bytes, BigInt(OP.record));
        writeLebNumber(entry.bytes, node.elements.length);
        break;
      }
      case "record":
      case "variant": {
        const map = node.kind === "record" ? node.fields : node.arms;
        frame.fields = this.sortedFields(map, path);
        writeSlebBig(entry.bytes, BigInt(node.kind === "record" ? OP.record : OP.variant));
        writeLebNumber(entry.bytes, frame.fields.length);
        break;
      }
      case "func": {
        writeSlebBig(entry.bytes, BigInt(OP.func));
        writeLebNumber(entry.bytes, node.args.length);
        frame.iterator = node.args[Symbol.iterator]();
        break;
      }
      case "service": {
        // Methods sorted by name bytes — the wire order the spec fixes.
        const methods = Object.keys(node.methods).map((name) => {
          const bytes = utf8BytesStrict(name);
          if (bytes === undefined) {
            this.fail("invalid_text", path, "a method name is Unicode scalar values");
          }
          return { name, bytes };
        });
        methods.sort((a, b) => compareBytes(a.bytes, b.bytes));
        writeSlebBig(entry.bytes, BigInt(OP.service));
        writeLebNumber(entry.bytes, methods.length);
        frame.methods = methods;
        break;
      }
    }
    this.typeStack.push(frame);
    return PENDING;
  }

  /**
   * Continue the entry on top of the type stack: record the child reference
   * that just completed, walk children until one needs an entry of its own
   * (then return; it is on the stack now) or none is left (then finish the
   * entry, hand its index to the parent and pop).
   */
  private resumeType(frame: TypeFrame, path: readonly PathSegment[]): void {
    const { node, entry, at } = frame;
    if (frame.awaiting) {
      frame.awaiting = false;
      entry.ref(this.typeResult);
      this.noteChild(frame, this.typeResult);
      frame.inBody = false;
    }
    switch (node.kind) {
      case "opt":
      case "vec": {
        if (frame.step === 0) {
          frame.step = 1;
          if (!this.typeChild(frame, node.inner as SchemaNode, path, at + 1)) {
            return;
          }
        }
        break;
      }
      case "tuple": {
        while (frame.step < node.elements.length) {
          const i = frame.step;
          frame.step += 1;
          writeLebNumber(entry.bytes, i);
          if (!this.typeChild(frame, node.elements[i] as SchemaNode, path, at + 1)) {
            return;
          }
        }
        break;
      }
      case "record":
      case "variant": {
        const fields = frame.fields as { key: string; id: number; schema: AnySchema }[];
        while (frame.step < fields.length) {
          const field = fields[frame.step];
          frame.step += 1;
          writeLebNumber(entry.bytes, field.id);
          if (!this.typeChild(frame, field.schema as SchemaNode, path, at + 1)) {
            return;
          }
        }
        break;
      }
      case "func": {
        // `step` 0 walks the arguments, 1 the results — each through the
        // array's own iterator, as the `for…of` loops this replaces did.
        for (;;) {
          const iterator = frame.iterator as Iterator<AnySchema>;
          const next = iterator.next();
          if (next.done === true) {
            if (frame.step === 0) {
              frame.step = 1;
              writeLebNumber(entry.bytes, node.results.length);
              frame.iterator = node.results[Symbol.iterator]();
              continue;
            }
            frame.iterator = undefined;
            break;
          }
          frame.inBody = true;
          if (!this.typeChild(frame, next.value as SchemaNode, path, at + 1)) {
            return;
          }
          frame.inBody = false;
        }
        const annotation = MODE_ANNOTATION[node.mode];
        if (annotation === undefined) {
          this.fail("unsupported_schema", path, `unknown method mode ${JSON.stringify(node.mode)}`);
        }
        if (annotation === 0) {
          writeLebNumber(entry.bytes, 0);
        } else {
          writeLebNumber(entry.bytes, 1);
          entry.bytes.push(annotation);
        }
        break;
      }
      case "service": {
        const methods = frame.methods as { name: string; bytes: Uint8Array }[];
        while (frame.step < methods.length) {
          const method = methods[frame.step];
          frame.step += 1;
          writeLebNumber(entry.bytes, method.bytes.length);
          for (const byte of method.bytes) {
            entry.bytes.push(byte);
          }
          const resolvedMethod = this.resolveType(
            node.methods[method.name] as SchemaNode,
            path,
            at,
          );
          if (resolvedMethod.node.kind !== "func") {
            this.fail(
              "unsupported_schema",
              path,
              `service method ${JSON.stringify(method.name)} must be a func schema`,
            );
          }
          if (!this.typeChild(frame, node.methods[method.name] as SchemaNode, path, at + 1)) {
            return;
          }
        }
        break;
      }
    }
    this.table[frame.index] = entry.finish();
    this.heights[frame.index] = frame.height;
    if (frame.recs !== undefined) {
      for (const rec of frame.recs) {
        const count = this.activeRecs.get(rec) as number;
        if (count === 1) {
          this.activeRecs.delete(rec);
        } else {
          this.activeRecs.set(rec, count - 1);
        }
      }
    }
    this.typeResult = frame.index;
    this.typeStack.pop();
  }

  /**
   * Fold a child reference into its parent's height: a completed composite
   * entry reaches `1 + its height` levels below the parent. A primitive
   * opcode is no level, and neither is a back edge to an entry still open on
   * the path — the recursion Candid cuts there too.
   */
  private noteChild(frame: TypeFrame, ref: number): void {
    if (ref >= 0) {
      const height = this.heights[ref];
      if (height !== OPEN && height + 1 > frame.height) {
        frame.height = height + 1;
      }
    }
  }

  /**
   * The Candid-depth charge for a reference the memo answers (issue #192,
   * the D1 rule; review of #208). An entry is written once but sits at every
   * position that references it, so a completed entry reused at `depth` is
   * charged as if walked there: its deepest level, `depth + height`, must be
   * within `maxDepth`. The first level past the limit is `maxDepth + 1`, which
   * `observed` reports, as when a walk opens an entry there.
   *
   * A back edge is not charged and adds no height: Candid does not expand a
   * type inside itself. That is a reference to an entry still open on the
   * path, and — `cut` — a `rec` object that an entry open on the path was
   * itself reached through: the compiler's "recursive name already active"
   * rule. It matters when an alias re-runs a knot's body (a generated
   * `ListAlias = rec(() => $List)` does): the copy's inner reference names
   * `$List`, whose original entry is complete but is active on this path.
   */
  private reuse(path: readonly PathSegment[], depth: number, ref: number, cut: boolean): void {
    if (ref < 0 || cut || this.heights[ref] === OPEN) {
      this.cut = true;
      return;
    }
    if (depth + this.heights[ref] > this.limits.maxDepth) {
      this.chargeTypeDepth(path, this.limits.maxDepth + 1);
    }
  }

  /**
   * Reference one child from `frame`'s entry. True when the reference was
   * known at once and has been recorded; false when the child opened an
   * entry of its own, whose frame is now on top — `frame` records the index
   * when it resumes.
   */
  private typeChild(
    frame: TypeFrame,
    schema: SchemaNode,
    path: readonly PathSegment[],
    depth: number,
  ): boolean {
    const ref = this.typeStart(schema, path, depth);
    if (ref === PENDING) {
      frame.awaiting = true;
      return false;
    }
    frame.entry.ref(ref);
    if (!this.cut) {
      this.noteChild(frame, ref);
    }
    return true;
  }

  /** Keys of a record/variant map with derived ids, ascending, collisions refused. */
  sortedFields(
    map: { readonly [key: string]: AnySchema },
    path: readonly PathSegment[],
  ): { key: string; id: number; schema: AnySchema }[] {
    const fields = Object.keys(map).map((key) => ({
      key,
      id: fieldIdOfKey(key),
      schema: map[key],
    }));
    fields.sort((a, b) => a.id - b.id);
    for (let i = 1; i < fields.length; i += 1) {
      if (fields[i].id === fields[i - 1].id) {
        this.fail(
          "duplicate_field_id",
          path,
          `keys ${JSON.stringify(fields[i - 1].key)} and ${JSON.stringify(
            fields[i].key,
          )} derive the same wire id ${fields[i].id}`,
        );
      }
    }
    return fields;
  }

  /**
   * Emit one value. Validation and emission are one walk: every property is
   * read exactly once, checked, and written, so validation cannot be
   * bypassed by a getter returning different values on repeated reads.
   *
   * The walk keeps its work on an explicit stack of `ValueFrame`s (issue
   * #192) rather than the host call stack, so `maxDepth` is its only depth
   * bound. Every read, charge, check, path push and pop, and byte happens in
   * the order the recursive walk this replaces performed them.
   */
  value(
    schema: SchemaNode,
    value: unknown,
    out: number[],
    path: PathSegment[],
    depth: number,
  ): void {
    this.enterValue(schema, value, out, path, depth);
    const stack = this.valueStack;
    while (stack.length > 0) {
      this.resumeValue(stack[stack.length - 1], path);
    }
  }

  /**
   * Begin one value: resolve and charge it, check it, and write it — at once
   * for a leaf, or by pushing the frame that walks its children. What were
   * tail calls (a `rec` hop, an opt's payload, a variant's payload after the
   * frame that checks its keys) continue in this loop, so this never calls
   * itself.
   */
  private enterValue(
    schema: SchemaNode,
    value: unknown,
    out: number[],
    path: PathSegment[],
    depth: number,
  ): void {
    for (;;) {
      const { node, depth: at } = this.resolve(schema, path, depth);
      this.step(path, at);
      switch (node.kind) {
        case "primitive":
          this.primitive(node.primitive, value, out, path);
          return;
        case "opt": {
          if (value === null) {
            out.push(0);
            return;
          }
          // Boxed or not is decided on the resolved inner node, exactly as
          // validate decides it; resolving here charges each rec hop once,
          // as the recursive call would have, so accounting is unchanged.
          const inner = this.resolve(node.inner as SchemaNode, path, at + 1);
          if (!admitsNull(inner.node)) {
            out.push(1);
            schema = inner.node;
            depth = inner.depth;
            continue;
          }
          if (!this.isPlainCandidate(value)) {
            this.fail(
              "invalid_type",
              path,
              `expected null or { some: … } for an opt whose inner type admits null, got ${describe(value)}`,
            );
          }
          path.push("some");
          if (!hasOwnEnumerable(value, "some")) {
            this.fail("missing_field", path, "a present boxed opt carries { some }");
          }
          out.push(1);
          this.valueStack.push({ kind: "boxed", value, depth: at });
          schema = inner.node;
          value = (value as { some?: unknown }).some;
          depth = inner.depth;
          continue;
        }
        case "vec": {
          if (!Array.isArray(value)) {
            this.fail("invalid_type", path, `expected an array, got ${describe(value)}`);
          }
          // Snapshot once: an element getter that mutates its own array's
          // length must not make the written count disagree with the
          // elements actually emitted.
          const length = value.length;
          writeLebNumber(out, length);
          this.valueStack.push({ kind: "vec", node, value, length, index: -1, depth: at, out });
          return;
        }
        case "blob": {
          if (!isUint8Array(value)) {
            this.fail("invalid_type", path, `expected a Uint8Array, got ${describe(value)}`);
          }
          const length = value.length;
          writeLebNumber(out, length);
          for (let i = 0; i < length; i += 1) {
            out.push(value[i]);
          }
          return;
        }
        case "unit": {
          if (!this.isPlainCandidate(value)) {
            this.fail("invalid_type", path, `expected an empty record, got ${describe(value)}`);
          }
          for (const key of Object.keys(value)) {
            this.step(path, at);
            path.push(key);
            this.fail("unexpected_field", path, "the empty record has no fields");
          }
          return;
        }
        case "tuple": {
          if (!Array.isArray(value)) {
            this.fail("invalid_type", path, `expected a tuple array, got ${describe(value)}`);
          }
          if (value.length !== node.elements.length) {
            this.fail(
              "invalid_length",
              path,
              `expected ${node.elements.length} elements, got ${value.length}`,
            );
          }
          this.valueStack.push({ kind: "tuple", node, value, index: -1, depth: at, out });
          return;
        }
        case "record": {
          if (!this.isPlainCandidate(value)) {
            this.fail("invalid_type", path, `expected a record, got ${describe(value)}`);
          }
          const fields = this.sortedFields(node.fields, path);
          // Validate and read in declaration order — the order validate
          // reports in, so the first issue's path agrees — writing each
          // field into its own buffer; the wire wants ascending-id order, so
          // the frame's end appends the buffers in that order (see `Rope`),
          // never the reads.
          this.valueStack.push({
            kind: "record",
            node,
            value,
            fields,
            keys: Object.keys(node.fields),
            index: -1,
            buffers: new Map<string, number[]>(),
            pendingKey: undefined,
            pendingBuffer: undefined,
            depth: at,
            out,
          });
          return;
        }
        case "variant": {
          if (!this.isPlainCandidate(value)) {
            this.fail("invalid_type", path, `expected a variant, got ${describe(value)}`);
          }
          path.push("tag");
          if (!hasOwnEnumerable(value, "tag")) {
            this.fail("missing_field", path, "a variant value carries a tag");
          }
          const tag = (value as { tag?: unknown }).tag;
          if (typeof tag !== "string") {
            this.fail("invalid_type", path, `expected a string tag, got ${describe(tag)}`);
          }
          if (!hasOwn(node.arms, tag)) {
            this.fail("unknown_tag", path, `${JSON.stringify(tag)} is not an arm of this variant`);
          }
          path.pop();
          const arms = this.sortedFields(node.arms, path);
          const index = arms.findIndex((arm) => arm.key === tag);
          writeLebNumber(out, index);
          const resolved = this.resolve(arms[index].schema as SchemaNode, path, at);
          const tagOnly = resolved.node.kind === "primitive" && resolved.node.primitive === "null";
          if (tagOnly) {
            // Mirror validate's walk exactly: the first non-tag key in
            // enumeration order is the issue, whatever its name.
            for (const key of Object.keys(value)) {
              this.step(path, at);
              if (key !== "tag") {
                path.push(key);
                this.fail("unexpected_field", path, "a null-payload arm is a bare { tag }");
              }
            }
            return;
          }
          path.push("value");
          if (!hasOwnEnumerable(value, "value")) {
            this.fail("missing_field", path, "this arm carries a payload");
          }
          this.valueStack.push({ kind: "variant", value, depth: at });
          schema = resolved.node;
          value = (value as { value?: unknown }).value;
          depth = resolved.depth + 1;
          continue;
        }
        case "func": {
          // A func value is inert reference data: { principal, method },
          // emitted in the transparent public form (issue #104).
          if (!this.isPlainCandidate(value)) {
            this.fail(
              "invalid_type",
              path,
              `expected a func reference ({ principal, method }), got ${describe(value)}`,
            );
          }
          path.push("principal");
          if (!hasOwnEnumerable(value, "principal")) {
            this.fail("missing_field", path, "a func reference names a principal");
          }
          const principalBytes = this.principalBytes(value.principal, path);
          path.pop();
          path.push("method");
          if (!hasOwnEnumerable(value, "method")) {
            this.fail("missing_field", path, "a func reference names a method");
          }
          const method = value.method;
          if (typeof method !== "string" || method.length === 0) {
            this.fail("invalid_type", path, "a method name is a non-empty string");
          }
          const methodBytes = utf8BytesStrict(method);
          if (methodBytes === undefined) {
            this.fail("invalid_text", path, "a method name is Unicode scalar values");
          }
          path.pop();
          for (const key of Object.keys(value)) {
            this.step(path, at);
            if (key !== "principal" && key !== "method") {
              path.push(key);
              this.fail("unexpected_field", path, "a func reference is { principal, method }");
            }
          }
          out.push(1);
          out.push(1);
          writeLebNumber(out, principalBytes.length);
          for (const byte of principalBytes) {
            out.push(byte);
          }
          writeLebNumber(out, methodBytes.length);
          for (const byte of methodBytes) {
            out.push(byte);
          }
          return;
        }
        case "service": {
          // A service value is the principal of a running service.
          const bytes = this.principalBytes(value, path);
          out.push(1);
          writeLebNumber(out, bytes.length);
          for (const byte of bytes) {
            out.push(byte);
          }
          return;
        }
      }
      return;
    }
  }

  /**
   * Advance the value frame on top of the stack by one child — start it
   * (it may push a frame of its own) or, with none left, finish the frame and
   * pop it.
   */
  private resumeValue(frame: ValueFrame, path: PathSegment[]): void {
    const stack = this.valueStack;
    switch (frame.kind) {
      case "vec": {
        for (;;) {
          if (frame.index >= 0) {
            path.pop();
          }
          frame.index += 1;
          if (frame.index >= frame.length) {
            stack.pop();
            return;
          }
          path.push(frame.index);
          this.enterValue(
            frame.node.inner as SchemaNode,
            frame.value[frame.index],
            frame.out,
            path,
            frame.depth + 1,
          );
          // A child that pushed a frame runs first; a leaf is already done,
          // and this loop carries on without a round trip through `value`.
          if (stack[stack.length - 1] !== frame) {
            return;
          }
        }
      }
      case "tuple": {
        const { node } = frame;
        for (;;) {
          if (frame.index >= 0) {
            path.pop();
          }
          frame.index += 1;
          if (frame.index >= node.elements.length) {
            stack.pop();
            return;
          }
          path.push(frame.index);
          this.enterValue(
            node.elements[frame.index] as SchemaNode,
            frame.value[frame.index],
            frame.out,
            path,
            frame.depth + 1,
          );
          if (stack[stack.length - 1] !== frame) {
            return;
          }
        }
      }
      case "record": {
        const { node, value } = frame;
        for (;;) {
          if (frame.pendingKey !== undefined) {
            frame.buffers.set(frame.pendingKey, frame.pendingBuffer as number[]);
            frame.pendingKey = undefined;
            path.pop();
          }
          frame.index += 1;
          if (frame.index >= frame.keys.length) {
            break;
          }
          const key = frame.keys[frame.index];
          path.push(key);
          if (!hasOwnEnumerable(value, key)) {
            this.fail("missing_field", path, "required field is missing");
          }
          const buffer: number[] = [];
          frame.pendingKey = key;
          frame.pendingBuffer = buffer;
          this.enterValue(
            node.fields[key] as SchemaNode,
            value[key],
            buffer,
            path,
            frame.depth + 1,
          );
          if (stack[stack.length - 1] !== frame) {
            return;
          }
        }
        stack.pop();
        for (const field of frame.fields) {
          this.rope.append(frame.out, frame.buffers.get(field.key) as number[]);
        }
        for (const key of Object.keys(value)) {
          this.step(path, frame.depth);
          if (!hasOwn(node.fields, key)) {
            path.push(key);
            this.fail("unexpected_field", path, "field is not part of this record");
          }
        }
        return;
      }
      case "boxed": {
        path.pop();
        this.valueStack.pop();
        for (const key of Object.keys(frame.value)) {
          this.step(path, frame.depth);
          if (key !== "some") {
            path.push(key);
            this.fail("unexpected_field", path, "a present boxed opt is exactly { some }");
          }
        }
        return;
      }
      case "variant": {
        path.pop();
        this.valueStack.pop();
        for (const key of Object.keys(frame.value)) {
          this.step(path, frame.depth);
          if (key !== "tag" && key !== "value") {
            path.push(key);
            this.fail("unexpected_field", path, "a variant value is { tag, value }");
          }
        }
        return;
      }
    }
  }

  /**
   * The raw id bytes of a `Principal` — the principal primitive, a func
   * reference's `principal`, a service reference — or fail closed. Strict:
   * exactly the strings `validate` accepts, refused with its code and path.
   * An object with `toText()` (an SDK `Principal`) is not read; callers
   * convert it once with `principal()`.
   */
  private principalBytes(value: unknown, path: PathSegment[]): Uint8Array {
    if (typeof value !== "string") {
      this.fail(
        "invalid_type",
        path,
        `expected a Principal (canonical principal text), got ${describe(value)}`,
      );
    }
    const bytes = principalBytesFromText(value);
    if (bytes === undefined) {
      this.fail(
        "invalid_type",
        path,
        "expected a Principal, got a string that is not canonical principal text",
      );
    }
    return bytes;
  }

  /**
   * An unbounded nat/int value, capped by the same numeric byte budget the
   * decoder enforces — encode and decode share one resource model — and
   * emitted by recursive halving so the cap bounds time as well as output.
   */
  private writeUnbounded(
    out: number[],
    value: bigint,
    signed: boolean,
    path: readonly PathSegment[],
  ): void {
    const groups = signed ? slebGroupCount(value) : lebGroupCount(value);
    if (groups > this.limits.maxNumericBytes) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `numeric_bytes limit ${this.limits.maxNumericBytes} exceeded`,
        {
          resource: "numeric_bytes",
          limit: this.limits.maxNumericBytes,
          observed: groups,
        },
      );
    }
    const image = value >= 0n ? value : value + (1n << BigInt(7 * groups));
    writeLebGroups(out, image, groups);
  }

  private primitive(name: string, value: unknown, out: number[], path: PathSegment[]): void {
    switch (name) {
      case "null":
        if (value !== null) {
          this.fail("invalid_type", path, `expected null, got ${describe(value)}`);
        }
        return;
      case "reserved":
        return;
      case "empty":
        this.fail("uninhabited_type", path, "empty has no values");
        return;
      case "bool":
        if (typeof value !== "boolean") {
          this.fail("invalid_type", path, `expected a boolean, got ${describe(value)}`);
        }
        out.push(value ? 1 : 0);
        return;
      case "nat":
        if (typeof value !== "bigint") {
          this.fail("invalid_type", path, `expected a bigint, got ${describe(value)}`);
        }
        if (value < 0n) {
          this.fail("out_of_range", path, "nat is non-negative");
        }
        this.writeUnbounded(out, value, false, path);
        return;
      case "int":
        if (typeof value !== "bigint") {
          this.fail("invalid_type", path, `expected a bigint, got ${describe(value)}`);
        }
        this.writeUnbounded(out, value, true, path);
        return;
      case "nat64":
      case "int64": {
        if (typeof value !== "bigint") {
          this.fail("invalid_type", path, `expected a bigint, got ${describe(value)}`);
        }
        const [min, max] = name === "nat64" ? [0n, NAT64_MAX] : [INT64_MIN, INT64_MAX];
        if (value < min || value > max) {
          this.fail("out_of_range", path, `${name} is ${min}..=${max}`);
        }
        const unsigned = value < 0n ? value + (1n << 64n) : value;
        for (let shift = 0n; shift < 64n; shift += 8n) {
          out.push(Number((unsigned >> shift) & 0xffn));
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
          this.fail("invalid_type", path, `expected a number, got ${describe(value)}`);
        }
        if (!Number.isInteger(value)) {
          this.fail("not_integer", path, `${name} requires an integer`);
        }
        const [min, max] = FIXED_RANGES[name];
        if (value < min || value > max) {
          this.fail("out_of_range", path, `${name} is ${min}..=${max}`);
        }
        const width = FIXED_WIDTH[name].bytes;
        let unsigned = value < 0 ? value + 2 ** (width * 8) : value;
        for (let i = 0; i < width; i += 1) {
          out.push(unsigned & 0xff);
          unsigned = Math.floor(unsigned / 256);
        }
        return;
      }
      case "float32": {
        if (typeof value !== "number") {
          this.fail("invalid_type", path, `expected a number, got ${describe(value)}`);
        }
        if (!Number.isNaN(value) && Math.fround(value) !== value) {
          this.fail(
            "unrepresentable_float32",
            path,
            "not exactly representable as float32; round with Math.fround first",
          );
        }
        const view = new DataView(new ArrayBuffer(4));
        view.setFloat32(0, value, true);
        for (let i = 0; i < 4; i += 1) {
          out.push(view.getUint8(i));
        }
        return;
      }
      case "float64": {
        if (typeof value !== "number") {
          this.fail("invalid_type", path, `expected a number, got ${describe(value)}`);
        }
        const view = new DataView(new ArrayBuffer(8));
        view.setFloat64(0, value, true);
        for (let i = 0; i < 8; i += 1) {
          out.push(view.getUint8(i));
        }
        return;
      }
      case "text": {
        if (typeof value !== "string") {
          this.fail("invalid_type", path, `expected a string, got ${describe(value)}`);
        }
        const bytes = utf8BytesStrict(value);
        if (bytes === undefined) {
          this.fail(
            "invalid_text",
            path,
            "text contains an unpaired surrogate; Candid text is Unicode scalar values",
          );
        }
        writeLebNumber(out, bytes.length);
        for (const byte of bytes) {
          out.push(byte);
        }
        return;
      }
      case "principal": {
        const bytes = this.principalBytes(value, path);
        out.push(1);
        writeLebNumber(out, bytes.length);
        for (const byte of bytes) {
          out.push(byte);
        }
        return;
      }
      default:
        this.fail("unsupported_schema", path, `unknown primitive ${JSON.stringify(name)}`);
    }
  }

  private isPlainCandidate(value: unknown): value is Record<string, unknown> {
    return (
      typeof value === "object" && value !== null && !Array.isArray(value) && !isUint8Array(value)
    );
  }
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/**
 * Decode a one-argument message against one schema.
 *
 * Never throws on any bytes. Throws `TypeError` on an options object with an
 * unknown key or a limit that is not a non-negative safe integer, exactly as
 * `encode` does.
 */
export function decode<T>(
  schema: Schema<T>,
  bytes: Uint8Array,
  options: CodecOptions = {},
): { ok: true; value: unknown } | { ok: false; issues: readonly CodecIssue[] } {
  const result = decodeWith(limitsOf("decode", options), [schema as AnyFieldSchema], bytes);
  return result.ok
    ? { ok: true, value: result.values[0] }
    : { ok: false, issues: rerootIssues(result.issues) };
}

/**
 * Decode a Candid message against an expected argument schema sequence.
 * Options are checked as `decode` documents.
 */
export function decodeArgs(
  schemas: readonly AnyFieldSchema[],
  bytes: Uint8Array,
  options: CodecOptions = {},
): DecodeResult {
  return decodeWith(limitsOf("decodeArgs", options), schemas, bytes);
}

function decodeWith(
  limits: Limits,
  schemas: readonly AnyFieldSchema[],
  bytes: Uint8Array,
): DecodeResult {
  const decoder = new Decoder(bytes, limits);
  const path: PathSegment[] = [];
  try {
    if (!isUint8Array(bytes)) {
      decoder.fail("invalid_type", path, `expected a Uint8Array, got ${describe(bytes)}`);
    }
    if (bytes.length > limits.maxBytes) {
      decoder.fail("resource_limit_exceeded", path, `bytes limit ${limits.maxBytes} exceeded`, {
        resource: "bytes",
        limit: limits.maxBytes,
        observed: bytes.length,
      });
    }
    decoder.header(path);
    const values: unknown[] = [];
    for (let index = 0; index < schemas.length; index += 1) {
      path.push(`args[${index}]` as string);
      if (index < decoder.argTypes.length) {
        values.push(
          decoder.valueAt(decoder.argTypes[index], schemas[index] as SchemaNode, path, 0),
        );
      } else {
        // A missing trailing argument follows the record-field rule: null
        // for opt-like expected types, a hard error otherwise.
        values.push(decoder.missingValue(schemas[index] as SchemaNode, path, 0));
      }
      path.pop();
    }
    // Extra trailing wire arguments are skipped per the tuple rule, then the
    // input must be exhausted: this slice supports no reference sequence.
    for (let index = schemas.length; index < decoder.argTypes.length; index += 1) {
      decoder.skipValue(decoder.argTypes[index], path, 0);
    }
    if (!decoder.exhausted()) {
      decoder.fail(
        "trailing_bytes",
        path,
        "bytes remain after the value section; reference sequences are unsupported",
      );
    }
    return { ok: true, values };
  } catch (error) {
    if (isStackExhaustion(error)) {
      // A property of the host, not of the schema: checked before the
      // catch-all so it is never labelled a schema problem. The walks are
      // iterative (issue #192); what overflows is user code they called.
      // (A `CoercionMismatch` is a plain `Error`, never stack-shaped, so
      // asking this first moves no verdict.)
      decoder.stackExhausted(path);
    } else if (isCoercionMismatch(error)) {
      // A type-level mismatch with no enclosing expected `opt` to absorb it
      // is the spec's hard error.
      decoder.issues.push({
        code: error.code,
        path: error.path,
        message: error.detail,
      });
    } else if (!isHalt(error)) {
      // The schema-side choke point: a rec thunk (or other schema surface)
      // that throws becomes an issue, never an escaping exception.
      decoder.issues.push({
        code: "unsupported_schema",
        path: renderPath(path),
        message: "the schema threw while being traversed",
      });
    }
    return { ok: false, issues: decoder.issues };
  }
}

/** A coercion failure: absorbed to null by the nearest expected `opt`. */
class CoercionMismatch extends Error {
  readonly code: CodecCode;
  readonly path: string;
  readonly detail: string;

  constructor(code: CodecCode, path: string, detail: string) {
    super(detail);
    this.code = code;
    this.path = path;
    this.detail = detail;
    mismatches.add(this);
  }
}

type WireEntry =
  | { readonly kind: "opt" | "vec"; readonly inner: number }
  | {
      readonly kind: "record" | "variant";
      readonly ids: readonly number[];
      readonly types: readonly number[];
    }
  | {
      readonly kind: "func";
      readonly args: readonly number[];
      readonly results: readonly number[];
      /** 0 = update (no annotation); else the 1..3 annotation byte. */
      readonly annotation: number;
    }
  | {
      readonly kind: "service";
      readonly methods: readonly { readonly name: string; readonly type: number }[];
    }
  | { readonly kind: "future" };

/** The wire entries by kind, as the walks below narrow them. */
type VecWire = Extract<WireEntry, { readonly kind: "opt" | "vec" }>;
type FieldsWire = Extract<WireEntry, { readonly kind: "record" | "variant" }>;
type FuncWire = Extract<WireEntry, { readonly kind: "func" }>;
type ServiceWire = Extract<WireEntry, { readonly kind: "service" }>;

/**
 * An expected `opt` decoding its constituent — the absorption frame. `start`
 * has not begun the constituent; `decoding` is waiting for it, and is the
 * phase a `CoercionMismatch` unwinds to; `absorbed` is set by that unwinding
 * (rewind and skip next); `skipping` waits for the skip. `rewind` and
 * `pathLength` are the cursor and the path depth at the opt itself, both
 * restored when a mismatch is absorbed.
 */
interface OptFrame {
  readonly kind: "opt";
  readonly wire: number;
  readonly node: Exclude<SchemaNode, RecNode>;
  readonly depth: number;
  readonly skipDepth: number;
  readonly rewind: number;
  readonly pathLength: number;
  phase: "start" | "decoding" | "absorbed" | "skipping";
}

/** A wire record decoding at an expected record, tuple or unit. */
interface RecordFrame {
  readonly kind: "record";
  readonly entry: { readonly ids: readonly number[]; readonly types: readonly number[] };
  readonly node: Exclude<SchemaNode, RecNode>;
  readonly expected: readonly { key: string | number; id: number; schema: AnySchema }[];
  readonly depth: number;
  readonly out: Record<string | number, unknown>;
  cursor: number;
  w: number;
  awaiting: "none" | "field" | "skip";
}

/**
 * One value (or skip) in progress on the decoder's explicit stack — the
 * continuation of what was a recursive call.
 */
type DecodeFrame =
  | OptFrame
  | RecordFrame
  | {
      readonly kind: "vec";
      readonly inner: number;
      readonly node: VecNode;
      readonly depth: number;
      readonly length: number;
      index: number;
      readonly out: unknown[];
      awaiting: boolean;
    }
  | {
      readonly kind: "variant";
      readonly tag: string;
      readonly tagOnly: boolean;
      readonly wire: number;
      readonly node: Exclude<SchemaNode, RecNode>;
      readonly depth: number;
      started: boolean;
    }
  | { readonly kind: "reserved"; readonly wire: number; readonly depth: number; started: boolean }
  | { readonly kind: "skip-vec"; readonly inner: number; remaining: number; readonly depth: number }
  | {
      readonly kind: "skip-record";
      readonly types: readonly number[];
      index: number;
      readonly depth: number;
    };

/** One pair of the subtype relation under test (see `wireSubtypeOfSchema`). */
interface SubFrame {
  readonly key: string;
  readonly wire: number;
  readonly node: Exclude<SchemaNode, RecNode>;
  readonly wireOnLeft: boolean;
  readonly depth: number;
  phase: number;
  index: number;
  entry?: WireEntry;
  expected?: { id: number; schema: AnySchema }[];
  names?: string[];
  sub?: { args: readonly unknown[]; results: readonly unknown[] };
  sup?: { args: readonly unknown[]; results: readonly unknown[] };
}

/** A child pair a `SubFrame` needs decided before it can continue. */
interface SubRequest {
  readonly wire: number;
  readonly schema: AnySchema;
  readonly wireOnLeft: boolean;
}

/**
 * Combine 7-bit LEB groups (least significant first) into one bigint by
 * recursive halving. A linear fold shifts an ever-growing bigint once per
 * byte — quadratic in encoded length, which turned the numeric byte cap
 * into a CPU budget rather than a memory one; halving keeps the work
 * quasi-linear so the cap bounds time as well.
 */
function combineLebGroups(parts: readonly number[], from: number, to: number): bigint {
  const length = to - from;
  if (length === 0) {
    return 0n;
  }
  if (length === 1) {
    return BigInt(parts[from] & 0x7f);
  }
  const mid = from + (length >> 1);
  const low = combineLebGroups(parts, from, mid);
  const high = combineLebGroups(parts, mid, to);
  return (high << BigInt(7 * (mid - from))) | low;
}

class Decoder {
  readonly issues: CodecIssue[] = [];
  readonly entries: WireEntry[] = [];
  argTypes: number[] = [];
  private readonly schemaIds = new Map<object, number>();
  /** One memo per decode run: reference checks repeat across values. */
  private readonly subtypeMemo = new Map<string, boolean>();
  private offset = 0;
  private elements = 0;
  /** The deepest depth charged so far: what a stack overflow reports. */
  private reached = 0;
  /** The value walk's explicit stack, and the value a finished frame hands back. */
  private readonly stack: DecodeFrame[] = [];
  private result: unknown = undefined;

  private readonly bytes: Uint8Array;
  private readonly limits: Limits;

  constructor(bytes: Uint8Array, limits: Limits) {
    this.bytes = bytes;
    this.limits = limits;
  }

  /** Record that the host stack ran out mid-walk (see `decodeWith`'s catch). */
  stackExhausted(path: readonly PathSegment[]): void {
    this.issues.push(stackIssue(path, this.limits.maxDepth, this.reached));
  }

  fail(
    code: CodecCode,
    path: readonly PathSegment[],
    message: string,
    resource_limit?: CodecResourceLimitInfo,
  ): never {
    this.issues.push(
      resource_limit === undefined
        ? { code, path: renderPath(path), message }
        : { code, path: renderPath(path), message, resource_limit },
    );
    throw new Halt();
  }

  /** A type-level mismatch: hard unless an expected `opt` absorbs it. */
  private mismatch(code: CodecCode, path: readonly PathSegment[], detail: string): never {
    throw new CoercionMismatch(code, renderPath(path), detail);
  }

  exhausted(): boolean {
    return this.offset === this.bytes.length;
  }

  private step(path: readonly PathSegment[], depth: number): void {
    if (depth > this.reached) {
      this.reached = depth;
    }
    if (depth > this.limits.maxDepth) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `value_depth limit ${this.limits.maxDepth} exceeded`,
        { resource: "value_depth", limit: this.limits.maxDepth, observed: depth },
      );
    }
    this.elements += 1;
    if (this.elements > this.limits.maxElements) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `value_elements limit ${this.limits.maxElements} exceeded`,
        {
          resource: "value_elements",
          limit: this.limits.maxElements,
          observed: this.elements,
        },
      );
    }
  }

  private byte(path: readonly PathSegment[]): number {
    if (this.offset >= this.bytes.length) {
      this.fail("truncated", path, "unexpected end of input");
    }
    const value = this.bytes[this.offset];
    this.offset += 1;
    return value;
  }

  private raw(length: number, path: readonly PathSegment[]): Uint8Array {
    if (this.offset + length > this.bytes.length) {
      this.fail("truncated", path, "unexpected end of input");
    }
    const slice = this.bytes.subarray(this.offset, this.offset + length);
    this.offset += length;
    return slice;
  }

  /** Minimal-form unsigned LEB128 bounded to a u32-representable count. */
  private lebU32(path: readonly PathSegment[], what: string): number {
    let result = 0;
    let shift = 0;
    let count = 0;
    let last = 0;
    for (;;) {
      const byte = this.byte(path);
      count += 1;
      last = byte;
      if (count > 5 || (count === 5 && (byte & 0x70) !== 0)) {
        this.fail("invalid_length", path, `${what} does not fit in 32 bits`);
      }
      result += (byte & 0x7f) * 2 ** shift;
      if ((byte & 0x80) === 0) {
        break;
      }
      shift += 7;
    }
    if (count > 1 && last === 0) {
      this.fail("overlong_leb128", path, `${what} uses a non-minimal encoding`);
    }
    return result;
  }

  /** Minimal-form unsigned LEB128 of unbounded magnitude, byte-capped. */
  private lebBig(path: readonly PathSegment[]): bigint {
    const parts: number[] = [];
    for (;;) {
      const byte = this.byte(path);
      parts.push(byte);
      if (parts.length > this.limits.maxNumericBytes) {
        this.fail(
          "resource_limit_exceeded",
          path,
          `numeric_bytes limit ${this.limits.maxNumericBytes} exceeded`,
          {
            resource: "numeric_bytes",
            limit: this.limits.maxNumericBytes,
            observed: parts.length,
          },
        );
      }
      if ((byte & 0x80) === 0) {
        break;
      }
    }
    if (parts.length > 1 && parts[parts.length - 1] === 0) {
      this.fail("overlong_leb128", path, "nat uses a non-minimal encoding");
    }
    return combineLebGroups(parts, 0, parts.length);
  }

  /** Minimal-form signed LEB128 of unbounded magnitude, byte-capped. */
  private slebBig(path: readonly PathSegment[]): bigint {
    const parts: number[] = [];
    for (;;) {
      const byte = this.byte(path);
      parts.push(byte);
      if (parts.length > this.limits.maxNumericBytes) {
        this.fail(
          "resource_limit_exceeded",
          path,
          `numeric_bytes limit ${this.limits.maxNumericBytes} exceeded`,
          {
            resource: "numeric_bytes",
            limit: this.limits.maxNumericBytes,
            observed: parts.length,
          },
        );
      }
      if ((byte & 0x80) === 0) {
        break;
      }
    }
    const last = parts[parts.length - 1];
    if (parts.length > 1) {
      const prior = parts[parts.length - 2];
      if ((last === 0x00 && (prior & 0x40) === 0) || (last === 0x7f && (prior & 0x40) !== 0)) {
        this.fail("overlong_leb128", path, "int uses a non-minimal encoding");
      }
    }
    let result = combineLebGroups(parts, 0, parts.length);
    if ((last & 0x40) !== 0) {
      result -= 1n << BigInt(parts.length * 7);
    }
    return result;
  }

  /** A type reference: a primitive opcode or an in-range table index. */
  private typeRef(path: readonly PathSegment[]): number {
    const value = this.slebBig(path);
    if (value >= 0n) {
      const index = Number(value);
      if (!Number.isSafeInteger(index)) {
        this.fail("malformed_type_table", path, "type index out of range");
      }
      return index;
    }
    const opcode = Number(value);
    const primitive = opcode >= -17 || opcode === OP.principal;
    if (!primitive) {
      this.fail(
        "malformed_type_table",
        path,
        `opcode ${opcode} is not a primitive and cannot appear inline`,
      );
    }
    return opcode;
  }

  header(path: readonly PathSegment[]): void {
    const magic = this.raw(4, path);
    if (magic[0] !== 0x44 || magic[1] !== 0x49 || magic[2] !== 0x44 || magic[3] !== 0x4c) {
      this.fail("invalid_magic", path, "input does not start with DIDL");
    }
    const count = this.lebU32(path, "type table size");
    if (count > this.limits.maxTypeTableEntries) {
      this.fail(
        "resource_limit_exceeded",
        path,
        `type_table_entries limit ${this.limits.maxTypeTableEntries} exceeded`,
        {
          resource: "type_table_entries",
          limit: this.limits.maxTypeTableEntries,
          observed: count,
        },
      );
    }
    for (let i = 0; i < count; i += 1) {
      this.entries.push(this.tableEntry(path));
    }
    // References may point forward; validate them only once the table is
    // complete, and enforce the spec's structural constraints.
    for (const entry of this.entries) {
      if (entry.kind === "opt" || entry.kind === "vec") {
        this.checkRef(entry.inner, path);
      } else if (entry.kind === "record" || entry.kind === "variant") {
        for (const type of entry.types) {
          this.checkRef(type, path);
        }
      } else if (entry.kind === "func") {
        for (const type of [...entry.args, ...entry.results]) {
          this.checkRef(type, path);
        }
      } else if (entry.kind === "service") {
        for (const method of entry.methods) {
          this.checkRef(method.type, path);
          // A service method type must denote a function type — the one
          // structural constraint the spec states about reference types.
          if (method.type < 0 || this.entries[method.type].kind !== "func") {
            this.fail("malformed_type_table", path, "a service method must denote a function type");
          }
        }
      }
    }
    const argCount = this.lebU32(path, "argument count");
    for (let i = 0; i < argCount; i += 1) {
      const ref = this.typeRef(path);
      this.checkRef(ref, path);
      this.argTypes.push(ref);
    }
  }

  private checkRef(ref: number, path: readonly PathSegment[]): void {
    if (ref >= 0 && ref >= this.entries.length) {
      this.fail(
        "malformed_type_table",
        path,
        `type index ${ref} is outside the ${this.entries.length}-entry table`,
      );
    }
  }

  private tableEntry(path: readonly PathSegment[]): WireEntry {
    const opcode = this.slebBig(path);
    if (opcode >= 0n || opcode >= BigInt(OP.empty) || opcode === BigInt(OP.principal)) {
      this.fail("malformed_type_table", path, "the type table may only contain composite types");
    }
    switch (Number(opcode)) {
      case OP.opt:
        return { kind: "opt", inner: this.typeRef(path) };
      case OP.vec:
        return { kind: "vec", inner: this.typeRef(path) };
      case OP.record:
      case OP.variant: {
        const count = this.lebU32(path, "field count");
        const ids: number[] = [];
        const types: number[] = [];
        let previous = -1;
        for (let i = 0; i < count; i += 1) {
          this.step(path, 0);
          const id = this.lebU32(path, "field id");
          if (id <= previous) {
            this.fail("malformed_type_table", path, "field ids must be strictly increasing");
          }
          previous = id;
          ids.push(id);
          types.push(this.typeRef(path));
        }
        return Number(opcode) === OP.record
          ? { kind: "record", ids, types }
          : { kind: "variant", ids, types };
      }
      case OP.func: {
        const args: number[] = [];
        const argCount = this.lebU32(path, "func argument count");
        for (let i = 0; i < argCount; i += 1) {
          this.step(path, 0);
          args.push(this.typeRef(path));
        }
        const results: number[] = [];
        const resultCount = this.lebU32(path, "func result count");
        for (let i = 0; i < resultCount; i += 1) {
          this.step(path, 0);
          results.push(this.typeRef(path));
        }
        const annotations = this.lebU32(path, "func annotation count");
        // The reference implementation refuses more than one annotation;
        // mirror it — fail-closed parity on headers, not just values.
        if (annotations > 1) {
          this.fail("malformed_type_table", path, "a function type carries at most one annotation");
        }
        let annotation = 0;
        for (let i = 0; i < annotations; i += 1) {
          annotation = this.byte(path);
          if (annotation < 1 || annotation > 3) {
            this.fail("malformed_type_table", path, "unknown function annotation");
          }
        }
        return { kind: "func", args, results, annotation };
      }
      case OP.service: {
        const methods: { name: string; type: number }[] = [];
        const methodCount = this.lebU32(path, "service method count");
        let previousName: Uint8Array | undefined;
        for (let i = 0; i < methodCount; i += 1) {
          this.step(path, 0);
          const length = this.lebU32(path, "method name length");
          const raw = Uint8Array.from(this.raw(length, path));
          const name = utf8Decode(raw);
          if (name === undefined) {
            this.fail("invalid_utf8", path, "method name is not well-formed UTF-8");
          }
          // Sorted strictly by name bytes, duplicates included — the
          // reference implementation refuses both.
          if (previousName !== undefined && compareBytes(previousName, raw) >= 0) {
            this.fail(
              "malformed_type_table",
              path,
              "service method names must be strictly increasing",
            );
          }
          previousName = raw;
          methods.push({ name, type: this.typeRef(path) });
        }
        return { kind: "service", methods };
      }
      default: {
        // A future type: self-describing length, skippable by construction.
        const length = this.lebU32(path, "future type length");
        this.raw(length, path);
        return { kind: "future" };
      }
    }
  }

  /** All references were validated after the table parse; lookup is total. */
  private entry(ref: number): WireEntry {
    return this.entries[ref];
  }

  /**
   * The record-field rule for a value the wire does not carry: `null` when
   * the expected type is opt-like (`opt`, `null`, `reserved`), a hard error
   * otherwise.
   */
  missingValue(schema: SchemaNode, path: PathSegment[], depth: number): unknown {
    const { node } = this.resolveSchema(schema, path, depth);
    if (
      node.kind === "opt" ||
      (node.kind === "primitive" && (node.primitive === "null" || node.primitive === "reserved"))
    ) {
      return null;
    }
    // A coercion failure, not a hard one: the spec's opt fallback rule
    // absorbs a record that cannot coerce (missing mandatory field
    // included) to null under an enclosing expected opt — the evolution
    // story for narrowing records. Top level converts it to a hard issue.
    this.mismatch("missing_field", path, "the wire carries no value for this field");
  }

  private resolveSchema(
    schema: SchemaNode,
    path: readonly PathSegment[],
    depth: number,
  ): { node: Exclude<SchemaNode, RecNode>; depth: number } {
    let node = schema;
    let hops = depth;
    while (node.kind === "rec") {
      hops += 1;
      this.step(path, hops);
      const body: unknown = node.body();
      if (
        typeof body !== "object" ||
        body === null ||
        typeof (body as { kind?: unknown }).kind !== "string"
      ) {
        this.fail("unsupported_schema", path, "a rec thunk did not produce a schema");
      }
      node = body as SchemaNode;
    }
    return { node: node as Exclude<SchemaNode, RecNode>, depth: hops };
  }

  /**
   * Decode one wire value at the expected schema, coercing per the spec.
   *
   * The walk keeps its work on an explicit stack of `DecodeFrame`s (issue
   * #192), so `maxDepth` is its only depth bound: a composite value is a
   * frame, a child value that completes at once is consumed on the spot, and
   * one that opens a frame of its own hands its value back through `result`
   * when that frame ends. Reads, charges, path pushes and pops and issues
   * happen in the order the recursive walk this replaces performed them.
   */
  valueAt(wire: number, schema: SchemaNode, path: PathSegment[], depth: number): unknown {
    const value = this.enterValue(wire, schema, path, depth);
    return value === PENDING ? this.run(path) : value;
  }

  /**
   * Run the frames on the stack to completion and answer the last value
   * delivered. This is where absorption lives: a `CoercionMismatch` thrown
   * anywhere above an expected `opt` that is decoding its constituent
   * unwinds the stack to that frame — the nearest one, as the recursive
   * walk's `try`/`catch` did — which then rewinds and skips (see
   * `resumeOpt`). With no such frame the mismatch propagates: at top level it
   * is the spec's hard error. Nothing unwound pops a path segment; the
   * absorbing frame truncates the path back to its own (issue #209).
   */
  private run(path: PathSegment[]): unknown {
    const stack = this.stack;
    for (;;) {
      try {
        while (stack.length > 0) {
          this.resume(stack[stack.length - 1], path);
        }
        return this.result;
      } catch (error) {
        if (!isCoercionMismatch(error)) {
          throw error;
        }
        let at = stack.length - 1;
        while (at >= 0) {
          const frame = stack[at];
          if (frame.kind === "opt" && frame.phase === "decoding") {
            break;
          }
          at -= 1;
        }
        if (at < 0) {
          throw error;
        }
        stack.length = at + 1;
        (stack[at] as OptFrame).phase = "absorbed";
      }
    }
  }

  /** Finish the value frame on top of the stack, handing `value` to its parent. */
  private deliver(value: unknown): void {
    this.stack.pop();
    this.result = value;
  }

  /**
   * Begin one value: resolve and charge it, then answer it at once (a leaf, a
   * reference, a blob) or push the frame that decodes it and answer
   * `PENDING`. Never calls itself: children start from `resume`.
   */
  private enterValue(
    wire: number,
    schema: SchemaNode,
    path: PathSegment[],
    depth: number,
  ): unknown {
    const { node, depth: at } = this.resolveSchema(schema, path, depth);
    this.step(path, at);

    // Expected reserved absorbs any wire value, consuming it.
    if (node.kind === "primitive" && node.primitive === "reserved") {
      this.stack.push({ kind: "reserved", wire, depth: at, started: false });
      return PENDING;
    }

    if (node.kind === "opt") {
      return this.optAt(wire, node, path, at);
    }

    if (node.kind === "func" || node.kind === "service") {
      return this.referenceAt(wire, node, path);
    }

    if (wire >= 0) {
      const entry = this.entry(wire);
      switch (entry.kind) {
        case "opt":
          // A wire opt at a non-opt expected type has no coercion rule.
          this.mismatch("type_mismatch", path, "wire opt at a non-opt expected type");
          break;
        case "vec":
          return this.vecAt(entry.inner, node, path, at);
        case "record":
          return this.recordAt(entry, node, path, at);
        case "variant":
          return this.variantAt(entry, node, path, at);
        case "func":
        case "service":
          this.mismatch(
            "type_mismatch",
            path,
            `a wire ${entry.kind} value at a non-reference expected type`,
          );
          break;
        case "future":
          this.mismatch("type_mismatch", path, "a wire future value has no expected counterpart");
      }
    }

    return this.primitiveAt(wire, node, path);
  }

  /** Advance the frame on top of the stack by one child, or finish it. */
  private resume(frame: DecodeFrame, path: PathSegment[]): void {
    switch (frame.kind) {
      case "opt":
        this.resumeOpt(frame, path);
        return;
      case "vec": {
        if (frame.awaiting) {
          frame.awaiting = false;
          frame.out.push(this.result);
          path.pop();
        }
        for (;;) {
          frame.index += 1;
          if (frame.index >= frame.length) {
            this.deliver(frame.out);
            return;
          }
          path.push(frame.index);
          const value = this.enterValue(
            frame.inner,
            frame.node.inner as SchemaNode,
            path,
            frame.depth + 1,
          );
          if (value === PENDING) {
            frame.awaiting = true;
            return;
          }
          frame.out.push(value);
          path.pop();
        }
      }
      case "record":
        this.resumeRecord(frame, path);
        return;
      case "variant": {
        if (!frame.started) {
          frame.started = true;
          if (!frame.tagOnly) {
            path.push("value");
          }
          const value = this.enterValue(frame.wire, frame.node, path, frame.depth);
          if (value === PENDING) {
            return;
          }
          this.result = value;
        }
        if (frame.tagOnly) {
          this.deliver({ tag: frame.tag });
          return;
        }
        const value = this.result;
        path.pop();
        this.deliver({ tag: frame.tag, value });
        return;
      }
      case "reserved": {
        if (!frame.started) {
          frame.started = true;
          if (!this.enterSkip(frame.wire, path, frame.depth)) {
            return;
          }
        }
        this.deliver(null);
        return;
      }
      case "skip-vec": {
        while (frame.remaining > 0) {
          frame.remaining -= 1;
          if (!this.enterSkip(frame.inner, path, frame.depth)) {
            return;
          }
        }
        this.stack.pop();
        return;
      }
      case "skip-record": {
        while (frame.index < frame.types.length) {
          const type = frame.types[frame.index];
          frame.index += 1;
          if (!this.enterSkip(type, path, frame.depth)) {
            return;
          }
        }
        this.stack.pop();
        return;
      }
    }
  }

  /**
   * The opportunistic opt rules: content mismatches coerce to null. A
   * present value is boxed as `{ some: v }` exactly when the resolved inner
   * node admits `null` — the rule validate and encode apply — so `None`,
   * `Some(None)`, and `Some(Some(x))` decode to three distinct values. The
   * coercion itself is untouched: a mismatch the outer opt absorbs is its
   * `None` (`null`), never `{ some: null }`.
   */
  private optAt(wire: number, node: OptNode, path: PathSegment[], depth: number): unknown {
    // Wire null and reserved carry zero bytes and mean null here.
    if (wire === OP.null || wire === OP.reserved) {
      return null;
    }
    let constituent = wire;
    if (wire >= 0 && this.entry(wire).kind === "opt") {
      const entry = this.entry(wire) as { kind: "opt"; inner: number };
      const tag = this.byte(path);
      if (tag === 0) {
        return null;
      }
      if (tag !== 1) {
        this.fail("invalid_tag_byte", path, "an opt value starts with 0 or 1");
      }
      constituent = entry.inner;
    }
    // Otherwise a non-nullable wire type auto-wraps: opt v when coercible,
    // else null. Either way the inner schema is resolved once here —
    // charging each rec hop as the constituent walk would have — so the
    // boxing decision reads the node the value is then decoded against.
    const inner = this.resolveSchema(node.inner as SchemaNode, path, depth + 1);
    this.stack.push({
      kind: "opt",
      wire: constituent,
      node: inner.node,
      depth: inner.depth,
      skipDepth: depth + 1,
      rewind: this.offset,
      pathLength: path.length,
      phase: "start",
    });
    return PENDING;
  }

  /**
   * An expected `opt` decoding its constituent, which is where coercion
   * failures are absorbed: the value bytes are consumed either way (rewind,
   * then skip), and the opt is `None` (`null`) — kept apart from a
   * constituent that legitimately decoded to `null`. Malformed input and
   * resource failures stay hard — absorption never hides a broken message.
   * `skipDepth` is the depth the constituent walk began at before its rec
   * hops were resolved, where a skip of the rewound bytes starts.
   */
  private resumeOpt(frame: OptFrame, path: PathSegment[]): void {
    switch (frame.phase) {
      case "start": {
        frame.phase = "decoding";
        const value = this.enterValue(frame.wire, frame.node, path, frame.depth);
        if (value === PENDING) {
          return;
        }
        this.result = value;
        break;
      }
      case "decoding":
        break;
      case "absorbed":
        // The cursor rewinds; the element charges do not. Refunding the
        // failed attempt would let nested opts multiply the traversal budget
        // by the absorption depth — charges are for work performed, and the
        // rewound walk performed it.
        this.offset = frame.rewind;
        // The abandoned descent's path segments go too (issue #209): the
        // unwinding popped none of them, so without this the skip below and
        // every later issue would be reported under the absorbed value's
        // stale segments. The skip reports at the opt's own path.
        path.length = frame.pathLength;
        frame.phase = "skipping";
        if (!this.enterSkip(frame.wire, path, frame.skipDepth)) {
          return;
        }
        this.deliver(null);
        return;
      case "skipping":
        this.deliver(null);
        return;
    }
    const decoded = this.result;
    this.deliver(admitsNull(frame.node) ? { some: decoded } : decoded);
  }

  private vecAt(
    inner: number,
    node: Exclude<SchemaNode, RecNode>,
    path: PathSegment[],
    depth: number,
  ): unknown {
    if (node.kind === "blob") {
      if (inner !== OP.nat8) {
        this.mismatch("type_mismatch", path, "blob is vec nat8 on the wire");
      }
      const length = this.lebU32(path, "blob length");
      // Every blob byte must exist in the input, so a declared length the
      // remaining bytes cannot satisfy is truncation — detected before the
      // budget loop, or a 13-byte header would burn min(length, maxElements)
      // charges that the vec nat8 path never pays (issue #128).
      if (this.offset + length > this.bytes.length) {
        this.fail("truncated", path, "unexpected end of input");
      }
      // One element charge per byte keeps blob and vec nat8 accounting equal.
      for (let i = 0; i < length; i += 1) {
        this.step(path, depth + 1);
      }
      return Uint8Array.from(this.raw(length, path));
    }
    if (node.kind !== "vec") {
      this.mismatch("type_mismatch", path, `wire vec at expected ${node.kind}`);
    }
    const length = this.lebU32(path, "vec length");
    // A wire `vec nat8` is the blob alias: every element is exactly one
    // byte, so a declared length beyond the remaining input is truncation
    // here too, diagnosed before any charging — the two schema shapes must
    // report the same public code for the same bytes (issue #128). Other
    // inners have no static width floor (`vec null` legally carries zero
    // bytes per element) and stay budget-guarded as before.
    if (inner === OP.nat8 && this.offset + length > this.bytes.length) {
      this.fail("truncated", path, "unexpected end of input");
    }
    this.stack.push({
      kind: "vec",
      inner,
      node,
      depth,
      length,
      index: -1,
      out: [],
      awaiting: false,
    });
    return PENDING;
  }

  private recordAt(
    entry: { readonly ids: readonly number[]; readonly types: readonly number[] },
    node: Exclude<SchemaNode, RecNode>,
    path: PathSegment[],
    depth: number,
  ): unknown {
    let expected: { key: string | number; id: number; schema: AnySchema }[];
    if (node.kind === "record") {
      expected = Object.keys(node.fields).map((key) => ({
        key,
        id: fieldIdOfKey(key),
        schema: node.fields[key],
      }));
    } else if (node.kind === "tuple") {
      expected = node.elements.map((schema, index) => ({ key: index, id: index, schema }));
    } else if (node.kind === "unit") {
      expected = [];
    } else {
      this.mismatch("type_mismatch", path, `wire record at expected ${node.kind}`);
    }
    expected.sort((a, b) => a.id - b.id);
    for (let i = 1; i < expected.length; i += 1) {
      if (expected[i].id === expected[i - 1].id) {
        this.fail(
          "duplicate_field_id",
          path,
          `keys ${JSON.stringify(String(expected[i - 1].key))} and ${JSON.stringify(
            String(expected[i].key),
          )} derive the same wire id ${expected[i].id}`,
        );
      }
    }
    this.stack.push({
      kind: "record",
      entry,
      node,
      expected,
      depth,
      out: Object.create(null) as Record<string | number, unknown>,
      cursor: 0,
      w: 0,
      awaiting: "none",
    });
    return PENDING;
  }

  /**
   * A wire record at an expected record, tuple or unit: wire fields in id
   * order, each decoded at its expected field or skipped; expected fields the
   * wire lacks follow the missing-field rule. `w` is the wire field in
   * progress and `cursor` the next expected field; `awaiting` says which kind
   * of child is in flight when the frame resumes.
   */
  private resumeRecord(frame: RecordFrame, path: PathSegment[]): void {
    const { entry, expected, out, depth } = frame;
    if (frame.awaiting === "field") {
      out[expected[frame.cursor].key] = this.result;
      path.pop();
      frame.cursor += 1;
      frame.w += 1;
    } else if (frame.awaiting === "skip") {
      frame.w += 1;
    }
    frame.awaiting = "none";
    for (; frame.w < entry.ids.length; frame.w += 1) {
      const w = frame.w;
      // Expected fields the wire skipped over, in id order.
      while (frame.cursor < expected.length && expected[frame.cursor].id < entry.ids[w]) {
        const field = expected[frame.cursor];
        path.push(field.key);
        out[field.key] = this.missingValue(field.schema as SchemaNode, path, depth + 1);
        path.pop();
        frame.cursor += 1;
      }
      if (frame.cursor < expected.length && expected[frame.cursor].id === entry.ids[w]) {
        const field = expected[frame.cursor];
        path.push(field.key);
        const value = this.enterValue(entry.types[w], field.schema as SchemaNode, path, depth + 1);
        if (value === PENDING) {
          frame.awaiting = "field";
          return;
        }
        out[field.key] = value;
        path.pop();
        frame.cursor += 1;
      } else if (!this.enterSkip(entry.types[w], path, depth + 1)) {
        // Present only on the wire: ignored, but its bytes must be walked.
        frame.awaiting = "skip";
        return;
      }
    }
    while (frame.cursor < expected.length) {
      const field = expected[frame.cursor];
      path.push(field.key);
      out[field.key] = this.missingValue(field.schema as SchemaNode, path, depth + 1);
      path.pop();
      frame.cursor += 1;
    }
    const { node } = frame;
    if (node.kind === "tuple") {
      const array: unknown[] = [];
      for (let i = 0; i < node.elements.length; i += 1) {
        array.push(out[i]);
      }
      this.deliver(array);
      return;
    }
    if (node.kind === "unit") {
      this.deliver({});
      return;
    }
    // Null-prototype construction mirrors contract.ts; hand the caller a
    // plain object so deepStrictEqual against literals behaves normally.
    this.deliver({ ...out });
  }

  private variantAt(
    entry: { readonly ids: readonly number[]; readonly types: readonly number[] },
    node: Exclude<SchemaNode, RecNode>,
    path: PathSegment[],
    depth: number,
  ): unknown {
    if (node.kind !== "variant") {
      this.mismatch("type_mismatch", path, `wire variant at expected ${node.kind}`);
    }
    const index = this.lebU32(path, "variant index");
    if (index >= entry.ids.length) {
      this.fail(
        "invalid_length",
        path,
        `variant index ${index} is outside the ${entry.ids.length}-arm wire type`,
      );
    }
    const wireId = entry.ids[index];
    // The same duplicate-derived-id refusal as encode and recordAt: two
    // arms deriving one wire id must fail closed, not dispatch to
    // whichever enumerates first.
    const arms = Object.keys(node.arms).map((key) => ({
      key,
      id: fieldIdOfKey(key),
    }));
    arms.sort((a, b) => a.id - b.id);
    for (let i = 1; i < arms.length; i += 1) {
      if (arms[i].id === arms[i - 1].id) {
        this.fail(
          "duplicate_field_id",
          path,
          `keys ${JSON.stringify(arms[i - 1].key)} and ${JSON.stringify(
            arms[i].key,
          )} derive the same wire id ${arms[i].id}`,
        );
      }
    }
    let match: { key: string; schema: AnySchema } | undefined;
    for (const key of Object.keys(node.arms)) {
      if (fieldIdOfKey(key) === wireId) {
        match = { key, schema: node.arms[key] };
        break;
      }
    }
    if (match === undefined) {
      // The spec's hard edge: variants trap where opts absorb — unless an
      // enclosing expected opt absorbs this mismatch.
      this.mismatch(
        "unknown_variant_tag",
        path,
        `wire tag id ${wireId} is not an arm of the expected variant`,
      );
    }
    const resolved = this.resolveSchema(match.schema as SchemaNode, path, depth);
    // A tag-only arm's payload still coerces at expected null — a wire arm
    // carrying a non-null payload there is a mismatch, not something to skip
    // over — and the value is the bare `{ tag }`.
    const tagOnly = resolved.node.kind === "primitive" && resolved.node.primitive === "null";
    this.stack.push({
      kind: "variant",
      tag: match.key,
      tagOnly,
      wire: entry.types[index],
      node: resolved.node,
      depth: resolved.depth + 1,
      started: false,
    });
    return PENDING;
  }

  /**
   * Decode a func/service *value* at an expected reference schema. The spec
   * makes references the one place deserialisation performs a real subtype
   * CHECK: the wire type must be a structural subtype of the expected type
   * (function params contravariant, results covariant, annotation sets
   * equal; service methods width-and-depth). A failed check is a coercion
   * mismatch — absorbable by an enclosing expected opt — while malformed
   * bytes stay hard errors. Opaque reference forms (tag 0) are refused, as
   * recorded for this codec slice.
   */
  private referenceAt(wire: number, node: FuncNode | ServiceNode, path: PathSegment[]): unknown {
    if (wire < 0) {
      this.mismatch("type_mismatch", path, `wire type ${wire} at an expected ${node.kind}`);
    }
    const entry = this.entry(wire);
    if (entry.kind !== node.kind) {
      this.mismatch("type_mismatch", path, `wire ${entry.kind} at an expected ${node.kind}`);
    }
    if (!this.wireSubtypeOfSchema(wire, node as unknown as AnySchema, this.subtypeMemo)) {
      this.mismatch(
        "type_mismatch",
        path,
        `the wire ${node.kind} type is not a subtype of the expected ${node.kind}`,
      );
    }
    const tag = this.byte(path);
    if (tag === 0) {
      this.fail("invalid_principal", path, "opaque references are unsupported in this slice");
    }
    if (tag !== 1) {
      this.fail("invalid_tag_byte", path, `a ${node.kind} value starts with 0 or 1`);
    }
    if (node.kind === "func") {
      const inner = this.byte(path);
      if (inner === 0) {
        this.fail("invalid_principal", path, "opaque references are unsupported in this slice");
      }
      if (inner !== 1) {
        this.fail("invalid_tag_byte", path, "a service value starts with 0 or 1");
      }
      const principal = this.principalBody(path);
      const length = this.lebU32(path, "method name length");
      const method = utf8Decode(this.raw(length, path));
      if (method === undefined) {
        this.fail("invalid_utf8", path, "method name is not well-formed UTF-8");
      }
      // Round-trip symmetry: validate and encode both refuse an empty
      // method name, so decode must not produce one.
      if (method.length === 0) {
        this.fail("invalid_length", path, "a method name is a non-empty string");
      }
      return { principal, method };
    }
    return this.principalBody(path);
  }

  private principalBody(path: readonly PathSegment[]): Principal {
    const length = this.lebU32(path, "principal length");
    if (length > 29) {
      this.fail("invalid_principal", path, "a principal id is at most 29 bytes");
    }
    // Canonical by construction: the rendering of an id of at most 29 bytes.
    return principalTextFromBytes(Uint8Array.from(this.raw(length, path))) as Principal;
  }

  /**
   * The static subtype relation between a wire type and an expected schema
   * (`wire <: schema`), with the mirrored direction for contravariant
   * positions. Coinductive: a pair under test is assumed true on revisit, so
   * recursive types terminate. Depth of schema rec unwrapping is bounded by
   * the depth limit; the pair memo bounds everything else.
   *
   * Evaluated on an explicit stack of `SubFrame`s (issue #192): a pair whose
   * answer needs its children's is a frame, and a child's answer flows back
   * to it — `false` ends the parent at once, as the short-circuiting `&&`
   * and early `return false` of the recursive relation did. Pairs are tested,
   * memoized and assumed in the same order, with the same schema reads.
   */
  private wireSubtypeOfSchema(
    wire: number,
    schema: AnySchema,
    seen: Map<string, boolean>,
  ): boolean {
    const first = this.subStart(wire, schema, true, seen, 0);
    if (typeof first === "boolean") {
      return first;
    }
    const stack: SubFrame[] = [first];
    outer: for (;;) {
      const frame = stack[stack.length - 1];
      let outcome = this.subNext(frame);
      while (typeof outcome !== "boolean") {
        const child = this.subStart(
          outcome.wire,
          outcome.schema,
          outcome.wireOnLeft,
          seen,
          frame.depth + 1,
        );
        if (typeof child !== "boolean") {
          stack.push(child);
          continue outer;
        }
        outcome = child ? this.subNext(frame) : false;
      }
      // The frame has its answer: memoize it and hand it down — a `false`
      // ends each waiting parent in turn, a `true` lets the parent continue.
      let value = outcome;
      let done = stack.pop() as SubFrame;
      for (;;) {
        seen.set(done.key, value);
        if (stack.length === 0) {
          return value;
        }
        if (value) {
          continue outer;
        }
        done = stack.pop() as SubFrame;
        value = false;
      }
    }
  }

  private schemaId(schema: object): number {
    let id = this.schemaIds.get(schema);
    if (id === undefined) {
      id = this.schemaIds.size;
      this.schemaIds.set(schema, id);
    }
    return id;
  }

  /**
   * One pair's entry into the relation: bounded by depth, answered from the
   * memo, or assumed true (the coinductive step) and returned as a frame for
   * `subNext` to decide.
   */
  private subStart(
    wire: number,
    schema: AnySchema,
    wireOnLeft: boolean,
    seen: Map<string, boolean>,
    depth: number,
  ): boolean | SubFrame {
    if (depth > this.reached) {
      this.reached = depth;
    }
    if (depth > this.limits.maxDepth) {
      return false;
    }
    // The key uses the UNRESOLVED schema's identity: rec thunks mint fresh
    // node objects on every body() call (that is how the generator emits
    // every declaration), but the rec object a structure references is
    // stable — the same anchor the encoder's table memo relies on. Keying
    // on the resolved node would never see a cycle revisit and the
    // coinductive assumption could not engage.
    const key = `${wire}|${this.schemaId(schema)}|${wireOnLeft ? "ws" : "sw"}`;
    const cached = seen.get(key);
    if (cached !== undefined) {
      return cached;
    }
    // Coinductive assumption for recursive pairs.
    seen.set(key, true);
    const node = this.resolveTypeNode(schema as SchemaNode);
    if (node === undefined) {
      seen.set(key, false);
      return false;
    }
    return { key, wire, node, wireOnLeft, depth, phase: 0, index: 0 };
  }

  /**
   * Rec resolution for the subtype relation: depth-guarded, never charged
   * against the value budgets — this is type-graph work, and charging it
   * turned an accept into a hard resource error on one cycle parity.
   * `undefined` (fail closed to "not a subtype") on garbage or over-deep
   * chains.
   */
  private resolveTypeNode(schema: SchemaNode): Exclude<SchemaNode, RecNode> | undefined {
    let node = schema;
    for (let hops = 0; hops <= this.limits.maxDepth; hops += 1) {
      if (node.kind !== "rec") {
        return node as Exclude<SchemaNode, RecNode>;
      }
      const body: unknown = node.body();
      if (
        typeof body !== "object" ||
        body === null ||
        typeof (body as { kind?: unknown }).kind !== "string"
      ) {
        return undefined;
      }
      node = body as SchemaNode;
    }
    return undefined;
  }

  /**
   * Advance one pair: its answer, or the next child pair it needs — called
   * again with the child's `true` (a child's `false` is the pair's answer
   * too, and never reaches here). `phase` 0 applies the universal and
   * primitive rules and sets the pair up; then `index` walks its obligations
   * (a func walks arguments in phase 1 and results in phase 2).
   */
  private subNext(frame: SubFrame): boolean | SubRequest {
    const { wire, node, wireOnLeft } = frame;
    const wireIsLeft = wireOnLeft;
    if (frame.phase === 0) {
      frame.phase = 1;
      // The universal rules first: empty on the left, reserved or opt on the
      // right — each true regardless of the other side.
      if (wireIsLeft && wire === OP.empty) {
        return true;
      }
      if (!wireIsLeft && node.kind === "primitive" && node.primitive === "empty") {
        return true;
      }
      if (wireIsLeft && node.kind === "primitive" && node.primitive === "reserved") {
        return true;
      }
      if (!wireIsLeft && wire === OP.reserved) {
        return true;
      }
      if (wireIsLeft && node.kind === "opt") {
        return true;
      }
      if (!wireIsLeft && wire >= 0 && this.entry(wire).kind === "opt") {
        return true;
      }
      if (!wireIsLeft && wire === OP.opt) {
        return true;
      }

      if (node.kind === "primitive") {
        const opcode = PRIMITIVE_OPCODES[node.primitive];
        if (opcode === undefined) {
          return false;
        }
        if (wire === opcode) {
          return true;
        }
        // nat <: int, in whichever direction has nat on the left.
        if (wireIsLeft) {
          return wire === OP.nat && opcode === OP.int;
        }
        return opcode === OP.nat && wire === OP.int;
      }

      if (wire < 0) {
        return false;
      }
      const entry = this.entry(wire);
      frame.entry = entry;
      switch (node.kind) {
        case "opt":
          // Schema opt on the left (schema <: wire): only opt <: opt depth
          // rule reaches here (wire opt on the right was handled above).
          return false;
        case "vec":
        case "blob":
          if (entry.kind !== "vec") {
            return false;
          }
          break;
        case "unit":
        case "record":
        case "tuple":
          if (entry.kind !== "record") {
            return false;
          }
          // The empty record (unit) is the zero-field case of the same width
          // rules: extra wire fields are fine on the wire-left side, and must
          // be opt-like on the schema-left side — never simply forbidden.
          frame.expected =
            node.kind === "record"
              ? Object.keys(node.fields).map((fieldKey) => ({
                  id: fieldIdOfKey(fieldKey),
                  schema: node.fields[fieldKey],
                }))
              : node.kind === "tuple"
                ? node.elements.map((element, index) => ({ id: index, schema: element }))
                : [];
          break;
        case "variant":
          if (entry.kind !== "variant") {
            return false;
          }
          frame.expected = Object.keys(node.arms).map((armKey) => ({
            id: fieldIdOfKey(armKey),
            schema: node.arms[armKey],
          }));
          break;
        case "func": {
          if (entry.kind !== "func") {
            return false;
          }
          const annotation = MODE_ANNOTATION[node.mode];
          if (annotation === undefined || entry.annotation !== annotation) {
            return false;
          }
          // func t <: t' — params contravariant (t'.args <: t.args as
          // tuples), results covariant (t.results <: t'.results), where a
          // tuple A <: B needs every B element present in A as a subtype or
          // opt-like; extra A elements are ignored. `wireOnLeft` decides
          // which side is t.
          frame.sub = wireOnLeft
            ? { args: entry.args, results: entry.results }
            : { args: node.args, results: node.results };
          frame.sup = wireOnLeft
            ? { args: node.args, results: node.results }
            : { args: entry.args, results: entry.results };
          break;
        }
        case "service":
          if (entry.kind !== "service") {
            return false;
          }
          if (wireIsLeft) {
            frame.names = Object.keys(node.methods);
          }
          break;
        default:
          return false;
      }
    }

    const entry = frame.entry as WireEntry;
    switch (node.kind) {
      case "vec":
        if (frame.index === 0) {
          frame.index = 1;
          return { wire: (entry as VecWire).inner, schema: node.inner, wireOnLeft };
        }
        return true;
      case "blob":
        // blob is vec nat8: same elementwise depth rule as vec, not exact
        // opcode equality (`vec empty <: vec nat8` holds covariantly).
        if (frame.index === 0) {
          frame.index = 1;
          return { wire: (entry as VecWire).inner, schema: NAT8_NODE as AnySchema, wireOnLeft };
        }
        return true;
      case "unit":
      case "record":
      case "tuple": {
        const fields = entry as FieldsWire;
        const expected = frame.expected as { id: number; schema: AnySchema }[];
        if (wireIsLeft) {
          // wire <: schema: every schema field present in the wire with a
          // subtype, or opt-like.
          while (frame.index < expected.length) {
            const field = expected[frame.index];
            frame.index += 1;
            const at = fields.ids.indexOf(field.id);
            if (at >= 0) {
              return { wire: fields.types[at], schema: field.schema, wireOnLeft: true };
            } else if (!this.optLikeSchema(field.schema)) {
              return false;
            }
          }
          return true;
        }
        // schema <: wire: every wire field present in the schema with a
        // subtype, or opt-like on the wire side.
        while (frame.index < fields.ids.length) {
          const i = frame.index;
          frame.index += 1;
          const match = expected.find((field) => field.id === fields.ids[i]);
          if (match !== undefined) {
            return { wire: fields.types[i], schema: match.schema, wireOnLeft: false };
          } else if (!this.optLikeWire(fields.types[i])) {
            return false;
          }
        }
        return true;
      }
      case "variant": {
        const fields = entry as FieldsWire;
        const arms = frame.expected as { id: number; schema: AnySchema }[];
        if (wireIsLeft) {
          // wire <: schema: every wire arm exists in the schema.
          while (frame.index < fields.ids.length) {
            const i = frame.index;
            frame.index += 1;
            const match = arms.find((arm) => arm.id === fields.ids[i]);
            if (match === undefined) {
              return false;
            }
            return { wire: fields.types[i], schema: match.schema, wireOnLeft: true };
          }
          return true;
        }
        // schema <: wire: every schema arm exists on the wire.
        while (frame.index < arms.length) {
          const arm = arms[frame.index];
          frame.index += 1;
          const at = fields.ids.indexOf(arm.id);
          if (at < 0) {
            return false;
          }
          return { wire: fields.types[at], schema: arm.schema, wireOnLeft: false };
        }
        return true;
      }
      case "func": {
        const func = entry as FuncWire;
        const sub = frame.sub as { args: readonly AnySchema[] | readonly number[] };
        const sup = frame.sup as { args: readonly unknown[]; results: readonly unknown[] };
        const subResults = (frame.sub as { results: readonly unknown[] }).results;
        if (frame.phase === 1) {
          // Params: iterate the SUBTYPE side's args (B of the tuple rule).
          while (frame.index < sub.args.length) {
            const i = frame.index;
            frame.index += 1;
            const supArg = sup.args[i];
            if (supArg !== undefined) {
              // t'.args[i] <: t.args[i] — the flipped direction.
              return wireOnLeft
                ? { wire: func.args[i], schema: node.args[i], wireOnLeft: false }
                : { wire: func.args[i], schema: node.args[i], wireOnLeft: true };
            }
            const optLike = wireOnLeft
              ? this.optLikeWire(func.args[i])
              : this.optLikeSchema(node.args[i]);
            if (!optLike) {
              return false;
            }
          }
          frame.phase = 2;
          frame.index = 0;
        }
        // Results: iterate the SUPERTYPE side's results (B of the rule).
        while (frame.index < sup.results.length) {
          const i = frame.index;
          frame.index += 1;
          const subResult = subResults[i];
          if (subResult !== undefined) {
            return wireOnLeft
              ? { wire: func.results[i], schema: node.results[i], wireOnLeft: true }
              : { wire: func.results[i], schema: node.results[i], wireOnLeft: false };
          }
          const optLike = wireOnLeft
            ? this.optLikeSchema(node.results[i])
            : this.optLikeWire(func.results[i]);
          if (!optLike) {
            return false;
          }
        }
        return true;
      }
      case "service": {
        const service = entry as ServiceWire;
        if (wireIsLeft) {
          // wire <: schema: every expected method exists on the wire with a
          // wire func subtype of the expected func.
          const names = frame.names as string[];
          while (frame.index < names.length) {
            const name = names[frame.index];
            frame.index += 1;
            const method = service.methods.find((candidate) => candidate.name === name);
            if (method === undefined) {
              return false;
            }
            return { wire: method.type, schema: node.methods[name], wireOnLeft: true };
          }
          return true;
        }
        // schema <: wire: every wire method exists in the schema.
        while (frame.index < service.methods.length) {
          const method = service.methods[frame.index];
          frame.index += 1;
          const schemaMethod = node.methods[method.name];
          if (schemaMethod === undefined || !hasOwn(node.methods, method.name)) {
            return false;
          }
          return { wire: method.type, schema: schemaMethod, wireOnLeft: false };
        }
        return true;
      }
      default:
        return false;
    }
  }

  private optLikeSchema(schema: AnySchema): boolean {
    const node = this.resolveTypeNode(schema as SchemaNode);
    return (
      node !== undefined &&
      (node.kind === "opt" ||
        (node.kind === "primitive" && (node.primitive === "null" || node.primitive === "reserved")))
    );
  }

  private optLikeWire(wire: number): boolean {
    if (wire === OP.null || wire === OP.reserved) {
      return true;
    }
    return wire >= 0 && this.entry(wire).kind === "opt";
  }

  private primitiveAt(
    wire: number,
    node: Exclude<SchemaNode, RecNode>,
    path: PathSegment[],
  ): unknown {
    if (node.kind !== "primitive") {
      this.mismatch("type_mismatch", path, `wire type ${wire} at expected ${node.kind}`);
    }
    const name = node.primitive;
    if (name === "empty") {
      // No value ever inhabits empty; whatever the wire claims, this
      // position cannot be filled.
      this.mismatch("type_mismatch", path, "no value inhabits empty");
    }
    const expectedOpcode = PRIMITIVE_OPCODES[name];
    if (expectedOpcode === undefined) {
      this.fail("unsupported_schema", path, `unknown primitive ${JSON.stringify(name)}`);
    }
    // The one primitive coercion: a wire nat is accepted at expected int.
    const natAtInt = name === "int" && wire === OP.nat;
    if (wire !== expectedOpcode && !natAtInt) {
      this.mismatch("type_mismatch", path, `wire type ${wire} at expected ${name}`);
    }
    switch (name) {
      case "null":
        return null;
      case "bool": {
        const byte = this.byte(path);
        if (byte > 1) {
          this.fail("invalid_tag_byte", path, "a bool is 0 or 1");
        }
        return byte === 1;
      }
      case "nat":
        return this.lebBig(path);
      case "int":
        return natAtInt ? this.lebBig(path) : this.slebBig(path);
      case "nat64":
      case "int64": {
        const raw = this.raw(8, path);
        let value = 0n;
        for (let i = 7; i >= 0; i -= 1) {
          value = (value << 8n) | BigInt(raw[i]);
        }
        if (name === "int64" && value > INT64_MAX) {
          value -= 1n << 64n;
        }
        return value;
      }
      case "nat8":
      case "nat16":
      case "nat32":
      case "int8":
      case "int16":
      case "int32": {
        const { bytes: width, signed } = FIXED_WIDTH[name];
        const raw = this.raw(width, path);
        let value = 0;
        for (let i = width - 1; i >= 0; i -= 1) {
          value = value * 256 + raw[i];
        }
        if (signed && value >= 2 ** (width * 8 - 1)) {
          value -= 2 ** (width * 8);
        }
        return value;
      }
      case "float32": {
        const raw = this.raw(4, path);
        const view = new DataView(new ArrayBuffer(4));
        for (let i = 0; i < 4; i += 1) {
          view.setUint8(i, raw[i]);
        }
        return view.getFloat32(0, true);
      }
      case "float64": {
        const raw = this.raw(8, path);
        const view = new DataView(new ArrayBuffer(8));
        for (let i = 0; i < 8; i += 1) {
          view.setUint8(i, raw[i]);
        }
        return view.getFloat64(0, true);
      }
      case "text": {
        const length = this.lebU32(path, "text length");
        const text = utf8Decode(this.raw(length, path));
        if (text === undefined) {
          this.fail("invalid_utf8", path, "text is not well-formed UTF-8");
        }
        return text;
      }
      case "principal": {
        const tag = this.byte(path);
        if (tag === 0) {
          this.fail(
            "invalid_principal",
            path,
            "opaque principal references are unsupported in this slice",
          );
        }
        if (tag !== 1) {
          this.fail("invalid_tag_byte", path, "a principal value starts with 0 or 1");
        }
        const length = this.lebU32(path, "principal length");
        if (length > 29) {
          this.fail("invalid_principal", path, "a principal id is at most 29 bytes");
        }
        // Canonical by construction: the rendering of an id of at most 29
        // bytes, delivered as the branded text itself.
        return principalTextFromBytes(Uint8Array.from(this.raw(length, path))) as Principal;
      }
      case "reserved":
        return null;
      default:
        this.fail("unsupported_schema", path, `unknown primitive ${JSON.stringify(name)}`);
    }
  }

  /** Walk one wire value without interpreting it, charging the budgets. */
  skipValue(wire: number, path: PathSegment[], depth: number): void {
    if (!this.enterSkip(wire, path, depth)) {
      this.run(path);
    }
  }

  /**
   * Begin skipping one wire value: true when it was skipped entirely, false
   * when a `skip-vec` or `skip-record` frame now finishes it (the caller
   * awaits that frame). An opt's and a variant's payload — tail calls in the
   * recursive walk — continue in this loop, so this never calls itself.
   */
  private enterSkip(wire: number, path: readonly PathSegment[], depth: number): boolean {
    for (;;) {
      this.step(path, depth);
      if (wire >= 0) {
        const entry = this.entry(wire);
        switch (entry.kind) {
          case "opt": {
            const tag = this.byte(path);
            if (tag === 1) {
              wire = entry.inner;
              depth += 1;
              continue;
            } else if (tag !== 0) {
              this.fail("invalid_tag_byte", path, "an opt value starts with 0 or 1");
            }
            return true;
          }
          case "vec": {
            const length = this.lebU32(path, "vec length");
            if (length === 0) {
              return true;
            }
            this.stack.push({
              kind: "skip-vec",
              inner: entry.inner,
              remaining: length,
              depth: depth + 1,
            });
            return false;
          }
          case "record": {
            if (entry.types.length === 0) {
              return true;
            }
            this.stack.push({
              kind: "skip-record",
              types: entry.types,
              index: 0,
              depth: depth + 1,
            });
            return false;
          }
          case "variant": {
            const index = this.lebU32(path, "variant index");
            if (index >= entry.types.length) {
              this.fail(
                "invalid_length",
                path,
                `variant index ${index} is outside the ${entry.types.length}-arm wire type`,
              );
            }
            wire = entry.types[index];
            depth += 1;
            continue;
          }
          case "func": {
            const tag = this.byte(path);
            if (tag === 0) {
              this.fail(
                "invalid_principal",
                path,
                "opaque references are unsupported in this slice",
              );
            }
            if (tag !== 1) {
              this.fail("invalid_tag_byte", path, "a func value starts with 0 or 1");
            }
            this.skipServiceValue(path);
            const length = this.lebU32(path, "method name length");
            if (utf8Decode(this.raw(length, path)) === undefined) {
              this.fail("invalid_utf8", path, "method name is not well-formed UTF-8");
            }
            return true;
          }
          case "service": {
            this.skipServiceValue(path);
            return true;
          }
          case "future": {
            // A future value: m bytes and n references, both self-described.
            const byteCount = this.lebU32(path, "future value byte count");
            const refCount = this.lebU32(path, "future value reference count");
            this.raw(byteCount, path);
            if (refCount > 0) {
              this.fail(
                "invalid_principal",
                path,
                "opaque references are unsupported in this slice",
              );
            }
            return true;
          }
        }
      }
      switch (wire) {
        case OP.null:
        case OP.reserved:
          return true;
        case OP.bool: {
          const byte = this.byte(path);
          if (byte > 1) {
            this.fail("invalid_tag_byte", path, "a bool is 0 or 1");
          }
          return true;
        }
        case OP.nat:
          this.lebBig(path);
          return true;
        case OP.int:
          this.slebBig(path);
          return true;
        case OP.nat8:
        case OP.int8:
          this.raw(1, path);
          return true;
        case OP.nat16:
        case OP.int16:
          this.raw(2, path);
          return true;
        case OP.nat32:
        case OP.int32:
        case OP.float32:
          this.raw(4, path);
          return true;
        case OP.nat64:
        case OP.int64:
        case OP.float64:
          this.raw(8, path);
          return true;
        case OP.text: {
          const length = this.lebU32(path, "text length");
          if (utf8Decode(this.raw(length, path)) === undefined) {
            this.fail("invalid_utf8", path, "text is not well-formed UTF-8");
          }
          return true;
        }
        case OP.principal: {
          const tag = this.byte(path);
          if (tag === 0) {
            this.fail(
              "invalid_principal",
              path,
              "opaque principal references are unsupported in this slice",
            );
          }
          if (tag !== 1) {
            this.fail("invalid_tag_byte", path, "a principal value starts with 0 or 1");
          }
          const length = this.lebU32(path, "principal length");
          // Skipping is not a validation exemption: the reference rejects an
          // over-long principal id wherever it appears.
          if (length > 29) {
            this.fail("invalid_principal", path, "a principal id is at most 29 bytes");
          }
          this.raw(length, path);
          return true;
        }
        case OP.empty:
          this.fail("type_mismatch", path, "no value inhabits empty");
          return true;
        default:
          this.fail("malformed_type_table", path, `unknown wire type ${wire}`);
      }
    }
  }

  private skipServiceValue(path: readonly PathSegment[]): void {
    const tag = this.byte(path);
    if (tag === 0) {
      this.fail("invalid_principal", path, "opaque references are unsupported in this slice");
    }
    if (tag !== 1) {
      this.fail("invalid_tag_byte", path, "a service value starts with 0 or 1");
    }
    const length = this.lebU32(path, "service id length");
    if (length > 29) {
      this.fail("invalid_principal", path, "a principal id is at most 29 bytes");
    }
    this.raw(length, path);
  }
}
