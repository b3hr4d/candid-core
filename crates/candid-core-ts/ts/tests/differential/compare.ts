// The differential fuzz of issue #196, TypeScript half: replay a corpus the
// Rust driver (`tests/differential/main.rs`) wrote, run the same inputs
// through this runtime, and classify every case as agreement or as one named
// divergence category.
//
// Three targets, each against its Rust reference:
//
// - decode: `decodeArgs` with schemas `schemaFromContract` builds from the
//   environment's envelope, against the `candid` crate's
//   `IDLArgs::from_bytes_with_types`;
// - validate: `validate` on a rebuilt JavaScript value, against candid-core's
//   `validate_host_value` on the HostValue that value converts to;
// - contract: `schemaFromContract` on an edited Contract document, against
//   `Contract::from_json` on the same graph.
//
// # The verdict mapping
//
// Accept against accept compares the decoded values under one defined
// mapping (`domain` here, `wire::domain` in Rust): `bigint` as
// `{ $int: decimal }`, every `number` as `{ $num: its binary64 bits }` (NaN as
// `"nan"`), `Uint8Array` as `{ $blob: hex }`, strings, booleans and `null` as
// themselves, arrays and plain objects structurally.
//
// Reject against reject, for decode, compares error *classes*, never messages
// or paths. The reference's class comes from its behaviour (see `wire.rs`):
// `header` (the header or type table does not parse), `malformed` (the
// message does not decode at its own wire types), `coercion` (well-formed;
// only the expected types refuse it). This runtime's class comes from its
// first issue's code: WIRE codes (malformed bytes), COERCION codes (a
// type-level refusal), or `resource_limit_exceeded`. They must agree:
//
// - reference `header` → a WIRE code (both read the header first);
// - reference `coercion` → a COERCION code (the bytes are well-formed);
// - reference `malformed` → a WIRE or a COERCION code (whichever problem the
//   walk meets first, which depends on byte order, not on a rule);
// - `resource_limit_exceeded` agrees with any rejection, and so does the
//   reference's `limit` (its decoding quota ran out even on the retry the
//   driver gives a message whose work its scan bounds; see
//   `wire::RETRY_VALUES`): the two sides' budgets are different policies.
//
// For validate and contract, reject against reject is agreement whatever the
// two sides' reasons: only verdicts are compared there (the HostValue
// validator's and `Contract::from_json`'s refusal codes are a different
// vocabulary from this runtime's, with no class map between them).
//
// # Intended differences (encoded in the mapping, never silently agreed)
//
// A divergence the runtime makes on purpose still gets its own category
// (`…:intended:…`), so the reviewed expected-divergence list pins exactly
// which cases show it; a fault that removes the difference (for example a
// decoder that starts accepting overlong LEB128) makes those cases agree,
// which the list reports as a disappeared divergence. The categories and
// their reasons live in `tests/goldens/differential/divergences.json`.
//
// # Attribution is tied to the input (issue #196 review)
//
// A symptom (`decode:ts-rejects:<code>`, `decode:value-mismatch`, …) is
// attributed to an intended difference or to the reference only when the
// reference side found the property that explains it in the input itself
// (`ref.flags`, computed by the Rust driver from the bytes or the types,
// never from anything this runtime says): `overlong_leb128` is the LEB128
// rule only on a message that holds a non-minimal group, `invalid_principal`
// the reference-sequence limit only on one whose future value carries
// references, and so on (see `attributeRefusal`). The same code on an input
// without the property stays a plain symptom, which the list reports as new.
// The property must also explain the symptom itself: a limit refusal is
// attributed only on the budget the flag names (`value_depth` for
// `deep_nesting`, `value_elements` for `many_values`), a value mismatch on a
// `wire_empty_record` message only where the values differ at an absorbed
// reference, and a validate acceptance on a `label_collision` case only where
// the reference refused the field set or the arm. No reference budget stands
// in for a reference verdict: the driver retries an exhausted decoding quota
// and rebuilds a validate value the HostValue JSON decoder's nesting cap
// refuses (see `wire::RETRY_VALUES` and `host_verdict` in Rust).

import { decodeArgs, type CodecIssue } from "../../codec.ts";
import { schemaFromContract } from "../../contract.ts";
import { validate } from "../../validate.ts";
import type { AnySchema } from "../../schema.ts";

