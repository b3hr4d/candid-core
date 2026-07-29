// The schema-driven JSON transport codec.
//
// This is the prototype's *interim* wire format, and saying so is part of the
// design: issue #103 (the TS-native Candid binary codec, conformance-vector
// gated) replaces this file at the transport boundary, and nothing above the
// `toWire`/`fromWire` surface knows JSON is underneath. Until then the
// framework needs a deterministic way to move domain values — `bigint`,
// `Uint8Array`, `Principal`, `T | null` options — through HTTP bodies and
// hydration payloads, which plain JSON cannot carry.
//
// Encoding, by schema position (schema-driven on both sides, so no tags):
//   nat/int/nat64/int64      decimal string
//   nat8..nat32/int8..int32  JSON number
//   float32/float64          JSON number; NaN/Infinity/-Infinity as those
//                            exact strings (bit payloads are not preserved —
//                            an accepted interim-codec limitation)
//   text                     JSON string        bool  JSON true/false
//   null                     JSON null          principal  its text form
//   blob                     base64 string      vec   JSON array
//   record                   JSON object in schema field order
//   tuple                    JSON array         unit  {}
//   variant                  { tag } / { tag, value }
//   opt T                    null | wire(T) — unambiguous because collapsing
//                            options are refused at construction
//   reserved                 null in both directions (the value carries no
//                            information)
//   empty                    unencodable; fails closed
//
// Accepted interim-codec limitations (all resolved by the #103 binary codec):
//   - negative zero: JSON.stringify serializes -0 as "0", so -0.0 round-trips
//     as +0.0 through every real transport;
//   - NaN payload bits are not preserved (a single canonical NaN token);
//   - `reserved` carries no information in either direction;
//   - two schemas with the same Candid type but different structural spelling
//     (`blob` vs `vec nat8`, `unit` vs the empty tuple) share an interface_id
//     yet encode differently here — the JSON shape is schema-position-driven,
//     so a value must be decoded under the same schema it was encoded with.
import type { Schema } from "./schema.ts";
import { CandidStartError } from "./errors.ts";
import { isPrincipalLike, principalFromText } from "./principal.ts";
import { ownEntry, resolveShape, shapeOf, type AnySchema } from "./walk.ts";
import type { ValidationIssue } from "./validate.ts";

export type WireValue =
  | null
  | boolean
  | number
  | string
  | WireValue[]
  | { [key: string]: WireValue };

const BASE64_ALPHABET =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

export function encodeBase64(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i] ?? 0;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    out += BASE64_ALPHABET[a >> 2];
    out += BASE64_ALPHABET[((a & 0x03) << 4) | ((b ?? 0) >> 4)];
    out += b === undefined ? "=" : BASE64_ALPHABET[((b & 0x0f) << 2) | ((c ?? 0) >> 6)];
    out += c === undefined ? "=" : BASE64_ALPHABET[c & 0x3f];
  }
  return out;
}

const BASE64_VALUES: ReadonlyMap<string, number> = new Map(
  Array.from(BASE64_ALPHABET, (char, index) => [char, index] as const),
);

/** Strict decode: canonical padding and alphabet only; `null` on anything else. */
export function decodeBase64(text: string): Uint8Array | null {
  if (text.length % 4 !== 0) {
    return null;
  }
  const padIndex = text.indexOf("=");
  const padLength = padIndex === -1 ? 0 : text.length - padIndex;
  if (padLength > 2) {
    return null;
  }
  if (padIndex !== -1 && !/^={1,2}$/.test(text.slice(padIndex))) {
    return null;
  }
  const body = padIndex === -1 ? text : text.slice(0, padIndex);
  const out = new Uint8Array((text.length / 4) * 3 - padLength);
  let outIndex = 0;
  let buffer = 0;
  let bits = 0;
  for (const char of body) {
    const value = BASE64_VALUES.get(char);
    if (value === undefined) {
      return null;
    }
    buffer = (buffer << 6) | value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      if (outIndex < out.length) {
        out[outIndex] = (buffer >> bits) & 0xff;
        outIndex += 1;
      } else if (((buffer >> bits) & 0xff) !== 0) {
        // Non-zero discarded bits mean a non-canonical encoding.
        return null;
      }
    }
  }
  if (outIndex !== out.length) {
    return null;
  }
  if (bits > 0 && (buffer & ((1 << bits) - 1)) !== 0) {
    return null;
  }
  return out;
}

/**
 * Encode a *domain-valid* value. Callers validate first (the canister facade
 * does); a value that still cannot be encoded is an internal invariant
 * failure and throws rather than emitting a corrupt wire document.
 */
export function toWire<T>(schema: Schema<T>, value: T): WireValue {
  return encode(schema, value as unknown);
}

