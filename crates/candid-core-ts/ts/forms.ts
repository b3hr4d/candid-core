// Form-generation metadata derived from a schema — the issue #105 slice, and
// the first tool built on the structure the combinators deliberately carry.
// The output is a UI-agnostic form model: what to render, labeled how,
// constrained how — never React/DOM components. Framework bindings consume
// this model; they are not part of it.
//
// # The model mirrors the domain shapes
//
// Every combinator yields exactly one documented control node (the walk's
// switch is exhaustive with a `never` check, so a new combinator breaks the
// build here rather than silently rendering nothing):
//
// - fixed-width integers → `integer` with number-safe `min`/`max`;
//   `nat`/`int`/`nat64`/`int64` → `bigint` with bigint bounds where they
//   exist — flagged so a UI never routes them through a `number` input;
// - `opt` → `optional`, a presence toggle plus the inner node; a `record`'s
//   opt field is present-with-`null`, so the toggle maps to `null`, never to
//   a missing key;
// - `vec` → `list` with per-index nodes on demand; `blob` → `bytes`;
// - `record`/`unit` → `group`; tuples → `tuple`;
// - `variant` → `choice` among arms, tag-only arms needing no payload
//   editor;
// - `func` → `funcReference` (a `{ principal, method }` pair), `service` →
//   `serviceReference` (a principal) — the issue #104 value shapes;
// - `rec` → `lazy`: a form cannot eagerly expand `List`, so recursion
//   surfaces as expandable-on-demand.
//
// # Labels
//
// Schema keys already carry the rendered names (the caller-supplied name
// table was applied when the schema was built — the same `TsNames` rule the
// generator uses). An unnamed Candid label rendered by the `_id_` convention
// surfaces with `label` as that rendering and its numeric id in `numericId`,
// so UIs can choose their own presentation for unnamed fields.
//
// # Paths round-trip with validation
//
// Node paths use exactly `validate`'s grammar (`$.next.tail[2].tag`), so a
// validation issue's `path` addresses a form node directly: walk the model
// with `formNodeAt` and attach the message. Building the model never throws
// on any *schema this core constructs*; a foreign schema object is a
// programmer error and throws `TypeError`, the same stance `createActor`
// takes.

import type {
  AnySchema,
  BlobSchema,
  FieldSchemas,
  FuncSchema,
  OptSchema,
  PrimitiveSchema,
  RecordSchema,
  RecSchema,
  ServiceSchema,
  TupleSchema,
  UnitSchema,
  VariantSchema,
  VecSchema,
} from "./schema.ts";
import { numericKeyId } from "./labels.ts";

/** What every form node carries, wherever it sits. */
export interface FormCommon {
  /** `$`-rooted path in `validate`'s grammar. */
  readonly path: string;
  /** The rendered label (field key or arm tag); absent at the root. */
  readonly label?: string;
  /** The numeric Candid id when `label` is the `_id_` rendering. */
  readonly numericId?: number;
}

export interface FormArm {
  readonly tag: string;
  readonly numericId?: number;
  /** A tag-only arm (`null` payload) needs no payload editor. */
  readonly tagOnly: boolean;
  /** The payload editor for `{ tag, value }` arms; undefined when tagOnly. */
  payload(): FormNode | undefined;
}

export type FormControl =
  | { readonly control: "text" }
  | { readonly control: "checkbox" }
  | { readonly control: "integer"; readonly min: number; readonly max: number }
  | { readonly control: "bigint"; readonly min?: bigint; readonly max?: bigint }
  | { readonly control: "float"; readonly bits: 32 | 64 }
  | { readonly control: "principal" }
  /** `null` and `reserved`: nothing to edit, the value is fixed. */
  | { readonly control: "constant" }
  /** `empty`: uninhabited — a UI must not offer an input at all. */
  | { readonly control: "uninhabited" }
  | { readonly control: "optional"; inner(): FormNode }
  | { readonly control: "list"; item(index: number): FormNode }
  | { readonly control: "bytes" }
  | {
      readonly control: "group";
      readonly fieldLabels: readonly string[];
      readonly fields: readonly (() => FormNode)[];
    }
  | { readonly control: "tuple"; readonly length: number; element(index: number): FormNode }
  | { readonly control: "choice"; readonly arms: readonly FormArm[] }
  | { readonly control: "funcReference" }
  | { readonly control: "serviceReference" }
  | { readonly control: "lazy"; expand(): FormNode };

