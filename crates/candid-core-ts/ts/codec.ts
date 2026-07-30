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
//   of CRC-32 ‖ bytes, dash-grouped by five) is implemented here — encode
//   parses `toText()` and refuses non-canonical text (`invalid_principal`),
//   decode returns a minimal `{ toText }` carrier of the canonical text.
//   Opaque reference forms (tag byte 0) and any external reference sequence
//   are unsupported in this slice: decode fails closed, and input with bytes
//   remaining after the value section is refused (`trailing_bytes`).
// - Encode performs its own complete validation walk — the checks mirror
//   `validate`, but reading each property exactly once, so a hostile getter
//   cannot pass validation and then hand the emitter a different value.
//
// # Bounded, fail-closed, no exceptions for control flow
//
// Nothing here throws on any input: hostile values produce issues via the
// same choke-point pattern as `validate` (`unreadable_value`), and malformed
// or adversarial bytes produce issues with explicit budgets in candid-core's
// `Limits` spirit: `maxBytes` caps input size up front, `maxTypeTableEntries`
// mirrors `max_type_nodes`, `maxDepth`/`maxElements` mirror the validate
// walk (every decoded element, skipped value, and rec hop charges the same
// budget — a zero-byte-per-element wire vector cannot decode more than
// `maxElements` values), and `maxNumericBytes` caps a single unbounded
// `nat`/`int` encoding. Decode stops at the first hard error: the wire
// format cannot be resynchronized after one, so the issue list is short by
// design. Determinism: identical inputs produce identical bytes, values, and
// issues — the type table is built in first-visit order over the schema
// graph and nothing depends on time, environment, or map iteration order.

import type { AnySchema, Schema } from "./schema.ts";
import {
  candidLabelHash,
  fieldIdOfKey,
  utf8BytesStrict,
  utf8Decode,
} from "./labels.ts";

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

export interface CodecResourceLimitInfo {
  readonly resource:
    | "bytes"
    | "type_table_entries"
    | "value_depth"
    | "value_elements"
    | "numeric_bytes";
  readonly limit: number;
  readonly observed: number;
}

export interface CodecIssue {
  readonly code: CodecCode;
  /** `$`-rooted path into the argument sequence, `$args[i]` per argument. */
  readonly path: string;
  readonly message: string;
  readonly resource_limit?: CodecResourceLimitInfo;
}

export type EncodeResult =
  | { readonly ok: true; readonly bytes: Uint8Array }
  | { readonly ok: false; readonly issues: readonly CodecIssue[] };

export type DecodeResult =
  | { readonly ok: true; readonly values: readonly unknown[] }
  | { readonly ok: false; readonly issues: readonly CodecIssue[] };

export interface CodecOptions {
  /** Input size ceiling for decode, in bytes. */
  readonly maxBytes?: number;
  /** Wire type table entry cap, mirroring `Limits::max_type_nodes`. */
  readonly maxTypeTableEntries?: number;
  /** Traversal depth cap, mirroring `Limits::max_value_depth`. */
  readonly maxDepth?: number;
  /** Traversal element budget, mirroring validate's accounting. */
  readonly maxElements?: number;
  /** Byte cap for one unbounded `nat`/`int` encoding. */
  readonly maxNumericBytes?: number;
}

export const DEFAULT_MAX_BYTES = 10_485_760;
export const DEFAULT_MAX_TYPE_TABLE_ENTRIES = 100_000;
export const DEFAULT_MAX_DEPTH = 256;
export const DEFAULT_MAX_ELEMENTS = 1_000_000;
export const DEFAULT_MAX_NUMERIC_BYTES = 1_048_576;

/** A decoded principal: the minimal structural carrier of canonical text. */
export interface DecodedPrincipal {
  toText(): string;
}

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
type SchemaNode =
  | PrimitiveNode
  | OptNode
  | VecNode
  | BlobNode
  | UnitNode
  | RecordNode
  | TupleNode
  | VariantNode
  | RecNode;

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

/** A checked halt: issues past this point must not be recorded. */
class Halt extends Error {}

interface Limits {
  readonly maxBytes: number;
  readonly maxTypeTableEntries: number;
  readonly maxDepth: number;
  readonly maxElements: number;
  readonly maxNumericBytes: number;
}

