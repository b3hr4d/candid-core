import { test } from "node:test";
import assert from "node:assert/strict";
import { c } from "../src/schema.ts";
import {
  createApp,
  HYDRATION_SCRIPT_ID,
  type HttpRequest,
} from "../src/canister.ts";
import { h } from "../src/html.ts";
import { defineRoute } from "../src/router.ts";
import { query, update } from "../src/server-fn.ts";
import { principalFromText } from "../src/principal.ts";
import { createNotesApp } from "../example/notes/app.ts";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function get(url: string): HttpRequest {
  return { method: "GET", url, headers: [], body: new Uint8Array() };
}

function post(url: string, payload: unknown): HttpRequest {
  return {
    method: "POST",
    url,
    headers: [["content-type", "application/json"]],
    body: encoder.encode(JSON.stringify(payload)),
  };
}

function bodyText(body: Uint8Array): string {
  return decoder.decode(body);
}

test("SSR: a route renders through its loader with a hydration payload", async () => {
  const app = createNotesApp();
  const response = await app.httpRequest(get("/"));
  assert.equal(response.status, 200);
  const html = bodyText(response.body);
  assert.match(html, /^<!doctype html>/);
  assert.match(html, /Canister-side rendering/);
  assert.match(html, /Schemas are generated/);
  const scriptMatch = new RegExp(
    `<script type="application/json" id="${HYDRATION_SCRIPT_ID}">([^<]*)</script>`,
  ).exec(html);
  assert.notEqual(scriptMatch, null);
  const payload = JSON.parse(scriptMatch?.[1] ?? "") as {
    route: string;
    params: Record<string, string>;
    data: unknown[];
  };
  assert.equal(payload.route, "home");
  assert.equal(payload.data.length, 2);
});

test("SSR: params flow into the loader; unknown ids render not-found data", async () => {
  const app = createNotesApp();
  const found = await app.httpRequest(get("/notes/2"));
  assert.match(bodyText(found.body), /Schemas are generated/);
  const missing = await app.httpRequest(get("/notes/999"));
  assert.equal(missing.status, 200);
  assert.match(bodyText(missing.body), /Note not found/);
  const invalid = await app.httpRequest(get("/notes/not-a-number"));
  assert.equal(invalid.status, 200);
  assert.match(bodyText(invalid.body), /Note not found/);
  const noRoute = await app.httpRequest(get("/nope"));
  assert.equal(noRoute.status, 404);
});

test("SSR output escapes attacker-controlled note content end to end", async () => {
  const app = createNotesApp();
  await app.candid.invoke("add_note", {
    title: '<img src=x onerror=alert(1)>',
    body: "</script><script>steal()</script>",
    tags: ['"><svg onload=evil()>'],
    attachment: null,
  });
  const response = await app.httpRequest(get("/"));
  const html = bodyText(response.body);
  assert.equal(html.includes("<img src=x"), false);
  assert.equal(html.includes("<svg"), false);
  assert.match(html, /&lt;img src=x onerror=alert\(1\)&gt;/);
  // The hydration payload carries the same strings, JSON-escaped so the
  // document cannot be broken out of.
  assert.equal(html.includes("</script><script>steal()"), false);
});

test("RPC: query functions answer on the query path", async () => {
  const app = createNotesApp();
  const response = await app.httpRequest(post("/__rpc/get_note", { arg: "1" }));
  assert.equal(response.status, 200);
  const body = JSON.parse(bodyText(response.body)) as {
    ok: boolean;
    value: { title: string };
  };
  assert.equal(body.ok, true);
  assert.equal(body.value.title, "Canister-side rendering");
});

test("RPC: update functions upgrade off the query path, then execute", async () => {
  const app = createNotesApp();
  const draft = {
    arg: { title: "T", body: "B", tags: [], attachment: null },
  };
  const queryPath = await app.httpRequest(post("/__rpc/add_note", draft));
  assert.equal(queryPath.upgrade, true);
  const updatePath = await app.httpRequestUpdate(post("/__rpc/add_note", draft));
  assert.equal(updatePath.status, 200);
  const body = JSON.parse(bodyText(updatePath.body)) as {
    ok: boolean;
    value: { id: string };
  };
  assert.equal(body.ok, true);
  assert.equal(body.value.id, "3");
});

