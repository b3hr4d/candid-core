// The differential fuzz of issue #196, TypeScript half: replay a corpus the
// Rust driver (`tests/differential/main.rs`) wrote, run the same inputs
// through this runtime, and judge every case against the reference's verdict.
//
// Three targets, each against its Rust reference:
//
// - decode: `decodeArgs` with schemas `schemaFromContract` builds from the
//   environment's envelope, against the `candid` crate's
//   `IDLArgs::from_bytes_with_types`;
// - validate: `validate` on a rebuilt JavaScript value, against candid-core's
//   `validate_host_value` (under its default limits, whose `max_value_depth`
//   this runtime's depth budget mirrors) on the HostValue that value converts
//   to;
// - contract: `schemaFromContract` on an edited Contract document, against
//   `Contract::from_json` on the same graph.
//
// # The judge
//
// A case has one of four outcomes, and nothing else decides it:
//
// - `agree`: both accept with equal values, or both refuse with compatible
//   reasons (below);
// - `diverge`, with its exact `symptom`: both verdicts, and the error class
//   or code of each refusal (see `symptom`);
// - `inconclusive`: the reference refused on a budget of its own (its
//   decoding quota, its stack guard, a HostValue construction budget), so it
//   never judged the input — never an agreement, never a divergence;
// - `skip`: the loader omits the declaration by design (pinned by the test).
//
// There are no rules that call a divergence intended or the reference's
// fault. Whether a divergence is accepted is decided outside this file, by
// the reviewed list `tests/goldens/differential/divergences.json`: a case id,
// the issue that explains it, and the exact symptom observed. A divergence
// whose id is not listed, or listed with another symptom, fails CI; so does a
// listed case that no longer diverges so. That is the whole acceptance rule
// (issue #196 redesign): an input property is not evidence that a refusal is
// right, so none is consulted.
//
// # Compatible refusals
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
// first issue's code: WIRE codes (malformed bytes) or COERCION codes (a
// type-level refusal):
//
// - reference `header` → a WIRE code (both read the header first);
// - reference `coercion` → a COERCION code (the bytes are well-formed);
// - reference `malformed` → a WIRE or a COERCION code (whichever problem the
//   walk meets first, which depends on byte order, not on a rule).
//
// Any other code is a divergence: above all `resource_limit_exceeded`. The
// `candid` crate has no depth or element budget, and the generator keeps
// every case below this runtime's (`wire::GEN_LEVELS` and its neighbours), so
// a depth or element refusal here is never compatible with anything the
// reference said. The one decode budget the reference has like this
// runtime's is the type table's claimed size (`wire::GEN_TABLE_ENTRIES`,
// configured equal): its refusal, class
// `resource_limit_exceeded/type_table_entries`, agrees only with this
// runtime's refusal on that resource, and nothing else agrees with it.
//
// For validate and contract, two refusals agree whatever their codes (the
// two vocabularies have no class map), except a limit refusal: the
// reference's validate budgets (`value_depth` and `value_elements`)
// are verdicts, so a `resource_limit_exceeded` refusal agrees only with the
// reference's refusal on the same resource, and with nothing else.

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
  readonly verdict: "accept" | "reject" | "inconclusive" | "panic" | "mapping_error";
  readonly values?: readonly unknown[];
  /**
   * In place of `values` when their JSON is longer than 64 KiB (a width
   * vector's million values): see `valuesDigest`.
   */
  readonly values_digest?: string;
  readonly class?: string;
  /** For `inconclusive`: the reference budget that refused. */
  readonly budget?: string;
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
  /** The structural edit this op starts (`contract::structural`); not replayed. */
  readonly edit?: string;
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

/**
 * The bytes of a hex string. A regression vector may write a run as
 * `(<hex>*<count>)` (`wire::unhex` reads the same notation), so a boundary
 * vector of a million bytes stays a short line.
 */
