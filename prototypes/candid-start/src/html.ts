// Server-side HTML rendering: `h()` elements, function components, and an
// escape-safe `renderToString`.
//
// There is deliberately no raw-HTML injection API. Text children and
// attribute values are always escaped; the single carve-out is
// `jsonScript`, whose content is JSON serialized by this module with `<`,
// `>`, `&`, and the U+2028/U+2029 separators escaped *inside the JSON string
// domain* — parser-safe and XSS-safe without ever concatenating unescaped
// input. Any future `dangerouslySetInnerHTML`-style API is a deliberate
// decision to make in review, not a convenience to slip in.
//
// SSR carries no event handlers: every `on*` prop is skipped at render time,
// whether its value is a function or a string. A future client runtime
// attaches behavior after hydration instead (TanStack Start's model, and the
// reason this stays a render concern).
import { CandidStartError } from "./errors.ts";

export type Child =
  | VNode
  | string
  | number
  | bigint
  | boolean
  | null
  | undefined
  | readonly Child[];

export type Props = Record<string, unknown>;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Component<P = any> = (props: P) => VNode;

export type VNode =
  | {
      readonly kind: "element";
      readonly tag: string;
      readonly props: Props;
      readonly children: readonly Child[];
    }
  | {
      readonly kind: "component";
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      readonly component: Component<any>;
      readonly props: Props;
    }
  | { readonly kind: "fragment"; readonly children: readonly Child[] }
  | { readonly kind: "json-script"; readonly id: string; readonly json: string };

const TAG_NAME = /^[a-zA-Z][a-zA-Z0-9-]*$/;
const ATTRIBUTE_NAME = /^[a-zA-Z_:][a-zA-Z0-9_:.-]*$/;

const VOID_ELEMENTS: ReadonlySet<string> = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
]);

// Attributes whose value is a URL and must not smuggle a script scheme.
const URL_ATTRIBUTES: ReadonlySet<string> = new Set([
  "href",
  "src",
  "action",
  "formaction",
  "data", // <object data="…">
]);

// Event-handler attribute names (`onclick`, `onload`, …). SSR never emits
// these: behavior is attached after hydration, so a string-valued handler is
// as much a footgun as a function-valued one and is skipped either way.
const EVENT_HANDLER_NAME = /^on[a-z]/i;

export function h(
  tag: string,
  props?: Props | null,
  ...children: readonly Child[]
): VNode;
export function h<P>(
  component: Component<P>,
  props: P,
  ...children: readonly Child[]
): VNode;
export function h(
  tag: string | Component,
  props?: unknown,
  ...children: readonly Child[]
): VNode {
  if (typeof tag === "function") {
    // Children passed to a component are threaded through `props.children`
    // (the React/TanStack convention) so they are never silently dropped; a
    // component reads them by declaring `children` in its props type.
    const base = (props ?? {}) as Props;
    const merged =
      children.length > 0 ? { ...base, children } : base;
    return { kind: "component", component: tag, props: merged };
  }
  if (!TAG_NAME.test(tag)) {
    throw new CandidStartError(
      "render_invalid_tag",
      `tag name ${JSON.stringify(tag)} is not a valid element name`,
    );
  }
  return {
    kind: "element",
    tag: tag.toLowerCase(),
    props: (props ?? {}) as Props,
    children,
  };
}

export function fragment(...children: readonly Child[]): VNode {
  return { kind: "fragment", children };
}

/**
 * A `<script type="application/json">` node whose body is serialized by the
 * framework. The payload must already be wire-shaped (JSON-safe): `bigint`
 * and byte values are the wire codec's job, not this one's.
 */
export function jsonScript(id: string, payload: unknown): VNode {
  if (!ATTRIBUTE_NAME.test(id)) {
    throw new CandidStartError(
      "render_invalid_attribute",
      `script id ${JSON.stringify(id)} is not attribute-safe`,
    );
  }
  const json = JSON.stringify(payload);
  if (json === undefined) {
    throw new CandidStartError(
      "render_unserializable_payload",
      "payload has no JSON serialization",
    );
  }
  // Escapes inside the JSON string domain: the document stays parseable HTML
  // (`</script>` cannot appear) and JSON.parse reads back the same value.
  const safe = json
    .replace(/&/g, "\\u0026")
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
  return { kind: "json-script", id, json: safe };
}

