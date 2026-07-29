// Dynamic schema construction from a canonical Contract JSON document — the
// second half of issue #102. `schemaFromContract` walks the arena (`types`,
// `declarations`, field `id`s) and builds the same `c.*` combinator objects
// the generated builders construct, so any contract can be validated at
// runtime without generated code.
//
// # The mapping is the generator's mapping
//
// Every owner-reviewed decision in `candid-core-ts`'s Rust emitter holds here,
// checked by the golden cross-check test:
//
// - anonymous `vec nat8` (an inner node no declaration names) is a blob;
// - a record whose ids are exactly `0..n-1` is a tuple; an empty record is
//   the unit schema; label text comes from a caller-supplied name table and
//   an unnamed field renders by the `_id_` convention;
// - an `opt` whose inner node is `opt`, `null`, or `reserved` is rejected
//   with `unrepresentable_option` — `T | null` cannot carry `None` versus
//   `Some(None)`, and the check is on the node, so an aliased opt collapses
//   just as surely;
// - top-level `func`/`service`/`class` declarations are skipped and reported
//   in `deferred`; one *nested* in a supported type fails closed with
//   `unsupported_construct` (issue #38 defers them). An actor interface is
//   likewise deferred and only reported via `actorDeferred`.
//
// One deliberate divergence: declaration names are not required to be
// TypeScript identifiers. The generator emits source text and must refuse
// `type delete`; this loader builds a map, where any name is a valid key.
//
// # Untrusted input, bounded and fail closed
//
// The document is data from outside: nothing is trusted, nothing throws, and
// every malformed shape is an issue in the same `{code, path, message,
// resource_limit?}` shape `validate.ts` documents, with codes reused from
// candid-core's own Contract validation where the condition is the same
// (`unsupported_contract_format`, `unsupported_format_version`,
// `unsupported_semantics_profile`, `unsupported_canonicalization_profile`,
// `dangling_type_ref`, `duplicate_field_id`, `empty_declaration_name`,
// `duplicate_declaration_name`, `resource_limit_exceeded`). Arena size,
// total field count, and declaration count are capped by the same defaults
// as candid-core's `Limits` (`max_type_nodes` 100_000, `max_fields` 500_000,
// `max_declarations` 100_000), and the caller's name table by its own
// entry cap. Checks run in one pass over the arena before anything is
// built, so construction is non-recursive: every edge becomes a lazy
// `c.rec` thunk, memoized per node, and an adversarially deep type graph
// costs O(1) construction depth. Unknown node kinds and unknown primitive
// names fail closed (`invalid_contract_document`): a future format revision
// must be adopted deliberately, not half-read. Nothing throws even when the
// document itself fights inspection (accessors, Proxies): the whole
// construction runs behind a fail-closed choke point that converts such an
// exception into an `invalid_contract_document` issue. Schema maps — record
// fields, variant arms, the schemas-by-name result — are built with a null
// prototype, so a field or declaration legitimately named `__proto__` is an
// ordinary own key, never a prototype write.
//
// What is deliberately *not* checked: the `identities` hashes. Verifying
// `candid-core:contract:v1` requires the canonicalization procedure, which is
// candid-core's job (and no unkeyed content ID authenticates itself). The
// four format/profile markers are required to match exactly; identities and
// producer are carried by canonical documents but not consulted here.

import { c, type AnySchema } from "./schema.ts";
import type { ResourceLimitInfo } from "./validate.ts";

/** Stable failure codes for contract loading. Closed: additions are API changes. */
export type ContractIssueCode =
  | "invalid_contract_document"
  | "unsupported_contract_format"
  | "unsupported_format_version"
  | "unsupported_semantics_profile"
  | "unsupported_canonicalization_profile"
  | "dangling_type_ref"
  | "unrepresentable_option"
  | "unsupported_construct"
  | "duplicate_field_id"
  | "duplicate_field_name"
  | "empty_declaration_name"
  | "duplicate_declaration_name"
  | "invalid_name_table"
  | "resource_limit_exceeded";