export type Descriptor = readonly [string, ...unknown[]];

export interface EnvLine {
  readonly kind: "env";
  readonly env: string;
  readonly did: string;
  readonly envelope: { readonly contract: unknown };
}

export interface Reference {
  readonly verdict: "accept" | "reject" | "panic" | "mapping_error";
  readonly values?: readonly unknown[];
  readonly class?: string;
  readonly flags?: readonly string[];
}

export interface DecodeLine {
  readonly kind: "decode";
  readonly id: string;
  readonly env: string;
  readonly wire: readonly string[] | null;
  readonly expected: readonly string[];
  readonly mutation: string;
  readonly hex: string;
  readonly ref: Reference;
}

export interface ValidateLine {
  readonly kind: "validate";
  readonly id: string;
  readonly env: string;
  readonly type: string;
  readonly value: Descriptor;
  readonly ref: Reference;
}

export interface ContractLine {
  readonly kind: "contract";
  readonly id: string;
  readonly env: string;
  readonly ops: readonly ContractOp[];
  readonly ref: Reference;
}

export interface ContractOp {
  readonly op: "set" | "delete" | "insert";
  readonly path: readonly (string | number)[];
  readonly value?: unknown;
}

export type CaseLine = DecodeLine | ValidateLine | ContractLine;

export interface Corpus {
  readonly header: { readonly [key: string]: unknown };
  readonly envs: ReadonlyMap<string, EnvLine>;
  readonly cases: readonly CaseLine[];
}

export function parseCorpus(text: string): Corpus {
  const lines = text.split("\n").filter((line) => line.length > 0);
  const header = JSON.parse(lines[0]) as { readonly [key: string]: unknown };
  const envs = new Map<string, EnvLine>();
  const cases: CaseLine[] = [];
  for (const line of lines.slice(1)) {
    const parsed = JSON.parse(line) as EnvLine | CaseLine;
    if (parsed.kind === "env") {
      envs.set(parsed.env, parsed);
    } else {
      cases.push(parsed);
    }
  }
  return { header, envs, cases };
}

export function fromHex(text: string): Uint8Array {
  const out = new Uint8Array(text.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = parseInt(text.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function toHex(bytes: Uint8Array): string {
  let text = "";
  for (const byte of bytes) {
    text += byte.toString(16).padStart(2, "0");
  }
  return text;
}

function bitsOf(value: number): string {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value);
  return view.getBigUint64(0).toString(16).padStart(16, "0");
}

function numberFromBits(bits: string): number {
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, BigInt(`0x${bits}`));
  return view.getFloat64(0);
}

/** A decoded domain value under the mapping the Rust driver emits. */
export function domain(value: unknown): unknown {
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return value;
  }
  if (typeof value === "bigint") {
    return { $int: value.toString() };
  }
  if (typeof value === "number") {
    return { $num: Number.isNaN(value) ? "nan" : bitsOf(value) };
  }
  if (value instanceof Uint8Array) {
    return { $blob: toHex(value) };
  }
  if (Array.isArray(value)) {
    return value.map(domain);
  }
  if (typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value)) {
      out[key] = domain((value as Record<string, unknown>)[key]);
    }
    return out;
  }
  return { $unmapped: typeof value };
}

/** The JavaScript value a descriptor denotes (see `host.rs`). */
export function fromDescriptor(descriptor: Descriptor): unknown {
  const [tag, payload] = descriptor;
  switch (tag) {
    case "n":
      return null;
    case "b":
      return payload as boolean;
    case "i":
      return BigInt(payload as string);
    case "f":
      return numberFromBits(payload as string);
    case "s":
      return payload as string;
    case "y":
      return fromHex(payload as string);
    case "a":
      return (payload as readonly Descriptor[]).map(fromDescriptor);
    case "o":
      // `Object.fromEntries` defines own data properties, so a `__proto__`
      // key is an ordinary key, never a prototype write.
      return Object.fromEntries(
        (payload as readonly (readonly [string, Descriptor])[]).map(([key, inner]) => [
          key,
          fromDescriptor(inner),
        ]),
      );
    default:
      throw new Error(`unknown descriptor tag ${JSON.stringify(tag)}`);
  }
}