export type FormNode = FormCommon & FormControl;

const FIXED: { readonly [name: string]: readonly [number, number] } = {
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

const IDENTIFIER_KEY = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

function extendPath(path: string, segment: string | number): string {
  if (typeof segment === "number") {
    return `${path}[${segment}]`;
  }
  return IDENTIFIER_KEY.test(segment)
    ? `${path}.${segment}`
    : `${path}[${JSON.stringify(segment)}]`;
}

interface Site {
  readonly path: string;
  readonly label?: string;
  readonly numericId?: number;
}

function labeled(path: string, key: string): Site {
  const numericId = numericKeyId(key);
  return numericId === undefined
    ? { path: extendPath(path, key), label: key }
    : { path: extendPath(path, key), label: key, numericId };
}

/** Build the form model for a schema. The root's path is `$`. */
export function formModel(schema: AnySchema): FormNode {
  return build(schema, { path: "$" });
}

/**
 * The form node a validation issue's `path` addresses, expanding lazy nodes
 * along the way. `undefined` when the path does not address a node this
 * model describes (an unknown key, a malformed path).
 */
export function formNodeAt(root: FormNode, path: string): FormNode | undefined {
  let node: FormNode | undefined = root;
  let rest = path.startsWith("$") ? path.slice(1) : undefined;
  if (rest === undefined) {
    return undefined;
  }
  while (node !== undefined && rest.length > 0) {
    while (node.control === "lazy") {
      node = node.expand();
    }
    const step = takeSegment(rest);
    if (step === undefined) {
      return undefined;
    }
    const [segment, remaining] = step;
    rest = remaining;
    node = descend(node, segment);
  }
  while (node !== undefined && node.control === "lazy") {
    node = node.expand();
  }
  return node;
}

function takeSegment(rest: string): [string | number, string] | undefined {
  if (rest.startsWith(".")) {
    const match = /^\.([A-Za-z_$][A-Za-z0-9_$]*)/.exec(rest);
    if (match === null) {
      return undefined;
    }
    return [match[1], rest.slice(match[0].length)];
  }
  if (rest.startsWith("[")) {
    const numeric = /^\[([0-9]+)\]/.exec(rest);
    if (numeric !== null) {
      return [Number(numeric[1]), rest.slice(numeric[0].length)];
    }
    const quoted = /^\[("(?:[^"\\]|\\.)*")\]/.exec(rest);
    if (quoted !== null) {
      return [JSON.parse(quoted[1]) as string, rest.slice(quoted[0].length)];
    }
  }
  return undefined;
}

function descend(node: FormNode, segment: string | number): FormNode | undefined {
  switch (node.control) {
    case "optional":
      return descendInto(node.inner(), segment);
    case "list":
      return typeof segment === "number" ? node.item(segment) : undefined;
    case "tuple":
      return typeof segment === "number" && segment < node.length
        ? node.element(segment)
        : undefined;
    case "group": {
      if (typeof segment !== "string") {
        return undefined;
      }
      const index = node.fieldLabels.indexOf(segment);
      return index >= 0 ? node.fields[index]() : undefined;
    }
    case "choice": {
      // Issue paths inside variants address `.value` (and `.tag`); a form
      // consumer resolves them against the currently chosen arm, which the
      // model cannot know — so `.tag` addresses the choice itself and any
      // other segment is per-arm territory left to the consumer.
      return segment === "tag" ? node : undefined;
    }
    default:
      return undefined;
  }
}

function descendInto(node: FormNode, segment: string | number): FormNode | undefined {
  let current = node;
  while (current.control === "lazy") {
    current = current.expand();
  }
  return descend(current, segment);
}

// The typed union the switch is exhaustive over: a new combinator added to
// schema.ts must be added here, and the `never`-typed default makes the
// omission a compile error — the acceptance criterion's build-breakage.
/* eslint-disable @typescript-eslint/no-explicit-any */
type SchemaNode =
  | PrimitiveSchema<any>
  | OptSchema<any>
  | VecSchema<any>
  | BlobSchema
  | UnitSchema
  | RecordSchema<FieldSchemas>
  | TupleSchema<readonly AnySchema[]>
  | VariantSchema<FieldSchemas>
  | FuncSchema
  | ServiceSchema
  | RecSchema<any>;
/* eslint-enable @typescript-eslint/no-explicit-any */

function erased(schema: AnySchema): SchemaNode {
  if (
    typeof schema !== "object" ||
    schema === null ||
    typeof (schema as { kind?: unknown }).kind !== "string"
  ) {
    throw new TypeError("not a schema object");
  }
  return schema as unknown as SchemaNode;
}

function resolveArm(schema: AnySchema): SchemaNode {
  let node = erased(schema);
  for (let hops = 0; hops < 256; hops += 1) {
    if (node.kind !== "rec") {
      return node;
    }
    node = erased(node.body());
  }
  throw new TypeError("rec chain exceeds the depth limit");
}

function build(schema: AnySchema, site: Site): FormNode {
  const node = erased(schema);
  const common: FormCommon = site;
  switch (node.kind) {
    case "primitive":
      return { ...common, ...primitiveControl(node.primitive) };
    case "opt": {
      const inner = node.inner as AnySchema;
      // The domain shape is `T | null` with the property present, so the
      // presence toggle edits the same path.
      return {
        ...common,
        control: "optional",
        inner: () => build(inner, { path: site.path }),
      };
    }
    case "vec": {
      const inner = node.inner as AnySchema;
      return {
        ...common,
        control: "list",
        item: (index: number) => build(inner, { path: extendPath(site.path, index) }),
      };
    }
    case "blob":
      return { ...common, control: "bytes" };
    case "unit":
      return { ...common, control: "group", fields: [], fieldLabels: [] };
    case "record": {
      const fields = node.fields;
      const keys = Object.keys(fields);
      return {
        ...common,
        control: "group",
        fieldLabels: keys,
        fields: keys.map((key) => () => build(fields[key], labeled(site.path, key))),
      };
    }
    case "tuple": {
      const elements = node.elements;
      return {
        ...common,
        control: "tuple",
        length: elements.length,
        element: (index: number) =>
          build(elements[index], { path: extendPath(site.path, index) }),
      };
    }
    case "variant": {
      const arms = node.arms;
      return {
        ...common,
        control: "choice",
        arms: Object.keys(arms).map((tag): FormArm => {
          const payload = resolveArm(arms[tag]);
          const tagOnly = payload.kind === "primitive" && payload.primitive === "null";
          const numericId = numericKeyId(tag);
          return {
            tag,
            ...(numericId === undefined ? {} : { numericId }),
            tagOnly,
            payload: () =>
              tagOnly
                ? undefined
                : build(arms[tag], {
                    path: extendPath(site.path, "value"),
                    label: tag,
                  }),
          };
        }),
      };
    }
    case "func":
      return { ...common, control: "funcReference" };
    case "service":
      return { ...common, control: "serviceReference" };
    case "rec": {
      const body = node.body;
      return { ...common, control: "lazy", expand: () => build(body(), site) };
    }
    default:
      return unreachableKind(node);
  }
}

/** The exhaustiveness backstop the acceptance criterion asks for. */
function unreachableKind(node: never): never {
  throw new TypeError(
    `no form control for schema kind ${JSON.stringify((node as { kind: string }).kind)}`,
  );
}

function primitiveControl(name: string): FormControl {
  switch (name) {
    case "text":
      return { control: "text" };
    case "bool":
      return { control: "checkbox" };
    case "nat":
      return { control: "bigint", min: 0n };
    case "int":
      return { control: "bigint" };
    case "nat64":
      return { control: "bigint", min: 0n, max: NAT64_MAX };
    case "int64":
      return { control: "bigint", min: INT64_MIN, max: INT64_MAX };
    case "nat8":
    case "nat16":
    case "nat32":
    case "int8":
    case "int16":
    case "int32": {
      const [min, max] = FIXED[name];
      return { control: "integer", min, max };
    }
    case "float32":
      return { control: "float", bits: 32 };
    case "float64":
      return { control: "float", bits: 64 };
    case "principal":
      return { control: "principal" };
    case "null":
    case "reserved":
      return { control: "constant" };
    case "empty":
      return { control: "uninhabited" };
    default:
      throw new TypeError(`no form control for primitive ${JSON.stringify(name)}`);
  }
}