function limitsOf(options: CodecOptions): Limits {
  return {
    maxBytes: options.maxBytes ?? DEFAULT_MAX_BYTES,
    maxTypeTableEntries:
      options.maxTypeTableEntries ?? DEFAULT_MAX_TYPE_TABLE_ENTRIES,
    maxDepth: options.maxDepth ?? DEFAULT_MAX_DEPTH,
    maxElements: options.maxElements ?? DEFAULT_MAX_ELEMENTS,
    maxNumericBytes: options.maxNumericBytes ?? DEFAULT_MAX_NUMERIC_BYTES,
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
// Principal text form
// ---------------------------------------------------------------------------

const BASE32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

const CRC32_TABLE: Uint32Array = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc = CRC32_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** Canonical principal text for raw id bytes. */
export function principalTextFromBytes(bytes: Uint8Array): string {
  const checksum = crc32(bytes);
  const data = new Uint8Array(4 + bytes.length);
  data[0] = (checksum >>> 24) & 0xff;
  data[1] = (checksum >>> 16) & 0xff;
  data[2] = (checksum >>> 8) & 0xff;
  data[3] = checksum & 0xff;
  data.set(bytes, 4);
  let encoded = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of data) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      encoded += BASE32_ALPHABET[(buffer >> bits) & 31];
    }
  }
  if (bits > 0) {
    encoded += BASE32_ALPHABET[(buffer << (5 - bits)) & 31];
  }
  let grouped = "";
  for (let i = 0; i < encoded.length; i += 5) {
    grouped += (i > 0 ? "-" : "") + encoded.slice(i, i + 5);
  }
  return grouped;
}

/**
 * Raw id bytes of canonical principal text, or `undefined` when the text is
 * not canonical: wrong alphabet or grouping, checksum mismatch, an id longer
 * than the 29-byte IC maximum, or any re-rendering difference (case
 * included) — encode fails closed rather than guessing.
 */
export function principalBytesFromText(text: string): Uint8Array | undefined {
  const compact = text.split("-").join("");
  let buffer = 0;
  let bits = 0;
  const data: number[] = [];
  for (const char of compact) {
    const index = BASE32_ALPHABET.indexOf(char);
    if (index < 0) {
      return undefined;
    }
    buffer = (buffer << 5) | index;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      data.push((buffer >> bits) & 0xff);
    }
  }
  if (data.length < 4 || data.length > 33) {
    return undefined;
  }
  const bytes = Uint8Array.from(data.slice(4));
  return principalTextFromBytes(bytes) === text ? bytes : undefined;
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

/** Encode one value; the message is the one-argument sequence. */
export function encode<T>(
  schema: Schema<T>,
  value: unknown,
  options: CodecOptions = {},
): EncodeResult {
  const result = encodeArgs([schema as AnySchema], [value], options);
  return result.ok ? result : { ok: false, issues: rerootIssues(result.issues) };
}