/** Codes that report malformed bytes. */
const WIRE = new Set([
  "invalid_magic",
  "malformed_type_table",
  "overlong_leb128",
  "invalid_utf8",
  "invalid_tag_byte",
  "truncated",
  "trailing_bytes",
  "invalid_principal",
  "invalid_length",
  "duplicate_field_id",
]);
/** Codes that report a type-level refusal of well-formed bytes. */
const COERCION = new Set(["type_mismatch", "unknown_variant_tag", "missing_field"]);

function deepEqual(left: unknown, right: unknown): boolean {
  if (left === right) {
    return true;
  }
  if (typeof left !== "object" || typeof right !== "object" || left === null || right === null) {
    return false;
  }
  if (Array.isArray(left) !== Array.isArray(right)) {
    return false;
  }
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) {
    return false;
  }
  for (const key of leftKeys) {
    if (!Object.prototype.hasOwnProperty.call(right, key)) {
      return false;
    }
    if (
      !deepEqual((left as Record<string, unknown>)[key], (right as Record<string, unknown>)[key])
    ) {
      return false;
    }
  }
  return true;
}

export interface Loaded {
  readonly schemas: { readonly [name: string]: AnySchema } | null;
  /** The loader's first issue code when it refused the envelope. */
  readonly refusal?: string;
}

/** Load every environment's schemas once. */
export function loadEnvs(corpus: Corpus): Map<string, Loaded> {
  const loaded = new Map<string, Loaded>();
  for (const [id, env] of corpus.envs) {
    const built = schemaFromContract(env.envelope);
    loaded.set(
      id,
      built.ok
        ? { schemas: built.schemas }
        : { schemas: null, refusal: firstIssue(built.issues).code },
    );
  }
  return loaded;
}

/**
 * The loader refused an envelope candid-core itself compiled: every case in
 * that environment is a divergence (the reference accepted the Contract),
 * never a skip.
 */
function envRefused(kase: CaseLine, env: Loaded): Outcome {
  return {
    id: kase.id,
    category: `env:ts-refuses-contract:${env.refusal ?? "?"}`,
    ours: { verdict: "reject", code: env.refusal ?? "?", path: "$" },
  };
}

/** What this runtime answered, in a form small enough to report. */
export type Ours =
  | { readonly verdict: "accept"; readonly values?: readonly unknown[] }
  | {
      readonly verdict: "reject";
      readonly code: string;
      readonly path: string;
      /** The budget a `resource_limit_exceeded` refusal ran out of. */
      readonly resource?: string;
    }
  | { readonly verdict: "skip"; readonly reason: string };

/** The outcome of one case: its category (`null` = agreement) and our answer. */
export interface Outcome {
  readonly id: string;
  readonly category: string | null;
  readonly ours: Ours;
}

interface AnyIssue {
  readonly code: string;
  readonly path: string;
  readonly resource_limit?: { readonly resource: string };
}

/** Our rejection: the first issue's code and path, and its budget if any. */
function rejection(issues: readonly AnyIssue[]): Ours {
  const first = issues[0];
  if (first === undefined) {
    return { verdict: "reject", code: "none", path: "$" };
  }
  return first.resource_limit === undefined
    ? { verdict: "reject", code: first.code, path: first.path }
    : {
        verdict: "reject",
        code: first.code,
        path: first.path,
        resource: first.resource_limit.resource,
      };
}

function firstIssue(issues: readonly { code: string; path: string }[]): {
  code: string;
  path: string;
} {
  const first = issues[0];
  return first === undefined ? { code: "none", path: "$" } : first;
}

function runDecode(kase: DecodeLine, env: Loaded): Outcome {
  if (env.schemas === null) {
    return envRefused(kase, env);
  }
  const schemas: AnySchema[] = [];
  for (const name of kase.expected) {
    const schema = env.schemas[name];
    if (schema === undefined) {
      return { id: kase.id, category: null, ours: { verdict: "skip", reason: "omitted" } };
    }
    schemas.push(schema);
  }
  const result = decodeArgs(schemas, fromHex(kase.hex));
  const ours: Ours = result.ok
    ? { verdict: "accept", values: result.values.map(domain) }
    : rejection(result.issues as readonly CodecIssue[]);
  return { id: kase.id, category: decodeCategory(kase.ref, ours), ours };
}

function flagged(ref: Reference, flag: string): boolean {
  return ref.flags?.includes(flag) === true;
}