function internal(message: string): never {
  throw new CandidStartError("wire_internal", message);
}

function encode(schema: AnySchema, value: unknown): WireValue {
  const shape = resolveShape(schema);
  if (shape === null) {
    internal("schema does not resolve to a structural node");
  }
  switch (shape.kind) {
    case "primitive":
      return encodePrimitive(shape.primitive ?? "", value);
    case "opt":
      if (shape.inner === undefined) {
        internal("opt schema without an inner schema");
      }
      return value === null ? null : encode(shape.inner, value);
    case "vec": {
      if (shape.inner === undefined) {
        internal("vec schema without an inner schema");
      }
      if (!Array.isArray(value)) {
        internal("vec value is not an array");
      }
      const inner = shape.inner;
      return value.map((element) => encode(inner, element));
    }
    case "blob":
      if (!(value instanceof Uint8Array)) {
        internal("blob value is not a Uint8Array");
      }
      return encodeBase64(value);
    case "unit":
      return {};
    case "record": {
      const fields = shape.fields;
      if (fields === undefined) {
        internal("record schema without fields");
      }
      if (typeof value !== "object" || value === null) {
        internal("record value is not an object");
      }
      const record = value as Record<string, unknown>;
      const out: { [key: string]: WireValue } = {};
      for (const key of Object.keys(fields)) {
        const fieldSchema = fields[key];
        if (fieldSchema === undefined) {
          continue;
        }
        safeSet(out, key, encode(fieldSchema, record[key]));
      }
      return out;
    }
    case "tuple": {
      const elements = shape.elements;
      if (elements === undefined) {
        internal("tuple schema without elements");
      }
      if (!Array.isArray(value) || value.length !== elements.length) {
        internal("tuple value has the wrong shape");
      }
      return elements.map((element, index) => encode(element, value[index]));
    }
    case "variant": {
      const arms = shape.arms;
      if (arms === undefined) {
        internal("variant schema without arms");
      }
      if (typeof value !== "object" || value === null) {
        internal("variant value is not an object");
      }
      const candidate = value as { tag?: unknown; value?: unknown };
      if (typeof candidate.tag !== "string") {
        internal("variant value has no string tag");
      }
      const arm = ownEntry(arms, candidate.tag);
      if (arm === undefined) {
        internal(`variant tag ${JSON.stringify(candidate.tag)} has no arm`);
      }
      const armShape = resolveShape(arm);
      if (armShape === null) {
        internal("variant arm schema does not resolve");
      }
      if (armShape.kind === "primitive" && armShape.primitive === "null") {
        return { tag: candidate.tag };
      }
      return { tag: candidate.tag, value: encode(arm, candidate.value) };
    }
    default:
      internal(`unknown schema kind \`${shape.kind}\``);
  }
}

function encodePrimitive(primitive: string, value: unknown): WireValue {
  switch (primitive) {
    case "null":
      return null;
    case "bool":
      if (typeof value !== "boolean") {
        internal("bool value is not a boolean");
      }
      return value;
    case "text":
      if (typeof value !== "string") {
        internal("text value is not a string");
      }
      return value;
    case "float32":
    case "float64":
      if (typeof value !== "number") {
        internal("float value is not a number");
      }
      if (Number.isFinite(value)) {
        return value;
      }
      return Number.isNaN(value) ? "NaN" : value > 0 ? "Infinity" : "-Infinity";
    case "nat":
    case "int":
    case "nat64":
    case "int64":
      if (typeof value !== "bigint") {
        internal(`${primitive} value is not a bigint`);
      }
      return value.toString(10);
    case "nat8":
    case "nat16":
    case "nat32":
    case "int8":
    case "int16":
    case "int32":
      if (typeof value !== "number") {
        internal(`${primitive} value is not a number`);
      }
      return value;
    case "reserved":
      return null;
    case "empty":
      internal("`empty` values cannot exist, so none can be encoded");
      break;
    case "principal":
      if (!isPrincipalLike(value)) {
        internal("principal value has no toText()");
      }
      return value.toText();
    default:
      internal(`unknown primitive \`${primitive}\``);
  }
}

export type DecodeResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly issues: readonly ValidationIssue[] };

/**
 * Decode untrusted wire JSON into a domain value. Structural only: shape
 * mismatches fail here with `wire_*` codes, and the caller then runs
 * `validate` over the result — range checks and domain rules live there,
 * once, rather than in two half-copies.
 */
export function fromWire<T>(schema: Schema<T>, wire: unknown): DecodeResult<T> {
  const decoder = new Decoder();
  const value = decoder.decode(schema, wire, 0);
  if (decoder.issues.length > 0) {
    return { ok: false, issues: decoder.issues };
  }
  return { ok: true, value: value as T };
}

