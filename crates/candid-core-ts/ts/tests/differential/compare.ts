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
// Reject against reject compares error *classes*, never messages or paths.
// The reference's class comes from its behaviour (see `wire.rs`): `header`
// (the header or type table does not parse), `malformed` (the message does
// not decode at its own wire types), `coercion` (well-formed; only the
// expected types refuse it). This runtime's class comes from its first
// issue's code: WIRE codes (malformed bytes), COERCION codes (a type-level
// refusal), or `resource_limit_exceeded`. They must agree:
//
// - reference `header` → a WIRE code (both read the header first);
// - reference `coercion` → a COERCION code (the bytes are well-formed);
// - reference `malformed` → a WIRE or a COERCION code (whichever problem the
//   walk meets first, which depends on byte order, not on a rule);
// - `resource_limit_exceeded` agrees with any rejection, and so does the
//   reference's `limit` (its decoding quota ran out): the two sides' budgets
//   are different policies (see "intended differences").
//
// # Intended differences (encoded in the mapping, never silently agreed)
//
// A divergence the runtime makes on purpose still gets its own category
// (`…:intended:…`), so the reviewed expected-divergence list pins exactly
// which cases show it; a fault that removes the difference (for example a
// decoder that starts accepting overlong LEB128) makes those cases agree,
// which the list reports as a disappeared divergence. The categories and
// their reasons live in `tests/goldens/differential/divergences.json`.

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
  /** One canonical field written with two spellings (see `main.rs`). */
  readonly label_collision?: boolean;
}

interface Reference {
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
  | { readonly verdict: "reject"; readonly code: string; readonly path: string }
  | { readonly verdict: "skip"; readonly reason: string };

/** The outcome of one case: its category (`null` = agreement) and our answer. */
export interface Outcome {
  readonly id: string;
  readonly category: string | null;
  readonly ours: Ours;
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
    : { verdict: "reject", ...firstIssue(result.issues as readonly CodecIssue[]) };
  return { id: kase.id, category: decodeCategory(kase.ref, ours), ours };
}

/**
 * The two decode differences this runtime makes on purpose, recognised where
 * the reference saw well-formed bytes (it accepted, or refused only at the
 * expected types) and this runtime refused them as malformed:
 *
 * - `leb128-minimality`: a non-minimal LEB128/SLEB128 group is refused here
 *   (`overlong_leb128`; or `invalid_length` when the group is longer than a
 *   u32 field's five bytes although its value fits, which the reference's
 *   acceptance proves), and accepted by the reference. Settled on #103
 *   (decision 4): the spec's strict-inverse reading.
 * - `empty-method-name`: a func reference with an empty method name is
 *   refused here (`invalid_length`) for round-trip symmetry with `validate`
 *   and `encode`, which refuse it too (as does candid-core's HostValue
 *   validator); the reference accepts it. The reference flags these inputs
 *   (`flags: ["empty_method"]`).
 * - `reference-sequences-unsupported`: a value carrying references (a future
 *   type's value with a non-zero reference count; an opaque func or service
 *   reference) is refused here (`invalid_principal`), a documented limit of
 *   this codec (README: "opaque reference values … and external reference
 *   sequences are refused"); the reference skips the count. A principal
 *   longer than 29 bytes, the code's other source, the reference refuses too.
 */
function intendedWire(code: string, ref: Reference): string | null {
  if (code === "overlong_leb128") {
    return "decode:intended:leb128-minimality";
  }
  if (code === "invalid_principal") {
    return "decode:intended:reference-sequences-unsupported";
  }
  if (code === "invalid_length") {
    return ref.flags?.includes("empty_method") === true
      ? "decode:intended:empty-method-name"
      : "decode:intended:leb128-minimality";
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
 * (`wire_empty_record`); on those, these three symptoms are its doing.
 */
const EMPTY_NORMALIZATION_SYMPTOMS = new Set([
  "decode:ts-accepts:coercion",
  "decode:ts-rejects:type_mismatch",
  "decode:value-mismatch",
]);

export function decodeCategory(ref: Reference, ours: Ours): string | null {
  const category = decodeSymptom(ref, ours);
  if (category === null) {
    return null;
  }
  if (
    EMPTY_NORMALIZATION_SYMPTOMS.has(category) &&
    ref.flags?.includes("wire_empty_record") === true
  ) {
    return "decode:reference:empty-normalization";
  }
  // The reference reads a func or service value where the wire type is
  // `empty` (its subtype relation has `empty <: t` and is all it checks for
  // references); this runtime refuses, no value inhabiting `empty`.
  if (
    category === "decode:ts-rejects:type_mismatch" &&
    ref.flags?.includes("wire_empty") === true
  ) {
    return "decode:reference:empty-wire-value";
  }
  return category;
}

/**
 * Symptoms of one canonical field written with two spellings (`d` in one
 * declaration, `100` in another): the field-name table attaches names to
 * canonical nodes, so this runtime keys both occurrences by the name while
 * the reference, and the mapping, read each declaration's own labels.
 */
const LABEL_COLLISION_SYMPTOMS = new Set([
  "decode:value-mismatch",
  "validate:ts-rejects:missing_field",
  "validate:ts-rejects:unexpected_field",
]);

function withEnv(category: string | null, envLine: EnvLine): string | null {
  return category !== null &&
    envLine.label_collision === true &&
    (LABEL_COLLISION_SYMPTOMS.has(category) || category.startsWith("validate:ts-accepts:"))
    ? "env:label-collision"
    : category;
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
    if (ours.code === "resource_limit_exceeded") {
      return "decode:intended:ts-limit";
    }
    return intendedWire(ours.code, ref) ?? `decode:ts-rejects:${ours.code}`;
  }
  if (ours.verdict === "accept") {
    // The reference's decoding quota is the harness's policy, not a rule of
    // either decoder (see `wire::DECODING_QUOTA`).
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
      return coercion ? null : (intendedWire(code, ref) ?? `decode:class:coercion-vs-${code}`);
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
  const ours: Ours = result.ok
    ? { verdict: "accept" }
    : { verdict: "reject", ...firstIssue(result.issues) };
  return { id: kase.id, category: verdictCategory("validate", kase.ref, ours), ours };
}

function verdictCategory(target: string, ref: Reference, ours: Ours): string | null {
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
    if (ours.code === "resource_limit_exceeded") {
      return `${target}:intended:ts-limit`;
    }
    return `${target}:ts-rejects:${ours.code}`;
  }
  if (ours.verdict !== "accept") {
    return null;
  }
  // The HostValue JSON decoder's budgets (64 levels of JSON nesting, …) are a
  // policy of that ABI, not of the domain: this runtime's own `maxDepth`
  // (256 schema steps) admits deeper values.
  return ref.class === "host_value_limit"
    ? `${target}:intended:limit-policy`
    : `${target}:ts-accepts:${ref.class ?? "?"}`;
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
  const ours: Ours = result.ok
    ? { verdict: "accept" }
    : { verdict: "reject", ...firstIssue(result.issues) };
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
    outcomes.push({ ...outcome, category: withEnv(outcome.category, envLine) });
  }
  return outcomes;
}