/**
 * The cause of a refusal the reference did not make (it accepted, or refused
 * only at the expected types), when the input carries the property that
 * explains it; `null` otherwise. Every attribution names its flag:
 *
 * - `leb128-minimality` (intended): a non-minimal LEB128/SLEB128 group is
 *   refused here (`overlong_leb128`; or `invalid_length` when the group is
 *   longer than a u32 field's five bytes although its value fits), accepted
 *   by the reference. Settled on #103 (decision 4): the spec's strict-inverse
 *   reading. Flag `non_minimal_leb128`: the scan met such a group.
 * - `empty-method-name` (intended): a func reference with an empty method
 *   name is refused here (`invalid_length`) for round-trip symmetry with
 *   `validate` and `encode`, which refuse it too (as does candid-core's
 *   HostValue validator). Flag `empty_method`: the scan met one (the
 *   reference may never decode it, when an `opt` above absorbs a later
 *   failure).
 * - `reference-sequences-unsupported` (intended): a future type's value with
 *   a non-zero reference count is refused here (`invalid_principal`), a
 *   documented limit of this codec (README: "opaque reference values … and
 *   external reference sequences are refused"); the reference ignores the
 *   count. Flag `future_references`.
 * - `ts-limit` (intended): `resource_limit_exceeded` on the budget the
 *   input exceeds: `value_depth` where the message's values, or the values
 *   the reference decoded from it at the expected types (a coercion can
 *   insert an `opt` at every level), nest deeper than the default `maxDepth`
 *   always admits (127 composite levels: two steps per level and one for the
 *   leaf; flag `deep_nesting`, see `wire::DEEP_LEVELS`), `value_elements`
 *   where the message holds more values than half `maxElements` (flag
 *   `many_values`). Any other budget, or the other one of the two, is a
 *   plain symptom: within those bounds a limit refusal is a defect.
 * - `skipped-method-utf8` (reference): a func reference's method name that is
 *   not UTF-8 is refused here (`invalid_utf8`); the reference does not check
 *   it when it skips the value. Flag `invalid_method_utf8`.
 * - `empty-wire-value` (reference): the reference reads a func or service
 *   value where the wire type is `empty` (its subtype relation has
 *   `empty <: t`, and is all it checks for references); this runtime refuses
 *   (`type_mismatch`: no value inhabits `empty`). Flag `wire_empty_value`:
 *   the message carries bytes at an `empty` wire type.
 */
function attributeRefusal(
  code: string,
  resource: string | undefined,
  ref: Reference,
): string | null {
  if (code === "invalid_length" && flagged(ref, "empty_method")) {
    return "decode:intended:empty-method-name";
  }
  if (
    (code === "overlong_leb128" || code === "invalid_length") &&
    flagged(ref, "non_minimal_leb128")
  ) {
    return "decode:intended:leb128-minimality";
  }
  if (code === "invalid_principal" && flagged(ref, "future_references")) {
    return "decode:intended:reference-sequences-unsupported";
  }
  if (
    code === "resource_limit_exceeded" &&
    ((resource === "value_depth" && flagged(ref, "deep_nesting")) ||
      (resource === "value_elements" && flagged(ref, "many_values")))
  ) {
    return "decode:intended:ts-limit";
  }
  if (code === "invalid_utf8" && flagged(ref, "invalid_method_utf8")) {
    return "decode:reference:skipped-method-utf8";
  }
  if (code === "type_mismatch" && flagged(ref, "wire_empty_value")) {
    return "decode:reference:empty-wire-value";
  }
  return null;
}

/**
 * Divergences the reference causes by rewriting uninhabited recursive records
 * in the *wire* type table to `empty` (`TypeEnv::replace_empty`, applied to
 * the wire side only): in a func or service signature that makes it accept a
 * reference type that is not a subtype (`empty <: E`) and refuse identical
 * types (`E <: empty`), and an `opt` around such a reference then absorbs to
 * `null`. The reference flags messages whose table holds such a record
 * reachable from a func or service type (`wire_empty_record`) — the only
 * place its subtype check runs; on those, these three symptoms are its doing.
 * A value mismatch is attributed only when the values differ nowhere but at
 * an absorbed reference (`absorbedReferencesOnly`).
 */
const EMPTY_NORMALIZATION_SYMPTOMS = new Set([
  "decode:ts-accepts:coercion",
  "decode:ts-rejects:type_mismatch",
  "decode:value-mismatch",
]);

