import { test } from "node:test";
import assert from "node:assert/strict";
import {
  escapeAttribute,
  escapeText,
  fragment,
  h,
  jsonScript,
  renderDocument,
  renderToString,
} from "../src/html.ts";

test("text children are always escaped", () => {
  const html = renderToString(h("p", null, '<script>alert("x")</script> & more'));
  assert.equal(
    html,
    "<p>&lt;script&gt;alert(\"x\")&lt;/script&gt; &amp; more</p>",
  );
});

test("attribute values cannot break out of their quotes", () => {
  const html = renderToString(h("div", { title: '" onmouseover="evil()' }));
  assert.equal(html, '<div title="&quot; onmouseover=&quot;evil()"></div>');
});

test("script-scheme URLs fail closed, smuggling included", () => {
  assert.throws(() => renderToString(h("a", { href: "javascript:alert(1)" }, "x")));
  assert.throws(
    () => renderToString(h("a", { href: " \u0000java\tscript:alert(1)" }, "x")),
    /render_unsafe_url|script-scheme/,
  );
  assert.equal(
    renderToString(h("a", { href: "/notes/1" }, "ok")),
    '<a href="/notes/1">ok</a>',
  );
});

test("hostile tag and attribute names are rejected", () => {
  assert.throws(() => h("scr ipt", null));
  assert.throws(() => renderToString(h("div", { "x><script": "1" })));
  // An `on*`-shaped hostile name is skipped as an event handler before the
  // name check, so it renders nothing rather than throwing — still safe.
  assert.equal(renderToString(h("div", { "onx><script": "1" })), "<div></div>");
});

test("event-handler props are skipped whether function or string", () => {
  assert.equal(
    renderToString(h("button", { onclick: "alert(1)", onmouseover: () => 0 }, "ok")),
    "<button>ok</button>",
  );
  // A non-handler attribute of the same value still renders.
  assert.equal(
    renderToString(h("button", { "data-x": "v" }, "ok")),
    '<button data-x="v">ok</button>',
  );
});

test("data: URLs are refused except a raster-image allowlist", () => {
  assert.throws(
    () => renderToString(h("iframe", { src: "data:text/html,<script>alert(1)</script>" })),
    /render_unsafe_url|data:/,
  );
  assert.throws(
    () => renderToString(h("img", { src: "data:image/svg+xml,<svg onload=alert(1)>" })),
    /render_unsafe_url|data:/,
  );
  assert.throws(
    () => renderToString(h("object", { data: "data:text/html,<script>1</script>" })),
    /render_unsafe_url|data:/,
  );
  assert.equal(
    renderToString(h("img", { src: "data:image/png;base64,iVBORw0KGgo=" })),
    '<img src="data:image/png;base64,iVBORw0KGgo=">',
  );
});

test("void elements refuse children", () => {
  assert.equal(renderToString(h("br", null)), "<br>");
  assert.throws(() => renderToString(h("img", { src: "/x.png" }, "child")));
});

test("function props are skipped: SSR carries no handlers", () => {
  const html = renderToString(h("button", { onclick: () => "evil" }, "ok"));
  assert.equal(html, "<button>ok</button>");
});

test("components, fragments, numbers, bigints, and skipped children", () => {
  const Item = (props: { readonly label: string }) =>
    h("li", null, props.label);
  const html = renderToString(
    fragment(
      h(Item, { label: "a" }),
      null,
      undefined,
      false,
      0,
      1n,
      h("ul", null, [h(Item, { label: "b" })]),
    ),
  );
  assert.equal(html, "<li>a</li>01<ul><li>b</li></ul>");
});

test("jsonScript embeds a parser-safe, parseable payload", () => {
  const payload = { text: "</script><img src=x onerror=alert(1)>", n: 1 };
  const html = renderToString(jsonScript("data", payload));
  assert.equal(html.includes("</script><img"), false);
  assert.match(html, /^<script type="application\/json" id="data">/);
  const inner = html.slice(html.indexOf(">") + 1, html.lastIndexOf("</script>"));
  assert.deepEqual(JSON.parse(inner), payload);
});

test("jsonScript escapes the line separators JSON allows raw", () => {
  const html = renderToString(jsonScript("data", { s: "\u2028\u2029" }));
  assert.equal(html.includes("\u2028"), false);
  const inner = html.slice(html.indexOf(">") + 1, html.lastIndexOf("</script>"));
  assert.deepEqual(JSON.parse(inner), { s: "\u2028\u2029" });
});

test("renderDocument prefixes the doctype", () => {
  const html = renderDocument(h("html", null, h("body", null, "hi")));
  assert.equal(html, "<!doctype html>\n<html><body>hi</body></html>\n");
});

test("escape helpers cover the metacharacters", () => {
  assert.equal(escapeText("<&>"), "&lt;&amp;&gt;");
  assert.equal(escapeAttribute("\"'<&>"), "&quot;&#39;&lt;&amp;&gt;");
});

test("render depth is bounded", () => {
  let node = h("div", null);
  for (let index = 0; index < 300; index += 1) {
    node = h("div", null, node);
  }
  assert.throws(() => renderToString(node), /render_depth_exceeded|nesting/);
});

test("render depth is bounded on array-nested children too", () => {
  let node = h("span", null, "x");
  let children: import("../src/html.ts").Child = node;
  for (let index = 0; index < 300; index += 1) {
    children = [children];
  }
  assert.throws(
    () => renderToString(h("div", null, children)),
    /render_depth_exceeded|nesting/,
  );
});