/** Encode an argument sequence to Candid wire bytes. */
export function encodeArgs(
  schemas: readonly AnySchema[],
  values: readonly unknown[],
  options: CodecOptions = {},
): EncodeResult {
  const encoder = new Encoder(limitsOf(options));
  const path: PathSegment[] = [];
  try {
    if (schemas.length !== values.length) {
      encoder.fail(
        "invalid_length",
        path,
        `${schemas.length} schemas for ${values.length} values`,
      );
    }
    const typeRefs = schemas.map((schema, index) => {
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
    // Element-by-element appends, never spreads: `push(...big)` routes the
    // whole array through the engine's argument list and throws RangeError
    // past ~124k elements — a well-formed large message, not a hostile one.
    const out: number[] = [0x44, 0x49, 0x44, 0x4c];
    writeLebNumber(out, encoder.table.length);
    for (const entry of encoder.table) {
      for (const byte of entry) {
        out.push(byte);
      }
    }
    writeLebNumber(out, typeRefs.length);
    for (const ref of typeRefs) {
      writeSlebBig(out, BigInt(ref));
    }
    for (const byte of body) {
      out.push(byte);
    }
    return { ok: true, bytes: Uint8Array.from(out) };
  } catch (error) {
    if (!(error instanceof Halt)) {
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

class Encoder {
  readonly issues: CodecIssue[] = [];
  /** Serialized table entries, in first-visit order. */
  readonly table: number[][] = [];
  private readonly memo = new Map<SchemaNode, number>();
  private elements = 0;

  private readonly limits: Limits;

  constructor(limits: Limits) {
    this.limits = limits;
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
        this.fail(
          "unsupported_schema",
          path,
          "a rec thunk did not produce a schema",
        );
      }
      node = body as SchemaNode;
    }
    return { node: node as Exclude<SchemaNode, RecNode>, depth: hops };
  }

  /**
   * Rec unwrapping for type-table construction: depth-guarded but never
   * element-charged. The table is type-graph work bounded by its own entry
   * cap; charging it against `maxElements` made encode's element accounting
   * diverge from validate's value-walk accounting.
   */
  private resolveType(
    schema: SchemaNode,
    path: readonly PathSegment[],
    depth: number,
  ): { node: Exclude<SchemaNode, RecNode>; depth: number } {
    let node = schema;
    let hops = depth;
    while (node.kind === "rec") {
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
        this.fail(
          "unsupported_schema",
          path,
          "a rec thunk did not produce a schema",
        );
      }
      node = body as SchemaNode;
    }
    return { node: node as Exclude<SchemaNode, RecNode>, depth: hops };
  }

  /**
   * The wire type reference for a schema: a negative primitive opcode or a
   * type-table index. Entries are memoized by node identity — including the
   * *unresolved* rec node, which is the stable anchor: a generated
   * `c.rec(() => …)` body builds fresh combinator objects on every call, so
   * memoizing only the resolved node would never see the cycle and the
   * table would grow until the entry cap. Memoizing the rec object itself
   * (reserved before recursion) is what ties the knot.
   */
  typeRef(schema: SchemaNode, path: readonly PathSegment[], depth: number): number {
    const known = this.memo.get(schema);
    if (known !== undefined) {
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
      this.memo.set(schema, existing);
      return existing;
    }
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
    this.table.push([]);
    const entry: number[] = [];
    switch (node.kind) {
      case "opt": {
        writeSlebBig(entry, BigInt(OP.opt));
        writeSlebBig(entry, BigInt(this.typeRef(node.inner as SchemaNode, path, at + 1)));
        break;
      }
      case "vec": {
        writeSlebBig(entry, BigInt(OP.vec));
        writeSlebBig(entry, BigInt(this.typeRef(node.inner as SchemaNode, path, at + 1)));
        break;
      }
      case "blob": {
        writeSlebBig(entry, BigInt(OP.vec));
        writeSlebBig(entry, BigInt(OP.nat8));
        break;
      }
      case "unit": {
        writeSlebBig(entry, BigInt(OP.record));
        writeLebNumber(entry, 0);
        break;
      }
      case "tuple": {
        writeSlebBig(entry, BigInt(OP.record));
        writeLebNumber(entry, node.elements.length);
        for (let i = 0; i < node.elements.length; i += 1) {
          writeLebNumber(entry, i);
          writeSlebBig(
            entry,
            BigInt(this.typeRef(node.elements[i] as SchemaNode, path, at + 1)),
          );
        }
        break;
      }
      case "record":
      case "variant": {
        const map = node.kind === "record" ? node.fields : node.arms;
        const fields = this.sortedFields(map, path);
        writeSlebBig(entry, BigInt(node.kind === "record" ? OP.record : OP.variant));
        writeLebNumber(entry, fields.length);
        for (const field of fields) {
          writeLebNumber(entry, field.id);
          writeSlebBig(
            entry,
            BigInt(this.typeRef(field.schema as SchemaNode, path, at + 1)),
          );
        }
        break;
      }
    }
    this.table[index] = entry;
    return index;
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
   */
  value(
    schema: SchemaNode,
    value: unknown,
    out: number[],
    path: PathSegment[],
    depth: number,
  ): void {
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
        out.push(1);
        this.value(node.inner as SchemaNode, value, out, path, at + 1);
        return;
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
        for (let i = 0; i < length; i += 1) {
          path.push(i);
          this.value(node.inner as SchemaNode, value[i], out, path, at + 1);
          path.pop();
        }
        return;
      }
      case "blob": {
        if (!isUint8Array(value)) {
          this.fail(
            "invalid_type",
            path,
            `expected a Uint8Array, got ${describe(value)}`,
          );
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
          this.fail(
            "invalid_type",
            path,
            `expected an empty record, got ${describe(value)}`,
          );
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
        for (let i = 0; i < node.elements.length; i += 1) {
          path.push(i);
          this.value(node.elements[i] as SchemaNode, value[i], out, path, at + 1);
          path.pop();
        }
        return;
      }
      case "record": {
        if (!this.isPlainCandidate(value)) {
          this.fail("invalid_type", path, `expected a record, got ${describe(value)}`);
        }
        const fields = this.sortedFields(node.fields, path);
        // Validate and read in declaration order — the order validate
        // reports in, so the first issue's path agrees — buffering each
        // field's bytes; the wire wants ascending-id order, so emission
        // reorders the buffers, never the reads.
        const buffers = new Map<string, number[]>();
        for (const key of Object.keys(node.fields)) {
          path.push(key);
          if (!hasOwnEnumerable(value, key)) {
            this.fail("missing_field", path, "required field is missing");
          }
          const buffer: number[] = [];
          this.value(
            node.fields[key] as SchemaNode,
            (value as Record<string, unknown>)[key],
            buffer,
            path,
            at + 1,
          );
          buffers.set(key, buffer);
          path.pop();
        }
        for (const field of fields) {
          for (const byte of buffers.get(field.key) as number[]) {
            out.push(byte);
          }
        }
        for (const key of Object.keys(value)) {
          this.step(path, at);
          if (!hasOwn(node.fields, key)) {
            path.push(key);
            this.fail("unexpected_field", path, "field is not part of this record");
          }
        }
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
          this.fail(
            "unknown_tag",
            path,
            `${JSON.stringify(tag)} is not an arm of this variant`,
          );
        }
        path.pop();
        const arms = this.sortedFields(node.arms, path);
        const index = arms.findIndex((arm) => arm.key === tag);
        writeLebNumber(out, index);
        const resolved = this.resolve(arms[index].schema as SchemaNode, path, at);
        const tagOnly =
          resolved.node.kind === "primitive" && resolved.node.primitive === "null";
        if (tagOnly) {
          // Mirror validate's walk exactly: the first non-tag key in
          // enumeration order is the issue, whatever its name.
          for (const key of Object.keys(value)) {
            this.step(path, at);
            if (key !== "tag") {
              path.push(key);
              this.fail(
                "unexpected_field",
                path,
                "a null-payload arm is a bare { tag }",
              );
            }
          }
          return;
        }
        path.push("value");
        if (!hasOwnEnumerable(value, "value")) {
          this.fail("missing_field", path, "this arm carries a payload");
        }
        this.value(
          resolved.node,
          (value as { value?: unknown }).value,
          out,
          path,
          resolved.depth + 1,
        );
        path.pop();
        for (const key of Object.keys(value)) {
          this.step(path, at);
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
    const image =
      value >= 0n ? value : value + (1n << BigInt(7 * groups));
    writeLebGroups(out, image, groups);
  }

  private primitive(
    name: string,
    value: unknown,
    out: number[],
    path: PathSegment[],
  ): void {
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
        const [min, max] =
          name === "nat64" ? [0n, NAT64_MAX] : [INT64_MIN, INT64_MAX];
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
        if (
          typeof value !== "object" ||
          value === null ||
          typeof (value as { toText?: unknown }).toText !== "function"
        ) {
          this.fail(
            "invalid_type",
            path,
            `expected a principal with toText(), got ${describe(value)}`,
          );
        }
        const text: unknown = (value as { toText(): unknown }).toText();
        if (typeof text !== "string") {
          this.fail("invalid_principal", path, "toText() did not return a string");
        }
        const bytes = principalBytesFromText(text);
        if (bytes === undefined) {
          this.fail(
            "invalid_principal",
            path,
            "toText() is not canonical principal text",
          );
        }
        out.push(1);
        writeLebNumber(out, bytes.length);
        for (const byte of bytes) {
          out.push(byte);
        }
        return;
      }
      default:
        this.fail(
          "unsupported_schema",
          path,
          `unknown primitive ${JSON.stringify(name)}`,
        );
    }
  }

  private isPlainCandidate(value: unknown): value is Record<string, unknown> {
    return (
      typeof value === "object" &&
      value !== null &&
      !Array.isArray(value) &&
      !isUint8Array(value)
    );
  }
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/** Decode a one-argument message against one schema. */
export function decode<T>(
  schema: Schema<T>,
  bytes: Uint8Array,
  options: CodecOptions = {},
): { ok: true; value: unknown } | { ok: false; issues: readonly CodecIssue[] } {
  const result = decodeArgs([schema as AnySchema], bytes, options);
  return result.ok
    ? { ok: true, value: result.values[0] }
    : { ok: false, issues: rerootIssues(result.issues) };
}

/** Decode a Candid message against an expected argument schema sequence. */
export function decodeArgs(
  schemas: readonly AnySchema[],
  bytes: Uint8Array,
  options: CodecOptions = {},
): DecodeResult {
  const limits = limitsOf(options);
  const decoder = new Decoder(bytes, limits);
  const path: PathSegment[] = [];
  try {
    if (!isUint8Array(bytes)) {
      decoder.fail("invalid_type", path, `expected a Uint8Array, got ${describe(bytes)}`);
    }
    if (bytes.length > limits.maxBytes) {
      decoder.fail(
        "resource_limit_exceeded",
        path,
        `bytes limit ${limits.maxBytes} exceeded`,
        { resource: "bytes", limit: limits.maxBytes, observed: bytes.length },
      );
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
    if (error instanceof CoercionMismatch) {
      // A type-level mismatch with no enclosing expected `opt` to absorb it
      // is the spec's hard error.
      decoder.issues.push({
        code: error.code,
        path: error.path,
        message: error.detail,
      });
    } else if (!(error instanceof Halt)) {
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
  }
}

type WireEntry =
  | { readonly kind: "opt" | "vec"; readonly inner: number }
  | {
      readonly kind: "record" | "variant";
      readonly ids: readonly number[];
      readonly types: readonly number[];
    }
  | { readonly kind: "func" | "service"; readonly types: readonly number[] }
  | { readonly kind: "future" };

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
  private offset = 0;
  private elements = 0;

  private readonly bytes: Uint8Array;
  private readonly limits: Limits;

  constructor(bytes: Uint8Array, limits: Limits) {
    this.bytes = bytes;
    this.limits = limits;
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
      if (
        (last === 0x00 && (prior & 0x40) === 0) ||
        (last === 0x7f && (prior & 0x40) !== 0)
      ) {
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
      } else if (
        entry.kind === "record" ||
        entry.kind === "variant" ||
        entry.kind === "func" ||
        entry.kind === "service"
      ) {
        for (const type of entry.types) {
          this.checkRef(type, path);
          // A service method type must denote a function type — the one
          // structural constraint the spec states about reference types.
          if (
            entry.kind === "service" &&
            (type < 0 || this.entries[type].kind !== "func")
          ) {
            this.fail(
              "malformed_type_table",
              path,
              "a service method must denote a function type",
            );
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
      this.fail(
        "malformed_type_table",
        path,
        "the type table may only contain composite types",
      );
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
            this.fail(
              "malformed_type_table",
              path,
              "field ids must be strictly increasing",
            );
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
        const types: number[] = [];
        const args = this.lebU32(path, "func argument count");
        for (let i = 0; i < args; i += 1) {
          this.step(path, 0);
          types.push(this.typeRef(path));
        }
        const results = this.lebU32(path, "func result count");
        for (let i = 0; i < results; i += 1) {
          this.step(path, 0);
          types.push(this.typeRef(path));
        }
        const annotations = this.lebU32(path, "func annotation count");
        // The reference implementation refuses more than one annotation;
        // mirror it — fail-closed parity on headers, not just values.
        if (annotations > 1) {
          this.fail(
            "malformed_type_table",
            path,
            "a function type carries at most one annotation",
          );
        }
        for (let i = 0; i < annotations; i += 1) {
          const annotation = this.byte(path);
          if (annotation < 1 || annotation > 3) {
            this.fail("malformed_type_table", path, "unknown function annotation");
          }
        }
        return { kind: "func", types };
      }
      case OP.service: {
        const types: number[] = [];
        const methods = this.lebU32(path, "service method count");
        let previousName: Uint8Array | undefined;
        for (let i = 0; i < methods; i += 1) {
          this.step(path, 0);
          const length = this.lebU32(path, "method name length");
          const raw = Uint8Array.from(this.raw(length, path));
          if (utf8Decode(raw) === undefined) {
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
          types.push(this.typeRef(path));
        }
        return { kind: "service", types };
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
      (node.kind === "primitive" &&
        (node.primitive === "null" || node.primitive === "reserved"))
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

  /** Decode one wire value at the expected schema, coercing per the spec. */
  valueAt(
    wire: number,
    schema: SchemaNode,
    path: PathSegment[],
    depth: number,
  ): unknown {
    const { node, depth: at } = this.resolveSchema(schema, path, depth);
    this.step(path, at);

    // Expected reserved absorbs any wire value, consuming it.
    if (node.kind === "primitive" && node.primitive === "reserved") {
      this.skipValue(wire, path, at);
      return null;
    }

    if (node.kind === "opt") {
      return this.optAt(wire, node, path, at);
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
        case "future":
          this.mismatch(
            "type_mismatch",
            path,
            `a wire ${entry.kind} value has no expected counterpart in this slice`,
          );
      }
    }

    return this.primitiveAt(wire, node, path);
  }

  /** The opportunistic opt rules: content mismatches coerce to null. */
  private optAt(wire: number, node: OptNode, path: PathSegment[], depth: number): unknown {
    // Wire null and reserved carry zero bytes and mean null here.
    if (wire === OP.null || wire === OP.reserved) {
      return null;
    }
    if (wire >= 0 && this.entry(wire).kind === "opt") {
      const entry = this.entry(wire) as { kind: "opt"; inner: number };
      const tag = this.byte(path);
      if (tag === 0) {
        return null;
      }
      if (tag !== 1) {
        this.fail("invalid_tag_byte", path, "an opt value starts with 0 or 1");
      }
      return this.absorbing(entry.inner, node.inner as SchemaNode, path, depth + 1);
    }
    // A non-nullable wire type auto-wraps: opt v when coercible, else null.
    return this.absorbing(wire, node.inner as SchemaNode, path, depth + 1);
  }

  /**
   * Decode the constituent, absorbing *coercion* failures to null: the value
   * bytes are consumed either way (rewind, then skip). Malformed input and
   * resource failures stay hard — absorption never hides a broken message.
   */
  private absorbing(
    wire: number,
    schema: SchemaNode,
    path: PathSegment[],
    depth: number,
  ): unknown {
    const rewind = this.offset;
    try {
      return this.valueAt(wire, schema, path, depth);
    } catch (error) {
      if (!(error instanceof CoercionMismatch)) {
        throw error;
      }
      // The cursor rewinds; the element charges do not. Refunding the
      // failed attempt would let nested opts multiply the traversal budget
      // by the absorption depth — charges are for work performed, and the
      // rewound walk performed it.
      this.offset = rewind;
      this.skipValue(wire, path, depth);
      return null;
    }
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
    const out: unknown[] = [];
    for (let i = 0; i < length; i += 1) {
      path.push(i);
      out.push(this.valueAt(inner, node.inner as SchemaNode, path, depth + 1));
      path.pop();
    }
    return out;
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
    const out: Record<string | number, unknown> = Object.create(null);
    let cursor = 0;
    for (let w = 0; w < entry.ids.length; w += 1) {
      // Expected fields the wire skipped over, in id order.
      while (cursor < expected.length && expected[cursor].id < entry.ids[w]) {
        const field = expected[cursor];
        path.push(field.key);
        out[field.key] = this.missingValue(field.schema as SchemaNode, path, depth + 1);
        path.pop();
        cursor += 1;
      }
      if (cursor < expected.length && expected[cursor].id === entry.ids[w]) {
        const field = expected[cursor];
        path.push(field.key);
        out[field.key] = this.valueAt(
          entry.types[w],
          field.schema as SchemaNode,
          path,
          depth + 1,
        );
        path.pop();
        cursor += 1;
      } else {
        // Present only on the wire: ignored, but its bytes must be walked.
        this.skipValue(entry.types[w], path, depth + 1);
      }
    }
    while (cursor < expected.length) {
      const field = expected[cursor];
      path.push(field.key);
      out[field.key] = this.missingValue(field.schema as SchemaNode, path, depth + 1);
      path.pop();
      cursor += 1;
    }
    if (node.kind === "tuple") {
      const array: unknown[] = [];
      for (let i = 0; i < node.elements.length; i += 1) {
        array.push(out[i]);
      }
      return array;
    }
    if (node.kind === "unit") {
      return {};
    }
    // Null-prototype construction mirrors contract.ts; hand the caller a
    // plain object so deepStrictEqual against literals behaves normally.
    return { ...out };
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
    const tagOnly =
      resolved.node.kind === "primitive" && resolved.node.primitive === "null";
    if (tagOnly) {
      // The payload still coerces at expected null — a wire arm carrying a
      // non-null payload here is a mismatch, not something to skip over.
      this.valueAt(entry.types[index], resolved.node, path, resolved.depth + 1);
      return { tag: match.key };
    }
    path.push("value");
    const value = this.valueAt(entry.types[index], resolved.node, path, resolved.depth + 1);
    path.pop();
    return { tag: match.key, value };
  }

  private primitiveAt(
    wire: number,
    node: Exclude<SchemaNode, RecNode>,
    path: PathSegment[],
  ): unknown {
    if (node.kind !== "primitive") {
      this.mismatch(
        "type_mismatch",
        path,
        `wire type ${wire} at expected ${node.kind}`,
      );
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
      this.mismatch(
        "type_mismatch",
        path,
        `wire type ${wire} at expected ${name}`,
      );
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
        const text = principalTextFromBytes(Uint8Array.from(this.raw(length, path)));
        return { toText: () => text } satisfies DecodedPrincipal;
      }
      case "reserved":
        return null;
      default:
        this.fail("unsupported_schema", path, `unknown primitive ${JSON.stringify(name)}`);
    }
  }

  /** Walk one wire value without interpreting it, charging the budgets. */
  skipValue(wire: number, path: readonly PathSegment[], depth: number): void {
    this.step(path, depth);
    if (wire >= 0) {
      const entry = this.entry(wire);
      switch (entry.kind) {
        case "opt": {
          const tag = this.byte(path);
          if (tag === 1) {
            this.skipValue(entry.inner, path, depth + 1);
          } else if (tag !== 0) {
            this.fail("invalid_tag_byte", path, "an opt value starts with 0 or 1");
          }
          return;
        }
        case "vec": {
          const length = this.lebU32(path, "vec length");
          for (let i = 0; i < length; i += 1) {
            this.skipValue(entry.inner, path, depth + 1);
          }
          return;
        }
        case "record": {
          for (const type of entry.types) {
            this.skipValue(type, path, depth + 1);
          }
          return;
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
          this.skipValue(entry.types[index], path, depth + 1);
          return;
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
          return;
        }
        case "service": {
          this.skipServiceValue(path);
          return;
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
          return;
        }
      }
    }
    switch (wire) {
      case OP.null:
      case OP.reserved:
        return;
      case OP.bool: {
        const byte = this.byte(path);
        if (byte > 1) {
          this.fail("invalid_tag_byte", path, "a bool is 0 or 1");
        }
        return;
      }
      case OP.nat:
        this.lebBig(path);
        return;
      case OP.int:
        this.slebBig(path);
        return;
      case OP.nat8:
      case OP.int8:
        this.raw(1, path);
        return;
      case OP.nat16:
      case OP.int16:
        this.raw(2, path);
        return;
      case OP.nat32:
      case OP.int32:
      case OP.float32:
        this.raw(4, path);
        return;
      case OP.nat64:
      case OP.int64:
      case OP.float64:
        this.raw(8, path);
        return;
      case OP.text: {
        const length = this.lebU32(path, "text length");
        if (utf8Decode(this.raw(length, path)) === undefined) {
          this.fail("invalid_utf8", path, "text is not well-formed UTF-8");
        }
        return;
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
        return;
      }
      case OP.empty:
        this.fail("type_mismatch", path, "no value inhabits empty");
        return;
      default:
        this.fail("malformed_type_table", path, `unknown wire type ${wire}`);
    }
  }

  private skipServiceValue(path: readonly PathSegment[]): void {
    const tag = this.byte(path);
    if (tag === 0) {
      this.fail(
        "invalid_principal",
        path,
        "opaque references are unsupported in this slice",
      );
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