export function decodeCategory(ref: Reference, ours: Ours): string | null {
  const symptom = decodeSymptom(ref, ours);
  if (symptom === null) {
    return null;
  }
  if (
    EMPTY_NORMALIZATION_SYMPTOMS.has(symptom) &&
    flagged(ref, "wire_empty_record") &&
    (symptom !== "decode:value-mismatch" ||
      (ours.verdict === "accept" && absorbedReferencesOnly(ours.values, ref.values)))
  ) {
    return "decode:reference:empty-normalization";
  }
  if (ours.verdict === "reject") {
    const refused =
      symptom === `decode:ts-rejects:${ours.code}` ||
      symptom === `decode:class:coercion-vs-${ours.code}`;
    if (refused) {
      return attributeRefusal(ours.code, ours.resource, ref) ?? symptom;
    }
  }
  return symptom;
}

const PRINCIPAL_TEXT = /^([a-z2-7]{5}-)*[a-z2-7]{1,5}$/;

/** A func or service reference under the domain mapping, boxed or not. */
function isReference(value: unknown): boolean {
  if (typeof value === "string") {
    // A service is its principal's canonical text: dash-separated groups of
    // five base32 characters (a text value of that form is not told apart,
    // but the flag already ties the case to a func or service type).
    return PRINCIPAL_TEXT.test(value);
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const keys = Object.keys(value).sort();
  if (keys.length === 1 && keys[0] === "some") {
    return isReference((value as { some: unknown }).some);
  }
  return keys.length === 2 && keys[0] === "method" && keys[1] === "principal";
}

/**
 * Whether two decoded values differ only where one holds `null` and the
 * other a func or service reference: the reference's failed subtype check
 * absorbed by an `opt` (`decode:reference:empty-normalization`), and nothing
 * else.
 */
function absorbedReferencesOnly(ours: unknown, theirs: unknown): boolean {
  if (deepEqual(ours, theirs)) {
    return true;
  }
  if (ours === null || theirs === null) {
    return isReference(ours === null ? theirs : ours);
  }
  if (typeof ours !== "object" || typeof theirs !== "object") {
    return false;
  }
  if (Array.isArray(ours) !== Array.isArray(theirs)) {
    return false;
  }
  const keys = Object.keys(ours);
  if (keys.length !== Object.keys(theirs).length) {
    return false;
  }
  return keys.every(
    (key) =>
      Object.prototype.hasOwnProperty.call(theirs, key) &&
      absorbedReferencesOnly(
        (ours as Record<string, unknown>)[key],
        (theirs as Record<string, unknown>)[key],
      ),
  );
}

/** A Candid label id: `_N_` read back, else the hash of the name. */
function labelId(key: string): string {
  const numbered = /^_(0|[1-9][0-9]*)_$/.exec(key);
  if (numbered !== null && Number(numbered[1]) < 4_294_967_296) {
    return numbered[1];
  }
  let hash = 0;
  for (const byte of new TextEncoder().encode(key)) {
    hash = (hash * 223 + byte) >>> 0;
  }
  return String(hash);
}

/** A domain value with every record key and variant tag replaced by its id. */
function byLabelId(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(byLabelId);
  }
  if (value === null || typeof value !== "object") {
    return value;
  }
  const out: Record<string, unknown> = {};
  for (const [key, inner] of Object.entries(value)) {
    out[key.startsWith("$") ? key : labelId(key)] =
      key === "tag" && typeof inner === "string" ? labelId(inner) : byLabelId(inner);
  }
  return out;
}

/**
 * Symptoms of one canonical field (or arm) written with two spellings (`d`
 * in one declaration, `100` in another): the field-name table attaches names to
 * canonical nodes, so this runtime keys both occurrences by the name while
 * the reference, and the mapping, read each declaration's own labels. The
 * reference flags a case whose types reach such a node (`label_collision`);
 * a decoded value must moreover equal the reference's once every label is
 * read as its id, so only a spelling difference is attributed. A validate
 * acceptance is attributed only where the reference refused the field set or
 * the arm (`record_field_set_mismatch`, `unknown_variant_id`): a key the
 * runtime reads by the other spelling.
 */
const COLLISION_REFUSALS = new Set(["record_field_set_mismatch", "unknown_variant_id"]);