export function escapeText(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function escapeAttribute(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function checkUrl(name: string, value: string): void {
  // Scheme smuggling tolerates leading control characters and whitespace in
  // HTML parsers; strip everything at or below 0x20 before looking.
  let stripped = "";
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0;
    if (code > 0x20) {
      stripped += char;
    }
  }
  const lowered = stripped.toLowerCase();
  if (lowered.startsWith("javascript:") || lowered.startsWith("vbscript:")) {
    throw new CandidStartError(
      "render_unsafe_url",
      `attribute ${JSON.stringify(name)} carries a script-scheme URL`,
    );
  }
  // `data:` is script-bearing too: `data:text/html` and `data:image/svg+xml`
  // execute script when loaded as an `<iframe>`/`<object>` source. Allow only
  // a raster-image allowlist and refuse the rest, rather than block every
  // data: URL and break legitimate inline images.
  if (
    lowered.startsWith("data:") &&
    !/^data:image\/(png|jpe?g|gif|webp|avif|bmp|x-icon|vnd\.microsoft\.icon)[;,]/.test(
      lowered,
    )
  ) {
    throw new CandidStartError(
      "render_unsafe_url",
      `attribute ${JSON.stringify(name)} carries a non-image data: URL`,
    );
  }
}

function renderAttributes(props: Props): string {
  let out = "";
  for (const name of Object.keys(props)) {
    const value = props[name];
    if (value === null || value === undefined || value === false) {
      continue;
    }
    if (typeof value === "function" || EVENT_HANDLER_NAME.test(name)) {
      // Server render carries no event handlers, string-valued ones included.
      continue;
    }
    if (!ATTRIBUTE_NAME.test(name)) {
      throw new CandidStartError(
        "render_invalid_attribute",
        `attribute name ${JSON.stringify(name)} is not renderable`,
      );
    }
    if (value === true) {
      out += ` ${name}`;
      continue;
    }
    if (
      typeof value !== "string" &&
      typeof value !== "number" &&
      typeof value !== "bigint"
    ) {
      throw new CandidStartError(
        "render_invalid_attribute",
        `attribute ${JSON.stringify(name)} has an unrenderable ${typeof value} value`,
      );
    }
    const text = String(value);
    if (URL_ATTRIBUTES.has(name.toLowerCase())) {
      checkUrl(name, text);
    }
    out += ` ${name}="${escapeAttribute(text)}"`;
  }
  return out;
}

const MAX_RENDER_DEPTH = 256;

function renderChild(child: Child, depth: number): string {
  if (depth > MAX_RENDER_DEPTH) {
    // Array-nested children recurse here without touching renderNode, so the
    // bound must be enforced on this path too — otherwise a deeply nested
    // array bypasses it and overflows the stack with a raw RangeError.
    throw new CandidStartError(
      "render_depth_exceeded",
      `render nesting exceeded ${MAX_RENDER_DEPTH} levels`,
    );
  }
  if (child === null || child === undefined || typeof child === "boolean") {
    return "";
  }
  if (typeof child === "string") {
    return escapeText(child);
  }
  if (typeof child === "number" || typeof child === "bigint") {
    return escapeText(String(child));
  }
  if (Array.isArray(child)) {
    let out = "";
    for (const nested of child) {
      out += renderChild(nested, depth + 1);
    }
    return out;
  }
  return renderNode(child as VNode, depth + 1);
}

function renderNode(node: VNode, depth: number): string {
  if (depth > MAX_RENDER_DEPTH) {
    throw new CandidStartError(
      "render_depth_exceeded",
      `render nesting exceeded ${MAX_RENDER_DEPTH} levels`,
    );
  }
  switch (node.kind) {
    case "component": {
      const rendered = node.component(node.props);
      return renderNode(rendered, depth + 1);
    }
    case "fragment": {
      let out = "";
      for (const child of node.children) {
        out += renderChild(child, depth);
      }
      return out;
    }
    case "json-script":
      return `<script type="application/json" id="${node.id}">${node.json}</script>`;
    case "element": {
      const open = `<${node.tag}${renderAttributes(node.props)}>`;
      if (VOID_ELEMENTS.has(node.tag)) {
        if (node.children.length > 0) {
          throw new CandidStartError(
            "render_void_with_children",
            `<${node.tag}> is a void element and cannot have children`,
          );
        }
        return open;
      }
      let out = open;
      for (const child of node.children) {
        out += renderChild(child, depth);
      }
      return `${out}</${node.tag}>`;
    }
    default: {
      throw new CandidStartError("render_invalid_node", "unknown VNode kind");
    }
  }
}

export function renderToString(node: VNode): string {
  return renderNode(node, 0);
}

/** Render a complete HTML document from a root `<html>` node. */
export function renderDocument(root: VNode): string {
  return `<!doctype html>\n${renderNode(root, 0)}\n`;
}
