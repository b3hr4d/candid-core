// Candid interface emission: from the registered server functions back to a
// `.did` service document.
//
// This is the closing half of the prototype's loop with candid-core. The
// schemas came *from* a Contract (generated builders); the framework derives
// the canister's public interface from what the app registered, emits it as
// Candid source, and candid-core compiles that back into a canonical
// Contract whose `interface_id` identifies the deployed wire interface.
// Interface drift between two builds is then a comparison of two ids, not a
// text diff.
//
// Types are emitted inline except where recursion forces a name: semantic
// Contract identities cover the canonical type graph, not declaration names,
// so an inline spelling and a named spelling of the same interface share
// their `interface_id` — the integration test proves exactly that with the
// real compiler.
//
// Determinism: methods are emitted sorted by name, synthetic `rec<n>` names
// are assigned in first-detection order of that traversal, and nothing
// depends on time, environment, or map iteration order. Two emissions of the
// same app are byte-identical.
import { CandidStartError } from "./errors.ts";
import { ownEntry, resolveShape, shapeOf, type AnySchema } from "./walk.ts";

export interface DidMethodSpec {
  readonly name: string;
  readonly mode: "query" | "update";
  /** `null` emits a zero-argument method `() -> …`. */
  readonly input: AnySchema | null;
  readonly output: AnySchema;
}

// Candid keywords and primitive type names; a label spelled like one of these
// is emitted quoted. Over-quoting is always valid, under-quoting never is.
const CANDID_KEYWORDS: ReadonlySet<string> = new Set([
  "type",
  "service",
  "import",
  "func",
  "record",
  "variant",
  "opt",
  "vec",
  "blob",
  "principal",
  "query",
  "oneway",
  "composite_query",
  "null",
  "bool",
  "nat",
  "int",
  "nat8",
  "nat16",
  "nat32",
  "nat64",
  "int8",
  "int16",
  "int32",
  "int64",
  "float32",
  "float64",
  "text",
  "reserved",
  "empty",
]);

const CANDID_IDENTIFIER = /^[A-Za-z_][A-Za-z0-9_]*$/;
const NUMERIC_LABEL = /^_(0|[1-9][0-9]*)_$/;
const LABEL_ID_MAX = 4_294_967_295; // Candid label ids are u32.

/** A method name must be a plain Candid identifier; fail closed otherwise. */
export function checkMethodName(name: string): void {
  if (!CANDID_IDENTIFIER.test(name) || CANDID_KEYWORDS.has(name)) {
    throw new CandidStartError(
      "did_invalid_method_name",
      `method name ${JSON.stringify(name)} is not a plain Candid identifier`,
    );
  }
}

function labelText(key: string): string {
  const numeric = NUMERIC_LABEL.exec(key);
  if (numeric !== null) {
    const id = Number(numeric[1]);
    if (id > LABEL_ID_MAX) {
      throw new CandidStartError(
        "did_label_out_of_range",
        `numeric label ${key} exceeds the u32 label-id space`,
      );
    }
    return String(id);
  }
  if (CANDID_IDENTIFIER.test(key) && !CANDID_KEYWORDS.has(key)) {
    return key;
  }
  // Quoted form, matching what candid_parser accepts for hostile labels (see
  // the quoting golden): backslash and quote escaped, control characters
  // refused rather than mis-emitted.
  let out = '"';
  for (const char of key) {
    const code = char.codePointAt(0) ?? 0;
    if (code < 0x20 || code === 0x7f) {
      throw new CandidStartError(
        "did_unencodable_label",
        `label ${JSON.stringify(key)} contains a control character`,
      );
    }
    if (char === "\\" || char === '"') {
      out += "\\";
    }
    out += char;
  }
  return out + '"';
}

class DidEmitter {
  /** Recursive `rec` nodes, keyed by object identity, in detection order. */
  private readonly named = new Map<object, string>();

  /** Pass 1: find every `rec` node that actually recurses and name it. */
  private discover(root: AnySchema): void {
    const finished = new Set<object>();
    const onStack = new Set<object>();

    const visit = (schema: AnySchema): void => {
      const shape = shapeOf(schema);
      if (shape.kind === "rec") {
        const node = schema as object;
        if (onStack.has(node)) {
          if (!this.named.has(node)) {
            this.named.set(node, `rec${this.named.size}`);
          }
          return;
        }
        if (finished.has(node)) {
          return;
        }
        onStack.add(node);
        if (typeof shape.body !== "function") {
          throw new CandidStartError(
            "did_unsupported_schema",
            "rec schema without a body thunk",
          );
        }
        visit(shape.body());
        onStack.delete(node);
        finished.add(node);
        return;
      }
      switch (shape.kind) {
        case "primitive":
        case "blob":
        case "unit":
          return;
        case "opt":
        case "vec":
          if (shape.inner === undefined) {
            throw new CandidStartError(
              "did_unsupported_schema",
              `${shape.kind} schema without an inner schema`,
            );
          }
          visit(shape.inner);
          return;
        case "record":
        case "variant": {
          const table = shape.kind === "record" ? shape.fields : shape.arms;
          if (table === undefined) {
            throw new CandidStartError(
              "did_unsupported_schema",
              `${shape.kind} schema without members`,
            );
          }
          for (const key of Object.keys(table)) {
            const child = ownEntry(table, key);
            if (child !== undefined) {
              visit(child);
            }
          }
          return;
        }
        case "tuple":
          if (shape.elements === undefined) {
            throw new CandidStartError(
              "did_unsupported_schema",
              "tuple schema without elements",
            );
          }
          for (const element of shape.elements) {
            visit(element);
          }
          return;
        default:
          throw new CandidStartError(
            "did_unsupported_schema",
            `unknown schema kind \`${shape.kind}\` — failing closed`,
          );
      }
    };

    visit(root);
  }