const LABEL_COLLISION_SYMPTOMS = new Set([
  "decode:value-mismatch",
  "validate:ts-rejects:missing_field",
  "validate:ts-rejects:unexpected_field",
  "validate:ts-rejects:unknown_tag",
]);

export function withCollision(category: string | null, kase: CaseLine, ours: Ours): string | null {
  if (
    category === null ||
    !flagged(kase.ref, "label_collision") ||
    !(LABEL_COLLISION_SYMPTOMS.has(category) || category.startsWith("validate:ts-accepts:"))
  ) {
    return category;
  }
  if (
    category === "decode:value-mismatch" &&
    (ours.verdict !== "accept" || !deepEqual(byLabelId(ours.values), byLabelId(kase.ref.values)))
  ) {
    return category;
  }
  if (
    category.startsWith("validate:ts-accepts:") &&
    !COLLISION_REFUSALS.has(kase.ref.class ?? "")
  ) {
    return category;
  }
  return "env:label-collision";
}

function decodeSymptom(ref: Reference, ours: Ours): string | null {
  if (ref.verdict === "panic") {
    return "decode:reference-panic";
  }
  if (ref.verdict === "mapping_error") {
    return "decode:mapping-error";
  }
  if (ours.verdict === "skip") {
    return null;
  }
  if (ref.verdict === "accept") {
    if (ours.verdict === "accept") {
      return deepEqual(ours.values, ref.values) ? null : "decode:value-mismatch";
    }
    return `decode:ts-rejects:${ours.code}`;
  }
  if (ours.verdict === "accept") {
    // The reference's decoding quota is the harness's policy, not a rule of
    // either decoder. The driver retries every message whose work its scan
    // bounds (within twice `maxElements` values, so every message this
    // runtime can accept) under a quota that does not run out on it (see
    // `wire::RETRY_VALUES`); `limit` survives that only where the scan could
    // not bound the work.
    return ref.class === "limit"
      ? "decode:intended:reference-quota"
      : `decode:ts-accepts:${ref.class ?? "?"}`;
  }
  const code = ours.code;
  if (code === "resource_limit_exceeded" || ref.class === "limit") {
    return null;
  }
  const wire = WIRE.has(code);
  const coercion = COERCION.has(code);
  if (!wire && !coercion) {
    return `decode:ts-internal:${code}`;
  }
  switch (ref.class) {
    case "header":
      return wire ? null : `decode:class:header-vs-${code}`;
    case "coercion":
      return coercion ? null : `decode:class:coercion-vs-${code}`;
    case "malformed":
      return null;
    default:
      return `decode:class:${ref.class ?? "?"}-vs-${code}`;
  }
}

function runValidate(kase: ValidateLine, env: Loaded): Outcome {
  if (env.schemas === null) {
    return envRefused(kase, env);
  }
  const schema = env.schemas[kase.type];
  if (schema === undefined) {
    return { id: kase.id, category: null, ours: { verdict: "skip", reason: "omitted" } };
  }
  const result = validate(schema, fromDescriptor(kase.value));
  const ours: Ours = result.ok ? { verdict: "accept" } : rejection(result.issues);
  return { id: kase.id, category: verdictCategory("validate", kase.ref, ours), ours };
}

export function verdictCategory(target: string, ref: Reference, ours: Ours): string | null {
  if (ref.verdict === "panic") {
    return `${target}:reference-panic`;
  }
  if (ours.verdict === "skip") {
    return null;
  }
  if (ref.verdict === "accept") {
    if (ours.verdict === "accept") {
      return null;
    }
    // `validate` charges a `rec` hop a depth step as the decoder does (two
    // per level of a recursive type), while `max_value_depth` counts one per
    // container: past 127 levels the runtime's documented depth policy
    // refuses what the reference accepts. Attributed only on that budget and
    // on a value the reference side found nested that deep (`deep_nesting`,
    // from the HostValue it judged); any other limit refusal is a symptom.
    if (
      target === "validate" &&
      ours.code === "resource_limit_exceeded" &&
      ours.resource === "value_depth" &&
      flagged(ref, "deep_nesting")
    ) {
      return "validate:intended:ts-limit";
    }
    return `${target}:ts-rejects:${ours.code}`;
  }
  if (ours.verdict !== "accept") {
    return null;
  }
  // No refusal of the reference is intended either. Where the HostValue JSON
  // decoder's own budgets (its 64-container nesting cap) refuse a validate
  // value, the driver rebuilds it through the HostValue constructors, under
  // `max_value_depth` (256, this runtime's `maxDepth`), and records that
  // judgement; `host_value_limit` remains only for a value past those.
  return `${target}:ts-accepts:${ref.class ?? "?"}`;
}