const MAX_WIRE_DEPTH = 64;
const MAX_BIGINT_DIGITS = 1024;

class Decoder {
  readonly issues: ValidationIssue[] = [];
  private readonly path: (string | number)[] = [];

  private issue(code: string, message: string): undefined {
    let rendered = "$";
    for (const segment of this.path) {
      rendered +=
        typeof segment === "number"
          ? `[${segment}]`
          : /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(segment)
            ? `.${segment}`
            : `[${JSON.stringify(segment)}]`;
    }
    this.issues.push({ code, path: rendered, message });
    return undefined;
  }

  decode(schema: AnySchema, wire: unknown, depth: number): unknown {
    if (depth > MAX_WIRE_DEPTH) {
      return this.issue(
        "wire_depth_exceeded",
        `wire nesting exceeded ${MAX_WIRE_DEPTH} levels`,
      );
    }
    const shape = resolveShape(schema);
    if (shape === null) {
      return this.issue("wire_internal", "schema does not resolve");
    }
    switch (shape.kind) {
      case "primitive":
        return this.decodePrimitive(shape.primitive ?? "", wire);
      case "opt": {
        if (shape.inner === undefined) {
          return this.issue("wire_internal", "opt schema without inner");
        }
        if (wire === null) {
          return null;
        }
        return this.decode(shape.inner, wire, depth + 1);
      }
      case "vec": {
        if (shape.inner === undefined) {
          return this.issue("wire_internal", "vec schema without inner");
        }
        if (!Array.isArray(wire)) {
          return this.issue("wire_expected_array", "expected a JSON array");
        }
        const out: unknown[] = [];
        for (let index = 0; index < wire.length; index += 1) {
          this.path.push(index);
          out.push(this.decode(shape.inner, wire[index], depth + 1));
          this.path.pop();
        }
        return this.issues.length > 0 ? undefined : out;
      }
      case "blob": {
        if (typeof wire !== "string") {
          return this.issue("wire_expected_base64", "expected a base64 string");
        }
        const bytes = decodeBase64(wire);
        if (bytes === null) {
          return this.issue("wire_expected_base64", "invalid base64");
        }
        return bytes;
      }
      case "unit": {
        if (!isPlainObject(wire)) {
          return this.issue("wire_expected_object", "expected {}");
        }
        if (Object.keys(wire).length > 0) {
          return this.issue("wire_unexpected_field", "the empty record has no fields");
        }
        return {};
      }
      case "record": {
        const fields = shape.fields;
        if (fields === undefined) {
          return this.issue("wire_internal", "record schema without fields");
        }
        if (!isPlainObject(wire)) {
          return this.issue("wire_expected_object", "expected a JSON object");
        }
        const out: Record<string, unknown> = {};
        for (const key of Object.keys(fields)) {
          const fieldSchema = fields[key];
          if (fieldSchema === undefined) {
            continue;
          }
          if (!Object.prototype.hasOwnProperty.call(wire, key)) {
            this.path.push(key);
            this.issue("wire_missing_field", "required field is absent");
            this.path.pop();
            continue;
          }
          this.path.push(key);
          safeSet(out, key, this.decode(fieldSchema, wire[key], depth + 1));
          this.path.pop();
        }
        for (const key of Object.keys(wire)) {
          if (!Object.prototype.hasOwnProperty.call(fields, key)) {
            this.path.push(key);
            this.issue("wire_unexpected_field", "field is not part of the record type");
            this.path.pop();
          }
        }
        return this.issues.length > 0 ? undefined : out;
      }
      case "tuple": {
        const elements = shape.elements;
        if (elements === undefined) {
          return this.issue("wire_internal", "tuple schema without elements");
        }
        if (!Array.isArray(wire) || wire.length !== elements.length) {
          return this.issue(
            "wire_expected_tuple",
            `expected a JSON array of ${elements.length} element(s)`,
          );
        }
        const out: unknown[] = [];
        for (let index = 0; index < elements.length; index += 1) {
          const element = elements[index];
          if (element === undefined) {
            continue;
          }
          this.path.push(index);
          out.push(this.decode(element, wire[index], depth + 1));
          this.path.pop();
        }
        return this.issues.length > 0 ? undefined : out;
      }
      case "variant": {
        const arms = shape.arms;
        if (arms === undefined) {
          return this.issue("wire_internal", "variant schema without arms");
        }
        if (!isPlainObject(wire)) {
          return this.issue("wire_expected_variant", "expected { tag } or { tag, value }");
        }
        const tag = wire["tag"];
        if (typeof tag !== "string") {
          return this.issue("wire_expected_variant", "variant has no string `tag`");
        }
        const extraKeys = Object.keys(wire).filter(
          (key) => key !== "tag" && key !== "value",
        );
        if (extraKeys.length > 0) {
          return this.issue("wire_unexpected_field", "variant carries extra keys");
        }
        const arm = ownEntry(arms, tag);
        if (arm === undefined) {
          return this.issue(
            "wire_unknown_variant_tag",
            `no arm named ${JSON.stringify(tag)}`,
          );
        }
        const armShape = resolveShape(arm);
        if (armShape === null) {
          return this.issue("wire_internal", "variant arm does not resolve");
        }
        if (armShape.kind === "primitive" && armShape.primitive === "null") {
          if (Object.prototype.hasOwnProperty.call(wire, "value")) {
            return this.issue(
              "wire_variant_payload_unexpected",
              `arm ${JSON.stringify(tag)} carries no payload`,
            );
          }
          return { tag };
        }
        if (!Object.prototype.hasOwnProperty.call(wire, "value")) {
          return this.issue(
            "wire_variant_payload_missing",
            `arm ${JSON.stringify(tag)} requires a payload`,
          );
        }
        this.path.push("value");
        const payload = this.decode(arm, wire["value"], depth + 1);
        this.path.pop();
        return this.issues.length > 0 ? undefined : { tag, value: payload };
      }
      default:
        return this.issue("wire_internal", `unknown schema kind \`${shape.kind}\``);
    }
  }

