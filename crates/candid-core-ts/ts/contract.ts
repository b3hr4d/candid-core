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
// - reference types build like everything else (issue #104): a func schema
//   carries its signature and describes `{ principal, method }` values, a
//   service schema carries its methods and describes principal values, and
//   the class actor root denotes its running service (init args are
//   install-time metadata). A class is legal *only* as the actor root: a
//   named declaration targeting a class node is refused, mirroring
//   candid-core's `class_not_actor_root` (issue #129). The actor interface,
//   when present, is built as a service schema and returned as `actor`.
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
import { candidLabelHash, isNumericShapedName } from "./labels.ts";

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
 *
 * Entries are hash-enforced (issue #103): a name must be the Candid preimage
 * of its id (`candidLabelHash(name) === id`), and `_N_`-shaped names are
 * refused — erased to a schema key they are indistinguishable from the
 * numeric-id rendering, and the codec derives wire ids from keys. A table
 * from real provenance always satisfies both; a lying one fails closed.
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

export type SchemaFromContractResult =
  | {
      readonly ok: true;
      /** One schema per declaration, in declaration order (issue #104: func and service kinds included; a class is never named — declarations targeting one are refused, issue #129). */
      readonly schemas: { readonly [name: string]: AnySchema };
      /** The actor interface as a service schema, when the document has one. */
      readonly actor?: AnySchema;
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

interface ParsedField {
  readonly id: number;
  readonly type: number;
}

interface ParsedMethod {
  readonly name: string;
  readonly func: number;
}

type ParsedNode =
  | { readonly kind: "primitive"; readonly primitive: string }
  | { readonly kind: "opt" | "vec"; readonly inner: number }
  | { readonly kind: "record" | "variant"; readonly fields: readonly ParsedField[] }
  | {
      readonly kind: "func";
      readonly args: readonly number[];
      readonly results: readonly number[];
      readonly mode: "update" | "query" | "composite_query" | "oneway";
    }
  | { readonly kind: "service"; readonly methods: readonly ParsedMethod[] }
  | { readonly kind: "class"; readonly init: readonly number[]; readonly service: number };

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
  /** A validated list of in-arena type refs, or undefined after issues. */
  const refList = (value: unknown, base: string): number[] | undefined => {
    if (!Array.isArray(value)) {
      push("invalid_contract_document", base, "expected an array of type references");
      return undefined;
    }
    const refs: number[] = [];
    let sound = true;
    for (let i = 0; i < value.length; i += 1) {
      totalFields += 1;
      if (totalFields > maxFields) {
        issues.push({
          code: "resource_limit_exceeded",
          path: base,
          message: `fields limit ${maxFields} exceeded`,
          resource_limit: { resource: "fields", limit: maxFields, observed: totalFields },
        });
        return undefined;
      }
      if (!validRef(value[i])) {
        push(
          "dangling_type_ref",
          `${base}[${i}]`,
          "type reference is outside the Contract arena",
        );
        sound = false;
        continue;
      }
      refs.push(value[i] as number);
    }
    return sound ? refs : undefined;
  };
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
      case "func": {
        // Reference types ship with issue #104: internals are parsed and
        // ref-checked like any other node.
        const args = refList(node.args, `${base}.args`);
        const results = refList(node.results, `${base}.results`);
        const mode = node.mode;
        if (
          mode !== "update" &&
          mode !== "query" &&
          mode !== "composite_query" &&
          mode !== "oneway"
        ) {
          push(
            "invalid_contract_document",
            `${base}.mode`,
            `unknown method mode ${JSON.stringify(mode)}`,
          );
          break;
        }
        // candid-core's oneway_has_results rule: a oneway function cannot
        // carry results — an actor built from one would await a reply the
        // call never provides.
        if (mode === "oneway" && Array.isArray(node.results) && node.results.length > 0) {
          push(
            "invalid_contract_document",
            `${base}.results`,
            "a oneway function has no results",
          );
          break;
        }
        if (args !== undefined && results !== undefined) {
          parsed[index] = { kind: "func", args, results, mode };
        }
        break;
      }
      case "service": {
        if (!Array.isArray(node.methods)) {
          push(
            "invalid_contract_document",
            `${base}.methods`,
            "a service node carries a methods array",
          );
          break;
        }
        const methods: ParsedMethod[] = [];
        let sound = true;
        const seenMethodNames = new Set<string>();
        for (let m = 0; m < node.methods.length; m += 1) {
          totalFields += 1;
          if (totalFields > maxFields) {
            issues.push({
              code: "resource_limit_exceeded",
              path: `${base}.methods`,
              message: `fields limit ${maxFields} exceeded`,
              resource_limit: { resource: "fields", limit: maxFields, observed: totalFields },
            });
            return { ok: false, issues };
          }
          const method: unknown = node.methods[m];
          const methodBase = `${base}.methods[${m}]`;
          if (
            !isObject(method) ||
            typeof method.name !== "string" ||
            typeof method.id !== "number" ||
            !Number.isInteger(method.id) ||
            typeof method.function !== "number" ||
            !Number.isInteger(method.function)
          ) {
            push(
              "invalid_contract_document",
              methodBase,
              "a service method is { name, id, function }",
            );
            sound = false;
            continue;
          }
          // candid-core's empty_method_name rule; hash("") is 0, so the
          // hash-consistency check below cannot catch this on its own.
          if (method.name.length === 0) {
            push(
              "invalid_contract_document",
              `${methodBase}.name`,
              "a service method name is non-empty",
            );
            sound = false;
            continue;
          }
          // The id is the Candid hash of the name — the same invariant the
          // name table enforces, and what makes the wire encoding honest.
          if (candidLabelHash(method.name) !== method.id) {
            push(
              "invalid_contract_document",
              `${methodBase}.id`,
              `${JSON.stringify(method.name)} hashes to ${candidLabelHash(method.name)}, not ${method.id}`,
            );
            sound = false;
            continue;
          }
          if (seenMethodNames.has(method.name)) {
            push(
              "invalid_contract_document",
              `${methodBase}.name`,
              "duplicate service method name",
            );
            sound = false;
            continue;
          }
          seenMethodNames.add(method.name);
          if (!validRef(method.function)) {
            push(
              "dangling_type_ref",
              `${methodBase}.function`,
              "type reference is outside the Contract arena",
            );
            sound = false;
            continue;
          }
          methods.push({ name: method.name, func: method.function });
        }
        if (sound) {
          parsed[index] = { kind: "service", methods };
        }
        break;
      }
      case "class": {
        const init = refList(node.init, `${base}.init`);
        if (
          typeof node.service !== "number" ||
          !Number.isInteger(node.service) ||
          !validRef(node.service)
        ) {
          push(
            "dangling_type_ref",
            `${base}.service`,
            "type reference is outside the Contract arena",
          );
          break;
        }
        if (init !== undefined) {
          parsed[index] = { kind: "class", init, service: node.service };
        }
        break;
      }
      default:
        push(
          "invalid_contract_document",
          `${base}.kind`,
          `unknown type node kind ${JSON.stringify(node.kind)}`,
        );
    }
  }

  // Pass two, on sound nodes only: the one-hop node checks the generator
  // performs during rendering — collapsing opts and the declaration-only rule
  // for classes.
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
      if (inner.kind === "class") {
        push(
          "unsupported_construct",
          `${base}.inner`,
          `a \`class\` type nests inside a supported type; classes exist only at declaration position`,
        );
      }
    } else if (node.kind === "record" || node.kind === "variant") {
      for (let j = 0; j < node.fields.length; j += 1) {
        const inner = parsed[node.fields[j].type];
        if (inner !== undefined && inner.kind === "class") {
          push(
            "unsupported_construct",
            `${base}.fields[${j}].type`,
            `a \`class\` type nests inside a supported type; classes exist only at declaration position`,
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
    // Hash consistency is the wire-correctness invariant (issue #103): the
    // codec derives every field's wire id from its rendered key, so a name
    // that does not hash back to its entry's id would silently encode a
    // wrong id. `_N_`-shaped names are refused outright — erased to a key,
    // they are indistinguishable from the numeric-id rendering convention.
    const name = entry[2];
    if (isNumericShapedName(name)) {
      push(
        "invalid_name_table",
        `$.names[${index}]`,
        "a name shaped like the _N_ id rendering is reserved",
      );
      continue;
    }
    if (candidLabelHash(name) !== entry[1]) {
      push(
        "invalid_name_table",
        `$.names[${index}]`,
        `${JSON.stringify(name)} hashes to ${candidLabelHash(name)}, not ${entry[1]}`,
      );
      continue;
    }
    nameTable.set(`${entry[0]}:${entry[1]}`, name);
  }

  const fieldKey = (container: number, id: number): string =>
    nameTable.get(`${container}:${id}`) ?? `_${id}_`;

  // Reference-type structural constraints (issue #104): a service method
  // must denote a func node, and a class must denote a service node — the
  // same constraints candid-core's own Contract validation enforces, so a
  // hand-built document cannot smuggle a mis-kinded reference past the
  // builder.
  for (let index = 0; index < types.length; index += 1) {
    const node = parsed[index];
    if (node === undefined) {
      continue;
    }
    if (node.kind === "service") {
      for (let m = 0; m < node.methods.length; m += 1) {
        const target = parsed[node.methods[m].func];
        if (target === undefined || target.kind !== "func") {
          push(
            "invalid_contract_document",
            `$.types[${index}].methods[${m}].function`,
            "a service method must denote a func type",
          );
        }
      }
    } else if (node.kind === "class") {
      const target = parsed[node.service];
      if (target === undefined || target.kind !== "service") {
        push(
          "invalid_contract_document",
          `$.types[${index}].service`,
          "a class must denote a service type",
        );
      }
    }
  }
  if (issues.length > 0) {
    return { ok: false, issues };
  }

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
      case "func":
        return c.func(node.args.map(lazy), node.results.map(lazy), node.mode);
      case "service": {
        const methods: { [name: string]: AnySchema } = Object.create(null);
        for (const method of node.methods) {
          methods[method.name] = lazy(method.func);
        }
        return c.service(methods);
      }
      case "class":
        // A class-typed reference denotes the running service (issue #104):
        // init args are install-time metadata a call surface never consumes.
        return schemaAt(node.service);
      default:
        // Unreachable by construction; a rec thunk that somehow reached an
        // unknown node still fails closed in `validate`.
        return { kind: "unsupported" } as AnySchema;
    }
  };

  const schemas: { [name: string]: AnySchema } = Object.create(null);
  for (const declaration of parsedDeclarations) {
    // The same shape the generator emits: every declaration wrapped in
    // `c.rec`, so aliases of one node share the memoized structure beneath.
    // Reference kinds build like everything else since issue #104. A
    // class-targeting declaration still builds here, but the placement walk
    // below refuses the document before the map can be returned (#129).
    schemas[declaration.name] = c.rec(() => schemaAt(declaration.type));
  }

  // candid-core restricts class nodes to the actor root
  // (`class_not_actor_root`); mirror it so the two loaders refuse the same
  // documents.
  const actorClassTarget = (() => {
    const raw: unknown = contract.actor;
    return isObject(raw) && raw.kind === "class" && typeof raw.class === "number"
      ? (raw.class as number)
      : undefined;
  })();
  const isClassRef = (ref: number): boolean => parsed[ref]?.kind === "class";
  for (let index = 0; index < types.length; index += 1) {
    const node = parsed[index];
    if (node === undefined) {
      continue;
    }
    if (node.kind === "class" && index !== actorClassTarget) {
      push(
        "invalid_contract_document",
        `$.types[${index}]`,
        "class nodes are only valid as the top-level class actor root",
      );
      return { ok: false, issues };
    }
    // candid-core's class_not_first_class_type rule: no type edge — func
    // args/results and a class's own init/service included — may target a
    // class, actor-root or not. Opt/vec inners and record/variant fields
    // are already refused by the pass-two nested checks.
    const edges: readonly (readonly [number, string])[] =
      node.kind === "func"
        ? [
            ...node.args.map((ref, i) => [ref, `$.types[${index}].args[${i}]`] as const),
            ...node.results.map(
              (ref, i) => [ref, `$.types[${index}].results[${i}]`] as const,
            ),
          ]
        : node.kind === "class"
          ? [
              ...node.init.map((ref, i) => [ref, `$.types[${index}].init[${i}]`] as const),
              [node.service, `$.types[${index}].service`] as const,
            ]
          : node.kind === "service"
            ? node.methods.map(
                (method, i) =>
                  [method.func, `$.types[${index}].methods[${i}].function`] as const,
              )
            : [];
    for (const [ref, path] of edges) {
      if (isClassRef(ref)) {
        push(
          "invalid_contract_document",
          path,
          "a class is not a first-class type; no type edge may target one",
        );
        return { ok: false, issues };
      }
    }
  }
  // The declaration half of the same rule (`class_not_actor_root`): a named
  // declaration must never target a class node. The actor-root exemption
  // above covers the node's existence, not names — candid-core refuses
  // `X : class` even when that class is the actor root (issue #129).
  // Reaching this point implies zero issues so far, so `parsedDeclarations`
  // aligns index-for-index with `$.declarations`.
  for (let index = 0; index < parsedDeclarations.length; index += 1) {
    if (isClassRef(parsedDeclarations[index].type)) {
      push(
        "invalid_contract_document",
        `$.declarations[${index}].type`,
        "a named declaration must not target a class node",
      );
      return { ok: false, issues };
    }
  }

  // The actor interface, when the document carries one: a service schema,
  // with a class actor unwrapped to its running service. Fail closed on any
  // shape the canonical format does not produce.
  let actor: AnySchema | undefined;
  if (contract.actor !== undefined && contract.actor !== null) {
    const raw: unknown = contract.actor;
    if (
      !isObject(raw) ||
      (raw.kind !== "service" && raw.kind !== "class") ||
      typeof raw[raw.kind as "service" | "class"] !== "number"
    ) {
      push("invalid_contract_document", "$.actor", "an actor is { kind, service|class }");
      return { ok: false, issues };
    }
    const target = raw[raw.kind as "service" | "class"] as number;
    if (!validRef(target)) {
      push(
        "dangling_type_ref",
        `$.actor.${raw.kind}`,
        "type reference is outside the Contract arena",
      );
      return { ok: false, issues };
    }
    const node = sound[target];
    const resolved = node.kind === "class" ? sound[node.service] : node;
    if (
      (raw.kind === "service" && node.kind !== "service" && node.kind !== "class") ||
      resolved.kind !== "service"
    ) {
      push(
        "invalid_contract_document",
        `$.actor.${raw.kind}`,
        "an actor must denote a service (or a class of one)",
      );
      return { ok: false, issues };
    }
    actor = c.rec(() => schemaAt(target));
  }

  return { ok: true, schemas, actor };
}