/** Replay one JSON edit, exactly as `contract::apply` does in Rust. */
export function applyOp(document: unknown, op: ContractOp): void {
  const path = op.path;
  if (path.length === 0) {
    return;
  }
  let parent: unknown = document;
  for (const segment of path.slice(0, -1)) {
    if (typeof segment === "string") {
      if (
        parent === null ||
        typeof parent !== "object" ||
        Array.isArray(parent) ||
        !Object.prototype.hasOwnProperty.call(parent, segment)
      ) {
        return;
      }
      parent = (parent as Record<string, unknown>)[segment];
    } else {
      if (!Array.isArray(parent) || segment >= parent.length) {
        return;
      }
      parent = parent[segment];
    }
  }
  const last = path[path.length - 1];
  if (typeof last === "string") {
    if (parent === null || typeof parent !== "object" || Array.isArray(parent)) {
      return;
    }
    const map = parent as Record<string, unknown>;
    if (op.op === "set") {
      Object.defineProperty(map, last, {
        value: clone(op.value),
        enumerable: true,
        writable: true,
        configurable: true,
      });
    } else if (op.op === "delete") {
      delete map[last];
    }
    return;
  }
  if (!Array.isArray(parent)) {
    return;
  }
  if (op.op === "set" && last < parent.length) {
    parent[last] = clone(op.value);
  } else if (op.op === "delete" && last < parent.length) {
    parent.splice(last, 1);
  } else if (op.op === "insert" && last <= parent.length) {
    parent.splice(last, 0, clone(op.value));
  }
}

function clone(value: unknown): unknown {
  return value === undefined ? null : JSON.parse(JSON.stringify(value));
}

/**
 * The contract target's intended differences: `schemaFromContract` reads
 * neither `identities` nor `producer` (documented in `contract.ts`: the hashes
 * need canonicalization, candid-core's job; producer metadata is untrusted
 * provenance). When an edit touched either, the reference also judged the
 * document without those edits (`ref.graph`): if that graph is accepted, the
 * refusal came from the metadata edit and is by design; otherwise the
 * category names the graph's own refusal.
 */
function contractCategory(kase: ContractLine, ours: Ours): string | null {
  const category = verdictCategory("contract", kase.ref, ours);
  const graph = (kase.ref as { readonly graph?: Reference }).graph;
  if (category === null || !category.startsWith("contract:ts-accepts:") || graph === undefined) {
    return category;
  }
  if (graph.verdict === "accept") {
    const roots = new Set(kase.ops.map((op) => op.path[0]));
    return roots.has("identities")
      ? "contract:intended:identities-unchecked"
      : "contract:intended:producer-unchecked";
  }
  return verdictCategory("contract", graph, ours);
}

function runContract(kase: ContractLine, envLine: EnvLine): Outcome {
  const document = clone(envLine.envelope.contract);
  for (const op of kase.ops) {
    applyOp(document, op);
  }
  const result = schemaFromContract(document);
  const ours: Ours = result.ok ? { verdict: "accept" } : rejection(result.issues);
  return { id: kase.id, category: contractCategory(kase, ours), ours };
}

/** Classify every case of a corpus. */
export function runCorpus(corpus: Corpus): Outcome[] {
  const loaded = loadEnvs(corpus);
  const outcomes: Outcome[] = [];
  for (const kase of corpus.cases) {
    const env = loaded.get(kase.env);
    const envLine = corpus.envs.get(kase.env);
    if (env === undefined || envLine === undefined) {
      throw new Error(`${kase.id}: unknown environment ${kase.env}`);
    }
    let outcome: Outcome;
    switch (kase.kind) {
      case "decode":
        outcome = runDecode(kase, env);
        break;
      case "validate":
        outcome = runValidate(kase, env);
        break;
      case "contract":
        outcome = runContract(kase, envLine);
        break;
    }
    outcomes.push({ ...outcome, category: withCollision(outcome.category, kase, outcome.ours) });
  }
  return outcomes;
}