  private decodePrimitive(primitive: string, wire: unknown): unknown {
    switch (primitive) {
      case "null":
        if (wire !== null) {
          return this.issue("wire_expected_null", "expected JSON null");
        }
        return null;
      case "bool":
        if (typeof wire !== "boolean") {
          return this.issue("wire_expected_bool", "expected a JSON boolean");
        }
        return wire;
      case "text":
        if (typeof wire !== "string") {
          return this.issue("wire_expected_string", "expected a JSON string");
        }
        return wire;
      case "float32":
      case "float64": {
        if (typeof wire === "number") {
          return wire;
        }
        if (wire === "NaN") {
          return Number.NaN;
        }
        if (wire === "Infinity") {
          return Number.POSITIVE_INFINITY;
        }
        if (wire === "-Infinity") {
          return Number.NEGATIVE_INFINITY;
        }
        return this.issue(
          "wire_expected_number",
          "expected a JSON number or NaN/Infinity/-Infinity",
        );
      }
      case "nat":
      case "int":
      case "nat64":
      case "int64": {
        if (typeof wire !== "string" || !/^-?\d+$/.test(wire)) {
          return this.issue(
            "wire_expected_bigint",
            `expected a decimal string for ${primitive}`,
          );
        }
        // `BigInt(str)` is O(n^2) in the digit count, so a multi-megabyte
        // decimal string within the 2 MiB body bound could still burn
        // hundreds of ms of synchronous CPU. Cap the digit count well above
        // any legitimate value (2^64 is 20 digits; 1024 digits is ~3400
        // bits) and fail closed before the conversion runs.
        if (wire.length > MAX_BIGINT_DIGITS) {
          return this.issue(
            "wire_number_too_large",
            `decimal string for ${primitive} exceeds ${MAX_BIGINT_DIGITS} digits`,
          );
        }
        return BigInt(wire);
      }
      case "nat8":
      case "nat16":
      case "nat32":
      case "int8":
      case "int16":
      case "int32":
        if (typeof wire !== "number") {
          return this.issue(
            "wire_expected_number",
            `expected a JSON number for ${primitive}`,
          );
        }
        return wire;
      case "reserved":
        // Whatever arrived, the domain value is `null`: reserved carries
        // nothing, and passing attacker-shaped structure through would hand
        // the application unvalidated data under a schema that asserts
        // nothing about it.
        return null;
      case "empty":
        return this.issue("wire_uninhabited", "`empty` has no values");
      case "principal": {
        if (typeof wire !== "string") {
          return this.issue("wire_expected_principal", "expected principal text");
        }
        return principalFromText(wire);
      }
      default:
        return this.issue("wire_internal", `unknown primitive \`${primitive}\``);
    }
  }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    !(value instanceof Uint8Array)
  );
}

/**
 * Assign an own property even when the key is `__proto__`. A legal Candid
 * label `__proto__` would otherwise hit `Object.prototype`'s setter — the
 * field would vanish from the encoded document and the decoded object rather
 * than round-trip. The reads already use `ownEntry`; the writes match it.
 */
function safeSet<V>(target: Record<string, V>, key: string, value: V): void {
  if (key === "__proto__") {
    Object.defineProperty(target, key, {
      value,
      enumerable: true,
      writable: true,
      configurable: true,
    });
  } else {
    target[key] = value;
  }
}