  /** Pass 2: render a type, replacing named `rec` nodes with their name. */
  private render(schema: AnySchema, insideOf: object | null): string {
    const shape = shapeOf(schema);
    if (shape.kind === "rec") {
      const node = schema as object;
      const name = this.named.get(node);
      if (name !== undefined && node !== insideOf) {
        return name;
      }
      if (name !== undefined && node === insideOf) {
        // Rendering the named definition body itself: recurse into the body,
        // where further self-references hit the branch above.
        if (typeof shape.body !== "function") {
          throw new CandidStartError(
            "did_unsupported_schema",
            "rec schema without a body thunk",
          );
        }
        return this.render(shape.body(), null);
      }
      if (typeof shape.body !== "function") {
        throw new CandidStartError(
          "did_unsupported_schema",
          "rec schema without a body thunk",
        );
      }
      return this.render(shape.body(), insideOf);
    }
    switch (shape.kind) {
      case "primitive": {
        const primitive = shape.primitive ?? "";
        if (!CANDID_KEYWORDS.has(primitive) && primitive !== "principal") {
          throw new CandidStartError(
            "did_unsupported_schema",
            `unknown primitive \`${primitive}\``,
          );
        }
        return primitive;
      }
      case "opt": {
        if (shape.inner === undefined) {
          throw new CandidStartError(
            "did_unsupported_schema",
            "opt schema without an inner schema",
          );
        }
        const inner = resolveShape(shape.inner);
        if (
          inner === null ||
          inner.kind === "opt" ||
          (inner.kind === "primitive" &&
            (inner.primitive === "null" || inner.primitive === "reserved"))
        ) {
          throw new CandidStartError(
            "did_unrepresentable_option",
            "opt of opt/null/reserved has no domain representation",
          );
        }
        return `opt ${this.render(shape.inner, insideOf)}`;
      }
      case "vec":
        if (shape.inner === undefined) {
          throw new CandidStartError(
            "did_unsupported_schema",
            "vec schema without an inner schema",
          );
        }
        return `vec ${this.render(shape.inner, insideOf)}`;
      case "blob":
        return "blob";
      case "unit":
        return "record {}";
      case "record": {
        const fields = shape.fields;
        if (fields === undefined) {
          throw new CandidStartError(
            "did_unsupported_schema",
            "record schema without fields",
          );
        }
        const keys = Object.keys(fields);
        if (keys.length === 0) {
          return "record {}";
        }
        const rendered = keys.map((key) => {
          const child = ownEntry(fields, key);
          if (child === undefined) {
            throw new CandidStartError(
              "did_unsupported_schema",
              "record field without a schema",
            );
          }
          return `${labelText(key)} : ${this.render(child, insideOf)}`;
        });
        return `record { ${rendered.join("; ")} }`;
      }
      case "tuple": {
        const elements = shape.elements;
        if (elements === undefined) {
          throw new CandidStartError(
            "did_unsupported_schema",
            "tuple schema without elements",
          );
        }
        if (elements.length === 0) {
          return "record {}";
        }
        const rendered = elements.map((element) =>
          this.render(element, insideOf),
        );
        return `record { ${rendered.join("; ")} }`;
      }
      case "variant": {
        const arms = shape.arms;
        if (arms === undefined) {
          throw new CandidStartError(
            "did_unsupported_schema",
            "variant schema without arms",
          );
        }
        const keys = Object.keys(arms);
        if (keys.length === 0) {
          throw new CandidStartError(
            "did_empty_variant",
            "a variant with no arms has no Candid spelling",
          );
        }
        const rendered = keys.map((key) => {
          const child = ownEntry(arms, key);
          if (child === undefined) {
            throw new CandidStartError(
              "did_unsupported_schema",
              "variant arm without a schema",
            );
          }
          const armShape = resolveShape(child);
          if (
            armShape !== null &&
            armShape.kind === "primitive" &&
            armShape.primitive === "null"
          ) {
            return labelText(key);
          }
          return `${labelText(key)} : ${this.render(child, insideOf)}`;
        });
        return `variant { ${rendered.join("; ")} }`;
      }
      default:
        throw new CandidStartError(
          "did_unsupported_schema",
          `unknown schema kind \`${shape.kind}\` — failing closed`,
        );
    }
  }

  emit(methods: readonly DidMethodSpec[]): string {
    const sorted = [...methods].sort((a, b) =>
      a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
    );
    for (const method of sorted) {
      checkMethodName(method.name);
      if (method.input !== null) {
        this.discover(method.input);
      }
      this.discover(method.output);
    }

    const lines: string[] = [];
    for (const [node, name] of this.named) {
      lines.push(
        `type ${name} = ${this.render(node as AnySchema, node)};`,
      );
    }
    if (lines.length > 0) {
      lines.push("");
    }
    lines.push("service : {");
    for (const method of sorted) {
      const arg =
        method.input === null ? "" : this.render(method.input, null);
      const ret = this.render(method.output, null);
      const annotation = method.mode === "query" ? " query" : "";
      lines.push(`  ${method.name} : (${arg}) -> (${ret})${annotation};`);
    }
    lines.push("}");
    return lines.join("\n") + "\n";
  }
}

/** Emit a deterministic Candid service document for the given methods. */
export function emitServiceDid(methods: readonly DidMethodSpec[]): string {
  const names = new Set<string>();
  for (const method of methods) {
    if (names.has(method.name)) {
      throw new CandidStartError(
        "did_duplicate_method",
        `method ${JSON.stringify(method.name)} is registered twice`,
      );
    }
    names.add(method.name);
  }
  return new DidEmitter().emit(methods);
}