export interface ContractIssue {
  readonly code: ContractIssueCode;
  /** `$`-rooted path into the contract document (or `$.names[…]`). */
  readonly path: string;
  readonly message: string;
  readonly resource_limit?: ResourceLimitInfo | ContractResourceLimitInfo;
}

export interface ContractResourceLimitInfo {
  readonly resource: "type_nodes" | "fields" | "declarations" | "name_table_entries";
  readonly limit: number;
  readonly observed: number;
}

/**
 * Caller-supplied field label text as `(container node, label id, name)`
 * triples — the same table `TsNames::from_pairs` takes in Rust, because the
 * semantic Contract stores only authoritative label ids. A field with no
 * entry renders by the ecosystem's `_id_` convention.
 */
export type FieldNameEntry = readonly [
  container: number,
  id: number,
  name: string,
];

export interface ContractSchemaOptions {
  readonly names?: readonly FieldNameEntry[];
  /** Arena size cap, mirroring `Limits::max_type_nodes`. */
  readonly maxTypeNodes?: number;
  /** Total field cap across all nodes, mirroring `Limits::max_fields`. */
  readonly maxFields?: number;
  /** Declaration count cap, mirroring `Limits::max_declarations`. */
  readonly maxDeclarations?: number;
}

export interface DeferredDeclaration {
  readonly name: string;
  readonly kind: "func" | "service" | "class";
}

export type SchemaFromContractResult =
  | {
      readonly ok: true;
      /** One schema per supported declaration, in declaration order. */
      readonly schemas: { readonly [name: string]: AnySchema };
      /** Top-level func/service/class declarations, skipped like the generator skips them. */
      readonly deferred: readonly DeferredDeclaration[];
      /** True when the document carries an actor interface (deferred wholesale). */
      readonly actorDeferred: boolean;
    }
  | { readonly ok: false; readonly issues: readonly ContractIssue[] };

export const DEFAULT_MAX_TYPE_NODES = 100_000;
export const DEFAULT_MAX_FIELDS = 500_000;
export const DEFAULT_MAX_DECLARATIONS = 100_000;
/**
 * The name table is caller-side input with no candid-core counterpart; it
 * addresses fields, so it shares the field cap's magnitude.
 */
export const DEFAULT_MAX_NAME_TABLE_ENTRIES = 500_000;

const PRIMITIVES: { readonly [name: string]: AnySchema } = {
  null: c.null,
  bool: c.bool,
  nat: c.nat,
  int: c.int,
  nat8: c.nat8,
  nat16: c.nat16,
  nat32: c.nat32,
  nat64: c.nat64,
  int8: c.int8,
  int16: c.int16,
  int32: c.int32,
  int64: c.int64,
  float32: c.float32,
  float64: c.float64,
  text: c.text,
  reserved: c.reserved,
  // The one cast in the table: `Schema<never>` is the single schema the
  // `Schema<any>` wildcard cannot hold (`any` is assignable to everything
  // except `never` in the invariant phantom's parameter).
  empty: c.empty as AnySchema,
  principal: c.principal,
};

const DEFERRED_KINDS = new Set(["func", "service", "class"]);

interface ParsedField {
  readonly id: number;
  readonly type: number;
}

type ParsedNode =
  | { readonly kind: "primitive"; readonly primitive: string }
  | { readonly kind: "opt" | "vec"; readonly inner: number }
  | { readonly kind: "record" | "variant"; readonly fields: readonly ParsedField[] }
  | { readonly kind: "func" | "service" | "class" };