test("RPC failure surface: unknown, malformed, invalid, oversized", async () => {
  const app = createNotesApp();
  const unknown = await app.httpRequest(post("/__rpc/nope", {}));
  assert.equal(unknown.status, 404);
  const getOnRpc = await app.httpRequest(get("/__rpc/get_note"));
  assert.equal(getOnRpc.status, 405);
  const malformed = await app.httpRequest({
    ...post("/__rpc/get_note", {}),
    body: encoder.encode("{not json"),
  });
  assert.equal(malformed.status, 400);
  assert.equal(
    (JSON.parse(bodyText(malformed.body)) as { code: string }).code,
    "rpc_malformed_json",
  );
  const invalid = await app.httpRequest(post("/__rpc/get_note", { arg: -5 }));
  assert.equal(invalid.status, 400);
  const parsed = JSON.parse(bodyText(invalid.body)) as {
    code: string;
    issues: { code: string }[];
  };
  assert.equal(parsed.code, "invalid_argument");
  assert.equal(parsed.issues.length > 0, true);
});

test("request body bound fails closed before parsing", async () => {
  const app = createApp({
    routes: [],
    serverFns: [
      query({ name: "ping", output: c.bool, handler: () => true }),
    ],
    shell: ({ body }) => h("html", null, h("body", null, body)),
    maxRequestBytes: 16,
  });
  const oversized = await app.httpRequest({
    method: "POST",
    url: "/__rpc/ping",
    headers: [],
    body: new Uint8Array(17),
  });
  assert.equal(oversized.status, 413);
});

test("a handler that breaks its own output contract is a 500, not a lie", async () => {
  const errors: unknown[] = [];
  const app = createApp({
    routes: [],
    serverFns: [
      query({
        name: "broken",
        output: c.nat,
        handler: () => "not a nat" as unknown as bigint,
      }),
    ],
    shell: ({ body }) => h("html", null, h("body", null, body)),
    onError: (error) => errors.push(error),
  });
  const response = await app.httpRequest(post("/__rpc/broken", {}));
  assert.equal(response.status, 500);
  const body = JSON.parse(bodyText(response.body)) as {
    code: string;
    issues?: unknown;
  };
  assert.equal(body.code, "invalid_result");
  // Server internals stay out of the body; the observer gets the details.
  assert.equal(body.issues, undefined);
  assert.equal(errors.length, 1);
});

test("candid surface: direct dispatch with caller override", async () => {
  const app = createNotesApp();
  const caller = principalFromText("aaaaa-aa");
  const created = (await app.candid.invoke(
    "add_note",
    { title: "Mine", body: "b", tags: [], attachment: null },
    { caller },
  )) as { owner: string };
  assert.equal(created.owner, "aaaaa-aa");
  await assert.rejects(app.candid.invoke("missing", undefined), /unknown_method|no candid method/);
});

test("zero-argument methods refuse stray arguments", async () => {
  const app = createNotesApp();
  const response = await app.httpRequest(post("/__rpc/list_notes", { arg: {} }));
  assert.equal(response.status, 400);
  const ok = await app.httpRequest(post("/__rpc/list_notes", {}));
  assert.equal(ok.status, 200);
});

test("name collisions fail app construction closed", () => {
  const shell = ({ body }: { body: ReturnType<typeof h> }) =>
    h("html", null, h("body", null, body));
  const route = defineRoute({
    name: "home",
    path: "/",
    data: c.bool,
    loader: () => true,
    component: () => h("div", null),
  });
  assert.throws(
    () =>
      createApp({
        routes: [route],
        serverFns: [
          query({ name: "home_loader", output: c.bool, handler: () => true }),
        ],
        shell,
      }),
    /app_duplicate_method|twice/,
  );
  assert.throws(
    () =>
      createApp({
        routes: [route, route],
        serverFns: [],
        shell,
      }),
    /app_duplicate_route|twice/,
  );
});

test("didText is deterministic and covers loaders as query methods", () => {
  const first = createNotesApp().didText();
  const second = createNotesApp().didText();
  assert.equal(first, second);
  assert.match(first, /home_loader : \(\) -> \(vec record \{.*\}\) query;/);
  assert.match(first, /note_loader : \(record \{ id : text \}\) -> \(opt record \{.*\}\) query;/);
  assert.match(first, /add_note : \(record \{.*\}\) -> \(record \{.*\}\);/);
});

test("update handlers cannot run in query context", async () => {
  // The HTTP query path already upgrades; this pins the deeper guard on the
  // server-fn itself.
  const fn = update({ name: "mutate", output: c.bool, handler: () => true });
  await assert.rejects(
    fn.invoke(undefined, {
      caller: principalFromText("2vxsx-fae"),
      mode: "query",
    }),
    /server_fn_requires_update|update/,
  );
});
