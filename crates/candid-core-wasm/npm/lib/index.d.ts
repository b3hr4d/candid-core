/**
 * The library surface of `@candid-core/cli`: data in, data out.
 *
 * Every type here describes what the wasm compiler actually serializes. In
 * particular a {@link Diagnostic} carries only `code` and `message`
 * unconditionally — every other field is genuinely optional, because the same
 * item algebra carries both compile diagnostics (which populate `phase` and
 * `severity`) and validation violations (which populate `path` and never
 * those two).
 */

/**
 * Candid input. Either self-contained text, or a bundle naming its entry.
 *
 * `{ source }` and `{ entry, files }` are mutually exclusive, and no other key
 * is accepted — a request carrying one is refused with `invalid_request`.
 */
export type Sources =
  string | { source: string } | { entry: string; files: Record<string, string> };

/**
 * A logical source location. Every field is optional: an *approximate* span
 * names the source without offsets, and a span may name offsets without a
 * source.
 */
export interface SourceSpan {
  source_name?: string;
  start_byte?: number;
  end_byte?: number;
}

/** A secondary location attached to a diagnostic. */
export interface RelatedLocation {
  message: string;
  span?: SourceSpan;
}

/** Which bound refused, the ceiling it enforces, and what was observed. */
export interface ResourceLimitInfo {
  resource: string;
  limit: number;
  observed: number;
}

/**
 * One failure item.
 *
 * `phase` is deliberately `string` rather than a union: the compiler's own
 * phases are joined by values this package writes itself, so any closed set
 * would be wrong today and wronger later. Compare `code`, which is the stable
 * identifier meant for programmatic use.
 */
export interface Diagnostic {
  code: string;
  message: string;
  phase?: string;
  severity?: string;
  path?: string;
  span?: SourceSpan;
  related?: RelatedLocation[];
  notes?: string[];
  resource_limit?: ResourceLimitInfo;
}

/**
 * The failure document, identical to what the native `candid-core` CLI
 * prints. Nothing is thrown for a data error.
 */
export interface Failure {
  ok: false;
  diagnostics: Diagnostic[];
}

/** One `[container, id, name]` field-label triple. */
export type FieldNameTriple = [container: number, id: number, name: string];

/**
 * A one-document `ContractEnvelope`: the canonical contract plus its
 * extensions map. `contract` is typed `unknown` on purpose — it is handed
 * whole to `schemaFromContract`, which itself accepts `unknown`, and pinning
 * its shape here would duplicate a model that versions independently.
 */
export interface ContractEnvelope {
  contract: unknown;
  extensions: {
    "org.candid-core.field-names/v1"?: FieldNameTriple[];
    [extension: string]: unknown;
  };
}

/** A successful module generation. */
export interface ModuleSuccess {
  ok: true;
  module: string;
}

/**
 * Initialize the embedded wasm module once; repeated calls return the same
 * promise. Node needs no argument. A browser may also pass nothing, or supply
 * its own `BufferSource`, `URL` or `Response`.
 *
 * The parameter is typed `unknown` deliberately: `BufferSource`, `URL` and
 * `Response` are DOM types, and naming them here would break every consumer
 * compiling without the DOM lib — which is most Node consumers of a CLI.
 */
export function init(input?: unknown): Promise<void>;

/**
 * Compile Candid sources into a one-document `ContractEnvelope` carrying the
 * `org.candid-core.field-names/v1` extension — exactly the document
 * `candid-core compile <path> --envelope` emits, ready for
 * `schemaFromContract`.
 *
 * Discriminate on the envelope, not on `ok`: success carries no `ok` key.
 *
 * @example
 * const result = await didToContract("service : { ping : () -> (); }");
 * if ("contract" in result) {
 *   const built = schemaFromContract(result);
 * } else {
 *   for (const issue of result.diagnostics) console.error(issue.code, issue.message);
 * }
 */
export function didToContract(sources: Sources): Promise<ContractEnvelope | Failure>;

/**
 * Generate the `@candid-core/schema` TypeScript module for Candid sources,
 * byte-identical to what the Rust-native generator emits.
 *
 * @example
 * const result = await didToModule("service : { ping : () -> (); }");
 * if (result.ok) await writeFile("./service.ts", result.module);
 */
export function didToModule(sources: Sources): Promise<ModuleSuccess | Failure>;
