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

/**
 * One declaration or method a module leaves out.
 */
export interface Omission {
  /** A named declaration, or a method of the actor's service. */
  kind: "declaration" | "method";
  /** The declaration or method name, as the Candid source spells it. */
  name: string;
  /**
   * Why, from a closed set: `reserved_field_name` (a field or arm named
   * like the `_N_` id rendering), `ambiguous_variant_arm` (an arm whose
   * payload is a declared `opt` of an uninhabited type),
   * `reserved_export_name` (a declaration named `actor` or `Actor`),
   * `invalid_declaration_name` (a name that is not identifier-shaped;
   * Contract documents only), or `references_omitted` (it references the
   * omitted declaration named by `via`).
   */
  reason:
    | "reserved_field_name"
    | "ambiguous_variant_arm"
    | "reserved_export_name"
    | "invalid_declaration_name"
    | "references_omitted";
  /** For `references_omitted` only: the omitted declaration referenced. */
  via?: string;
}

/**
 * A successful module generation. A module that had to leave something out
 * is still a success: everything it emits is exactly what it would be
 * without the omitted declarations, and `omitted` lists what is missing.
 */
export interface ModuleSuccess {
  ok: true;
  /**
   * The generated TypeScript. With omissions, its header lists them, one
   * `// Omitted:` line each; with none, the text is unchanged.
   */
  module: string;
  /**
   * What the module leaves out: a declaration no module can
   * represent, together with every declaration and actor method that
   * references it — through nested `func` and `service` types too, up to the
   * containing declaration. Only the actor drops individual methods, from
   * both `actor` and `Actor`. Declarations first, then methods, each sorted
   * by name; empty, never absent, when nothing is omitted.
   * `schemaFromContract` reports the same list for the same Contract.
   */
  omitted: Omission[];
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
 * byte-identical to what the Rust-native generator emits, with the list of
 * declarations and methods it had to leave out.
 *
 * @example
 * const result = await didToModule("service : { ping : () -> (); }");
 * if (result.ok) {
 *   for (const entry of result.omitted) console.warn("omitted", entry.name, entry.reason);
 *   await writeFile("./service.ts", result.module);
 * }
 */
export function didToModule(sources: Sources): Promise<ModuleSuccess | Failure>;

/** The identities of one compiled interface. */
export interface Identities {
  /** The Contract identity: the whole canonical graph, declarations included. */
  contract_id: string;
  /**
   * The interface identity: only what the service reaches. Equal identities
   * mean an unchanged interface.
   */
  interface_id: string;
}

/** A successful projection. */
export interface ProjectionSuccess {
  ok: true;
  /**
   * The projected `.did`: the methods named and every declaration they
   * reach, as one self-contained file. Declaration names, doc comments and
   * argument names are kept; a service class's init arguments are not.
   * Methods are printed in name order (code point), record fields and
   * variant arms in label-id order, declarations in source order (the
   * entry's first, then each imported source's).
   */
  did: string;
  /** The methods the projection holds, in name order (code point). */
  methods: string[];
  /** The identities of the sources projected. */
  input: Identities;
  /** The identities of the projection. */
  projection: Identities;
}

/**
 * Project Candid sources onto the methods named: a self-contained `.did`
 * holding exactly those methods and every declaration they reach. The same
 * sources and the same set of names give the same bytes, in any order.
 *
 * A failure is `unknown_method` (one per name the service lacks; its
 * `notes` list the service's methods), `empty_method_list`, `no_service`,
 * or the compiler's own diagnostics.
 *
 * @example
 * const result = await projectDid(ledgerDid, ["icrc1_balance_of", "icrc1_transfer"]);
 * if (result.ok) await writeFile("./ledger.did", result.did);
 */
export function projectDid(
  sources: Sources,
  methods: readonly string[],
): Promise<ProjectionSuccess | Failure>;

/**
 * One finding of a compatibility check.
 *
 * - `method_missing` (error): the live service has no method of that name.
 * - `mode_changed` (error): the method's mode differs (`query`,
 *   `composite_query`, `update` or `oneway`).
 * - `method_incompatible` (error): the live method's type is not a subtype
 *   of the written one; `path` says where.
 * - `special_opt_rule` (warning): the types are compatible only because
 *   Candid reads a mismatch under an `opt` as `null`, so a value at `path`
 *   decodes as `null`. One is reported at every such path along which no
 *   pair of types repeats: a shared type under each path that reaches it,
 *   and a recursive type up to where a path comes back round to a pair it
 *   already passed through.
 * - `resource_limit_exceeded`: as an error, the check of this method
 *   stopped at one of its bounds (`resource_limit.resource` is
 *   `check_depth` or `check_steps`) and fails closed. As a warning
 *   (`check_warnings`), the method has more `special_opt_rule` warnings
 *   than the 1,000 reported; its verdict is complete and stands.
 */
export interface CompatibilityDiagnostic {
  code:
    | "method_missing"
    | "mode_changed"
    | "method_incompatible"
    | "special_opt_rule"
    | "resource_limit_exceeded";
  severity: "error" | "warning";
  /** The written method the finding is about. */
  method: string;
  /**
   * Where in the method's type: `$args[i]` or `$results[i]`, then `.name`
   * (or `["name"]`, or `[id]` for a numeric label) per record field or
   * variant arm, `[*]` per vec element, `?` per opt content, and
   * `::args[i]`, `::results[i]` or `::name` into a func or service type.
   * Absent for `method_missing`, `mode_changed` and
   * `resource_limit_exceeded`.
   */
  path?: string;
  message: string;
  resource_limit?: ResourceLimitInfo;
}

/** A completed compatibility check. */
export interface CompatibilityReport {
  ok: true;
  /** True when no diagnostic is an error. Warnings do not clear it. */
  compatible: boolean;
  written: Identities;
  /**
   * The live interface's identities. Compare `interface_id` with the one
   * recorded when the written file was made to tell an unchanged interface
   * from one that changed compatibly.
   */
  live: Identities;
  /** By written method name, each method's errors before its warnings. */
  diagnostics: CompatibilityDiagnostic[];
}

/** A check that could not run: `input` names the side that failed. */
export interface CheckFailure extends Failure {
  input?: "written" | "live";
}

/**
 * Check that a live interface is still compatible with a written one: the
 * live service must be a Candid subtype of the written one. Every written
 * method must exist in the live service with the same mode, its arguments
 * contravariant and its results covariant; methods only the live service
 * has are ignored. A hand-written subset `.did` is as valid a written side
 * as a projection.
 *
 * @example
 * const report = await checkCompatible(writtenDid, liveDid);
 * if (report.ok && !report.compatible) {
 *   for (const item of report.diagnostics) console.error(item.method, item.code, item.path);
 * }
 */
export function checkCompatible(
  written: Sources,
  live: Sources,
): Promise<CompatibilityReport | CheckFailure>;

/**
 * What happened to one entry of `candid-core-cli gen`.
 *
 * - `written`: the entry generated and at least one of its two files was
 *   created or changed on disk.
 * - `unchanged`: the entry generated and both files already held exactly
 *   those bytes; nothing was written.
 * - `drifted`: `--check` only — at least one file is missing or differs from
 *   what the entry now generates; nothing was written.
 * - `failed`: the entry did not generate; `diagnostics` says why. A compile
 *   failure writes nothing for the entry.
 */
export type CliEntryStatus = "written" | "unchanged" | "drifted" | "failed";

/** One entry's part of the `--json` document. */
export interface CliEntryReport {
  /** The entry exactly as it was given on the command line. */
  entry: string;
  status: CliEntryStatus;
  /** The generated module's path: `-o` joined with `<stem>.ts`. */
  module: string;
  /** The contract envelope's path: `-o` joined with `<stem>.envelope.json`. */
  envelope: string;
  /**
   * What the module leaves out, exactly as `didToModule` returns it. Empty
   * for a `failed` entry and when nothing is omitted. Omissions never make an
   * entry fail.
   */
  omitted: Omission[];
  /**
   * The compiler's diagnostics for a `failed` entry, in the same shape the
   * library returns; empty otherwise. Failures that occur before compilation
   * use the same shape with codes this CLI originates:
   * `did_file_read_error`, `did_source_not_found`, `resource_limit_exceeded`,
   * `nondeterministic_output` and `internal_error` (which carries no `phase`).
   */
  diagnostics: Diagnostic[];
}

/**
 * The one document `candid-core-cli gen … --json` prints on stdout, and
 * nothing else: stderr stays empty. The process exit code is 0 exactly when
 * `ok` is true, and 1 otherwise. A usage error exits 64 and prints no
 * document.
 */
export interface CliReport {
  /**
   * The document's version. It changes only when an existing field's meaning
   * or shape changes; adding a field does not change it. Refuse a version
   * you do not know.
   */
  schemaVersion: 1;
  /** True when no entry is `failed` or `drifted`. Omissions do not clear it. */
  ok: boolean;
  /** Whether `--check` was given. */
  check: boolean;
  /** One report per entry, in command-line order. */
  entries: CliEntryReport[];
  /**
   * Every path `--check` found missing or different from what its entry now
   * generates, in entry order, module before envelope. Always empty without
   * `--check`, and for a `failed` entry.
   */
  drift: string[];
}