function hasOwn(target: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(target, key);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Build `Schema` objects for every supported declaration of a Contract. */
export function schemaFromContract(
  contract: unknown,
  options: ContractSchemaOptions = {},
): SchemaFromContractResult {
  // The fail-closed choke point behind the no-throw claim: a document that is
  // not plain parsed JSON — accessors that throw, hostile Proxy traps — blows
  // up inside one of the reads below, and the failure must be an issue, not
  // an escaping exception. The lazy thunks built on success only ever read
  // the plain parsed structures, never the raw document, so they cannot
  // throw later.
  try {
    return buildFromContract(contract, options);
  } catch {
    return {
      ok: false,
      issues: [
        {
          code: "invalid_contract_document",
          path: "$",
          message: "the document threw while being inspected",
        },
      ],
    };
  }
}

function buildFromContract(
  contract: unknown,
  options: ContractSchemaOptions,
): SchemaFromContractResult {
  const issues: ContractIssue[] = [];
  const push = (code: ContractIssueCode, path: string, message: string) => {
    issues.push({ code, path, message });
  };

  if (!isObject(contract)) {
    push("invalid_contract_document", "$", "a Contract document is a JSON object");
    return { ok: false, issues };
  }

  // The four self-identification markers, checked exactly the way candid-core
  // checks them. A document claiming a different format or profile is not
  // half-read on the assumption it is compatible.
  if (contract.format !== "candid-core") {
    push("unsupported_contract_format", "$.format", 'format must be "candid-core"');
  }
  if (contract.format_version !== 1) {
    push("unsupported_format_version", "$.format_version", "format_version must be 1");
  }
  if (contract.semantics_profile !== "candid-1") {
    push(
      "unsupported_semantics_profile",
      "$.semantics_profile",
      'semantics_profile must be "candid-1"',
    );
  }
  if (contract.canonicalization_profile !== "candid-core-canon-1") {
    push(
      "unsupported_canonicalization_profile",
      "$.canonicalization_profile",
      'canonicalization_profile must be "candid-core-canon-1"',
    );
  }

  const maxTypeNodes = options.maxTypeNodes ?? DEFAULT_MAX_TYPE_NODES;
  const maxFields = options.maxFields ?? DEFAULT_MAX_FIELDS;

  const rawTypes = contract.types;
  const rawDeclarations = contract.declarations;
  if (!Array.isArray(rawTypes)) {
    push("invalid_contract_document", "$.types", "types must be an array");
  }
  if (!Array.isArray(rawDeclarations)) {
    push("invalid_contract_document", "$.declarations", "declarations must be an array");
  }
  if (issues.length > 0) {
    return { ok: false, issues };
  }
  const types = rawTypes as unknown[];
  const declarations = rawDeclarations as unknown[];

  if (types.length > maxTypeNodes) {
    issues.push({
      code: "resource_limit_exceeded",
      path: "$.types",
      message: `type_nodes limit ${maxTypeNodes} exceeded (observed ${types.length})`,
      resource_limit: {
        resource: "type_nodes",
        limit: maxTypeNodes,
        observed: types.length,
      },
    });
    return { ok: false, issues };
  }

  // The third mirrored cap: a tiny arena with millions of uniquely named
  // declarations must fail closed exactly as candid-core's own validator
  // fails it (resource "declarations", `Limits::max_declarations`).
  const maxDeclarations = options.maxDeclarations ?? DEFAULT_MAX_DECLARATIONS;
  if (declarations.length > maxDeclarations) {
    issues.push({
      code: "resource_limit_exceeded",
      path: "$.declarations",
      message: `declarations limit ${maxDeclarations} exceeded (observed ${declarations.length})`,
      resource_limit: {
        resource: "declarations",
        limit: maxDeclarations,
        observed: declarations.length,
      },
    });
    return { ok: false, issues };
  }

  const validRef = (value: unknown): value is number =>
    typeof value === "number" &&
    Number.isInteger(value) &&
    value >= 0 &&
    value < types.length;

  // Pass one: parse every arena node structurally. Nothing recursive — each
  // node is examined alone, and edges are checked as bare indices.
  const parsed: (ParsedNode | undefined)[] = new Array(types.length);
  let totalFields = 0;
  for (let index = 0; index < types.length; index += 1) {
    const base = `$.types[${index}]`;
    const node = types[index];
    if (!isObject(node) || typeof node.kind !== "string") {
      push("invalid_contract_document", base, "a type node is an object with a kind");
      continue;
    }
    switch (node.kind) {
      case "primitive": {
        const primitive = node.primitive;
        if (typeof primitive !== "string" || !hasOwn(PRIMITIVES, primitive)) {
          push(
            "invalid_contract_document",
            `${base}.primitive`,
            "unknown primitive type",
          );
          continue;
        }
        parsed[index] = { kind: "primitive", primitive };
        break;
      }
      case "opt":
      case "vec": {
        if (!validRef(node.inner)) {
          push(
            "dangling_type_ref",
            `${base}.inner`,
            "type reference is outside the Contract arena",
          );
          continue;
        }
        parsed[index] = { kind: node.kind, inner: node.inner };
        break;
      }
      case "record":
      case "variant": {
        if (!Array.isArray(node.fields)) {
          push("invalid_contract_document", `${base}.fields`, "fields must be an array");
          continue;
        }
        totalFields += node.fields.length;
        if (totalFields > maxFields) {
          issues.push({
            code: "resource_limit_exceeded",
            path: `${base}.fields`,
            message: `fields limit ${maxFields} exceeded (observed ${totalFields})`,
            resource_limit: {
              resource: "fields",
              limit: maxFields,
              observed: totalFields,
            },
          });
          return { ok: false, issues };
        }
        const fields: ParsedField[] = [];
        const seenIds = new Set<number>();
        let sound = true;
        for (let j = 0; j < node.fields.length; j += 1) {
          const field = node.fields[j];
          const fieldBase = `${base}.fields[${j}]`;
          if (
            !isObject(field) ||
            typeof field.id !== "number" ||
            !Number.isInteger(field.id) ||
            field.id < 0 ||
            field.id > 0xffff_ffff
          ) {
            push(
              "invalid_contract_document",
              `${fieldBase}.id`,
              "a field id is a u32 Candid label id",
            );
            sound = false;
            continue;
          }
          if (seenIds.has(field.id)) {
            push("duplicate_field_id", `${fieldBase}.id`, "field id repeats in this node");
            sound = false;
            continue;
          }
          seenIds.add(field.id);
          if (!validRef(field.type)) {
            push(
              "dangling_type_ref",
              `${fieldBase}.type`,
              "type reference is outside the Contract arena",
            );
            sound = false;
            continue;
          }
          fields.push({ id: field.id, type: field.type });
        }
        if (sound) {
          parsed[index] = { kind: node.kind, fields };
        }
        break;
      }
      case "func":
      case "service":
      case "class":
        // Deferred kinds are never built, so their internals are not parsed —
        // the generator never looks inside them either.
        parsed[index] = { kind: node.kind };
        break;
      default:
        push(
          "invalid_contract_document",
          `${base}.kind`,
          `unknown type node kind ${JSON.stringify(node.kind)}`,
        );
    }
  }

  // Pass two, on sound nodes only: the one-hop node checks the generator
  // performs during rendering — collapsing opts and supported→deferred edges.
  for (let index = 0; index < types.length; index += 1) {
    const node = parsed[index];
    if (node === undefined) {
      continue;
    }
    const base = `$.types[${index}]`;
    if (node.kind === "opt" || node.kind === "vec") {
      const inner = parsed[node.inner];
      if (inner === undefined) {
        continue;
      }
      if (node.kind === "opt") {
        const collapsing =
          inner.kind === "opt"
            ? "opt"
            : inner.kind === "primitive" &&
                (inner.primitive === "null" || inner.primitive === "reserved")
              ? inner.primitive
              : undefined;
        if (collapsing !== undefined) {
          push(
            "unrepresentable_option",
            `${base}.inner`,
            `opt wraps \`${collapsing}\`: \`T | null\` cannot distinguish None from Some(None) there`,
          );
          continue;
        }
      }
      if (DEFERRED_KINDS.has(inner.kind)) {
        push(
          "unsupported_construct",
          `${base}.inner`,
          `a \`${inner.kind}\` type nests inside a supported type; func/service/class are deferred (issue #38)`,
        );
      }
    } else if (node.kind === "record" || node.kind === "variant") {
      for (let j = 0; j < node.fields.length; j += 1) {
        const inner = parsed[node.fields[j].type];
        if (inner !== undefined && DEFERRED_KINDS.has(inner.kind)) {
          push(
            "unsupported_construct",
            `${base}.fields[${j}].type`,
            `a \`${inner.kind}\` type nests inside a supported type; func/service/class are deferred (issue #38)`,
          );
        }
      }
    }
  }

  // Declarations: names bind schemas, so emptiness and duplication are the
  // same defects candid-core rejects, under the same codes.
  interface ParsedDeclaration {
    readonly name: string;
    readonly type: number;
  }
  const parsedDeclarations: ParsedDeclaration[] = [];
  const seenNames = new Set<string>();
  for (let index = 0; index < declarations.length; index += 1) {
    const base = `$.declarations[${index}]`;
    const declaration = declarations[index];
    if (!isObject(declaration) || typeof declaration.name !== "string") {
      push("invalid_contract_document", base, "a declaration is { name, type }");
      continue;
    }
    if (declaration.name.length === 0) {
      push("empty_declaration_name", `${base}.name`, "declaration name must not be empty");
      continue;
    }
    if (seenNames.has(declaration.name)) {
      push(
        "duplicate_declaration_name",
        `${base}.name`,
        `declaration name ${JSON.stringify(declaration.name)} repeats`,
      );
      continue;
    }
    seenNames.add(declaration.name);
    if (!validRef(declaration.type)) {
      push(
        "dangling_type_ref",
        `${base}.type`,
        "type reference is outside the Contract arena",
      );
      continue;
    }
    parsedDeclarations.push({ name: declaration.name, type: declaration.type });
  }

  // The caller's name table, validated like any other input — its entry
  // count included, so a corrupted table cannot amplify into an unbounded
  // issue list or Map.
  const nameTable = new Map<string, string>();
  const rawNames = options.names ?? [];
  if (rawNames.length > DEFAULT_MAX_NAME_TABLE_ENTRIES) {
    issues.push({
      code: "resource_limit_exceeded",
      path: "$.names",
      message: `name_table_entries limit ${DEFAULT_MAX_NAME_TABLE_ENTRIES} exceeded (observed ${rawNames.length})`,
      resource_limit: {
        resource: "name_table_entries",
        limit: DEFAULT_MAX_NAME_TABLE_ENTRIES,
        observed: rawNames.length,
      },
    });
    return { ok: false, issues };
  }
  for (let index = 0; index < rawNames.length; index += 1) {
    const entry: unknown = rawNames[index];
    if (
      !Array.isArray(entry) ||
      entry.length !== 3 ||
      typeof entry[0] !== "number" ||
      !Number.isInteger(entry[0]) ||
      entry[0] < 0 ||
      typeof entry[1] !== "number" ||
      !Number.isInteger(entry[1]) ||
      entry[1] < 0 ||
      typeof entry[2] !== "string"
    ) {
      push(
        "invalid_name_table",
        `$.names[${index}]`,
        "a name table entry is [container, id, name]",
      );
      continue;
    }
    nameTable.set(`${entry[0]}:${entry[1]}`, entry[2]);
  }

  const fieldKey = (container: number, id: number): string =>
    nameTable.get(`${container}:${id}`) ?? `_${id}_`;

  // Rendered keys must be unique per node: two ids mapping to one supplied
  // name (or a supplied name colliding with an `_id_` rendering) would
  // silently drop a field from the built record. Fail closed instead. The
  // check skips exactly the nodes whose build never renders a key — empty
  // records (unit) and tuple-shaped records — because the generator ignores
  // name collisions there too: a positional tuple has no keys to collide.
  for (let index = 0; index < types.length; index += 1) {
    const node = parsed[index];
    if (node === undefined || (node.kind !== "record" && node.kind !== "variant")) {
      continue;
    }
    if (
      node.kind === "record" &&
      node.fields.every((field, position) => field.id === position)
    ) {
      continue;
    }
    const seenKeys = new Set<string>();
    for (let j = 0; j < node.fields.length; j += 1) {
      const key = fieldKey(index, node.fields[j].id);
      if (seenKeys.has(key)) {
        push(
          "duplicate_field_name",
          `$.types[${index}].fields[${j}]`,
          `field key ${JSON.stringify(key)} repeats in this node`,
        );
      }
      seenKeys.add(key);
    }
  }

  if (issues.length > 0) {
    return { ok: false, issues };
  }

  // Everything below runs on a fully validated arena. Construction mirrors
  // the generator: one schema per node, every edge a lazy memoized thunk, so
  // recursion terminates the same way it terminates in the generated code —
  // through `c.rec`, with `validate`'s depth limit as the runtime bound.
  const sound = parsed as ParsedNode[];

  // The generator's `first_names`: the first declaration name for each node
  // decides the blob rule ("an element type the Contract declares by name is
  // a deliberate abstraction and keeps its name").
  const declared = new Set<number>();
  for (const declaration of parsedDeclarations) {
    declared.add(declaration.type);
  }

  const memo: (AnySchema | undefined)[] = new Array(types.length);
  const schemaAt = (ref: number): AnySchema => {
    const existing = memo[ref];
    if (existing !== undefined) {
      return existing;
    }
    const built = build(ref);
    memo[ref] = built;
    return built;
  };
  const lazy = (ref: number): AnySchema => c.rec(() => schemaAt(ref));

  const build = (ref: number): AnySchema => {
    const node = sound[ref];
    switch (node.kind) {
      case "primitive":
        return PRIMITIVES[node.primitive];
      case "opt":
        return c.opt(lazy(node.inner));
      case "vec": {
        const inner = sound[node.inner];
        if (
          !declared.has(node.inner) &&
          inner.kind === "primitive" &&
          inner.primitive === "nat8"
        ) {
          return c.blob();
        }
        return c.vec(lazy(node.inner));
      }
      case "record": {
        if (node.fields.length === 0) {
          return c.unit();
        }
        if (node.fields.every((field, position) => field.id === position)) {
          return c.tuple(node.fields.map((field) => lazy(field.type)));
        }
        // Null-prototype maps throughout: on a plain `{}`, assigning the key
        // "__proto__" invokes the inherited prototype setter and silently
        // drops the field — a fail-open for a legitimately quoted Candid
        // name. With no prototype, every key is an ordinary own property.
        const fields: { [key: string]: AnySchema } = Object.create(null);
        for (const field of node.fields) {
          fields[fieldKey(ref, field.id)] = lazy(field.type);
        }
        return c.record(fields);
      }
      case "variant": {
        const arms: { [key: string]: AnySchema } = Object.create(null);
        for (const field of node.fields) {
          arms[fieldKey(ref, field.id)] = lazy(field.type);
        }
        return c.variant(arms);
      }
      default:
        // Unreachable: deferred nodes are never built (declarations of these
        // kinds are skipped, and nested ones failed closed above). A `rec`
        // thunk that somehow reached one would still fail closed in
        // `validate` as an unsupported schema.
        return { kind: "unsupported" } as AnySchema;
    }
  };

  const schemas: { [name: string]: AnySchema } = Object.create(null);
  const deferred: DeferredDeclaration[] = [];
  for (const declaration of parsedDeclarations) {
    const node = sound[declaration.type];
    if (node.kind === "func" || node.kind === "service" || node.kind === "class") {
      deferred.push({ name: declaration.name, kind: node.kind });
      continue;
    }
    // The same shape the generator emits: every declaration wrapped in
    // `c.rec`, so aliases of one node share the memoized structure beneath.
    schemas[declaration.name] = c.rec(() => schemaAt(declaration.type));
  }

  return {
    ok: true,
    schemas,
    deferred,
    actorDeferred: contract.actor !== undefined && contract.actor !== null,
  };
}