export function fromHex(text: string): Uint8Array {
  const chunks: string[] = [];
  const run = /\(([0-9a-f]*)\*([0-9]+)\)/g;
  let at = 0;
  for (const match of text.matchAll(run)) {
    chunks.push(text.slice(at, match.index));
    chunks.push(match[1].repeat(Number(match[2])));
    at = match.index + match[0].length;
  }
  chunks.push(text.slice(at));
  const plain = chunks.join("");
  const out = new Uint8Array(plain.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = parseInt(plain.slice(i * 2, i * 2 + 2), 16);
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
    case "r":
      // A run: `["r", count, item]` is an array of `count` values `item`
      // denotes (a boundary vector's million elements, `host.rs`).
      return Array.from({ length: payload as number }, () =>
        fromDescriptor(descriptor[2] as Descriptor),
      );
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

/**
 * A short, stable digest of decoded values (FNV-1a, 32 bits, over their JSON
 * under the domain mapping): what a value-mismatch symptom carries, so a
 * listed mismatch whose decoded values change is a new symptom.
 */
export function digest(values: unknown): string {
  const text = JSON.stringify(values);
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

/** JSON with every object's keys sorted (`wire::canonical_json` in Rust). */
export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(",")}]`;
  }
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const keys = Object.keys(record).sort();
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

/**
 * `fnv1a32:<8 hex digits>/<length>` of the canonical JSON of decoded values:
 * what a corpus line records in place of values longer than 64 KiB of JSON
 * (`wire::values_digest`), compared with our values' digest.
 */
export function valuesDigest(values: unknown): string {
  const text = canonicalJson(values);
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `fnv1a32:${hash.toString(16).padStart(8, "0")}/${text.length}`;
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
      built.ok ? { schemas: built.schemas } : { schemas: null, refusal: firstCode(built.issues) },
    );
  }
  return loaded;
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
  | { readonly verdict: "skip"; readonly reason: string }
  /** The loader refused an envelope candid-core itself compiled. */
  | { readonly verdict: "env-refused"; readonly code: string };

/** How one case came out (see the module comment). */
export type Status = "agree" | "diverge" | "inconclusive" | "skip";

/** The outcome of one case: its status, its symptom when it diverges, our answer. */
export interface Outcome {
  readonly id: string;
  readonly kind: CaseLine["kind"];
  readonly status: Status;
  /** Set exactly when `status` is `diverge`. */
  readonly symptom: string | null;
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

function firstCode(issues: readonly { code: string }[]): string {
  return issues[0]?.code ?? "none";
}

/** Our side of a symptom: `accept`, `accept#<digest>`, `reject:<code>[/<resource>]`. */
function oursText(ours: Ours, withDigest: boolean): string {
  switch (ours.verdict) {
    case "accept":
      return withDigest ? `accept#${digest(ours.values)}` : "accept";
    case "reject":
      return ours.resource === undefined
        ? `reject:${ours.code}`
        : `reject:${ours.code}/${ours.resource}`;
    case "env-refused":
      return `env-refused:${ours.code}`;
    case "skip":
      return `skip:${ours.reason}`;
  }
}

/** The reference's side of a symptom: `accept` or `reject:<class>`. */
function refText(ref: Reference): string {
  return ref.verdict === "reject" ? `reject:${ref.class ?? "?"}` : ref.verdict;
}

/**
 * The exact symptom of a divergence: `ts=<ours> ref=<theirs>`, where a
 * value mismatch (both accept, values differ) carries a digest of our
 * decoded values (`ts=accept#1a2b3c4d ref=accept`).
 */
export function symptom(ref: Reference, ours: Ours): string {
  const valueMismatch = ours.verdict === "accept" && ref.verdict === "accept";
  return `ts=${oursText(ours, valueMismatch)} ref=${refText(ref)}`;
}

/** Our refusal's limit class, `resource_limit_exceeded/<resource>`, or null. */
function limitClass(ours: Ours): string | null {
  return ours.verdict === "reject" && ours.code === "resource_limit_exceeded"
    ? `resource_limit_exceeded/${ours.resource}`
    : null;
}

/** Whether two decode refusals agree (see the module comment). */
function decodeRefusalsAgree(refClass: string | undefined, ours: Ours): boolean {
  if (ours.verdict !== "reject") {
    return false;
  }
  const code = ours.code;
  if (refClass?.startsWith("resource_limit_exceeded/") === true) {
    return limitClass(ours) === refClass;
  }
  switch (refClass) {
    case "header":
      return WIRE.has(code);
    case "coercion":
      return COERCION.has(code);
    case "malformed":
      return WIRE.has(code) || COERCION.has(code);
    default:
      return false;
  }
}

/** Whether two validate or contract refusals agree (see the module comment). */
function verdictRefusalsAgree(refClass: string | undefined, ours: Ours): boolean {
  if (ours.verdict !== "reject") {
    return false;
  }
  const theirLimit = refClass?.startsWith("resource_limit_exceeded") === true ? refClass : null;
  return limitClass(ours) === theirLimit;
}

/** Judge one case from both answers (see the module comment). */
export function judge(kase: CaseLine, ours: Ours): Outcome {
  const outcome = (status: Status): Outcome => ({
    id: kase.id,
    kind: kase.kind,
    status,
    symptom: status === "diverge" ? symptom(kase.ref, ours) : null,
    ours,
  });
  const ref = kase.ref;
  if (ref.verdict !== "accept" && ref.verdict !== "reject") {
    // The reference did not judge (a budget of its own), or the harness
    // failed (a reference panic, a value the mapping cannot express): checked
    // before a skip, so a campaign counts every case the reference did not
    // judge, whatever this runtime did with it.
    return outcome("inconclusive");
  }
  if (ours.verdict === "skip") {
    return outcome("skip");
  }
  if (ours.verdict === "env-refused") {
    return outcome("diverge");
  }
  if (ref.verdict === "accept" && ours.verdict === "accept") {
    const equal =
      kase.kind !== "decode" ||
      (ref.values_digest === undefined
        ? deepEqual(ours.values, ref.values)
        : valuesDigest(ours.values) === ref.values_digest);
    return outcome(equal ? "agree" : "diverge");
  }
  if (ref.verdict === "reject" && ours.verdict === "reject") {
    const agree =
      kase.kind === "decode"
        ? decodeRefusalsAgree(ref.class, ours)
        : verdictRefusalsAgree(ref.class, ours);
    return outcome(agree ? "agree" : "diverge");
  }
  return outcome("diverge");
}

function runDecode(kase: DecodeLine, env: Loaded): Ours {
  if (env.schemas === null) {
    return { verdict: "env-refused", code: env.refusal ?? "?" };
  }
  const schemas: AnySchema[] = [];
  for (const name of kase.expected) {
    const schema = env.schemas[name];
    if (schema === undefined) {
      return { verdict: "skip", reason: "omitted" };
    }
    schemas.push(schema);
  }
  const result = decodeArgs(schemas, fromHex(kase.hex));
  return result.ok
    ? { verdict: "accept", values: result.values.map(domain) }
    : rejection(result.issues as readonly CodecIssue[]);
}

function runValidate(kase: ValidateLine, env: Loaded): Ours {
  if (env.schemas === null) {
    return { verdict: "env-refused", code: env.refusal ?? "?" };
  }
  const schema = env.schemas[kase.type];
  if (schema === undefined) {
    return { verdict: "skip", reason: "omitted" };
  }
  const result = validate(schema, fromDescriptor(kase.value));
  return result.ok ? { verdict: "accept" } : rejection(result.issues);
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

function runContract(kase: ContractLine, envLine: EnvLine): Ours {
  const document = clone(envLine.envelope.contract);
  for (const op of kase.ops) {
    applyOp(document, op);
  }
  const result = schemaFromContract(document);
  return result.ok ? { verdict: "accept" } : rejection(result.issues);
}

/** Judge every case of a corpus. */
export function runCorpus(corpus: Corpus): Outcome[] {
  const loaded = loadEnvs(corpus);
  const outcomes: Outcome[] = [];
  for (const kase of corpus.cases) {
    const env = loaded.get(kase.env);
    const envLine = corpus.envs.get(kase.env);
    if (env === undefined || envLine === undefined) {
      throw new Error(`${kase.id}: unknown environment ${kase.env}`);
    }
    let ours: Ours;
    switch (kase.kind) {
      case "decode":
        ours = runDecode(kase, env);
        break;
      case "validate":
        ours = runValidate(kase, env);
        break;
      case "contract":
        ours = runContract(kase, envLine);
        break;
    }
    outcomes.push(judge(kase, ours));
  }
  return outcomes;
}
