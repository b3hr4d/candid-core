// The issue #153 browser smoke: `didToContract` runs inside a real headless
// page — wasm fetched over HTTP, compiled and executed by the browser — and
// its envelope feeds `@candid-core/schema`'s `schemaFromContract`, field
// names included. This is the half of the browser story the Node tests
// cannot claim: no `node:` builtin exists in the page, so everything the
// library needs in a browser is proven present.
//
// The browser is supplied, never downloaded here: `CHROMIUM_PATH` names the
// executable (CI passes the pinned setup-chrome build; this repository's
// development environment ships one at /opt/pw-browsers/chromium). The
// schema runtime is served from crates/candid-core-ts/ts/dist — build it
// first (`npm run build` there) or this test refuses with that instruction.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

import { chromium } from "playwright-core";

const HERE = new URL(".", import.meta.url).pathname;
const WASM_DIR = path.join(HERE, "..", "..", "wasm");
const DIST_DIR = path.join(HERE, "..", "..", "..", "..", "candid-core-ts", "ts", "dist");
const CHROMIUM = process.env.CHROMIUM_PATH ?? "/opt/pw-browsers/chromium";

const SOURCE = [
  "type Payload = record { owner: principal; amount: nat };",
  "service : { transfer: (Payload) -> () };",
].join("\n");

const PAGE = `<!doctype html>
<script type="module">
  import init, { didToContract } from "/wasm/candid_core_wasm.js";
  import { schemaFromContract } from "/schema/contract.js";
  import { validate } from "/schema/validate.js";
  try {
    await init();
    const envelope = JSON.parse(didToContract(JSON.stringify({ source: ${JSON.stringify(
      SOURCE,
    )} })));
    const built = schemaFromContract(envelope);
    if (!built.ok) {
      window.__result = { failed: built.issues };
    } else {
      window.__result = {
        ok: true,
        schemas: Object.keys(built.schemas),
        actor: built.actor !== undefined,
        // Envelope-carried names must have rendered: a keyed value passes
        // only if "owner"/"amount" are the schema's field keys.
        named: validate(built.schemas.Payload, {
          owner: { toText: () => "aaaaa-aa" },
          amount: 5n,
        }),
      };
    }
  } catch (error) {
    window.__result = { threw: String(error) };
  }
</script>`;

const TYPES = { ".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html" };

function serve() {
  const server = createServer((request, response) => {
    let file;
    if (request.url === "/" || request.url === "/index.html") {
      response.writeHead(200, { "Content-Type": "text/html" });
      response.end(PAGE);
      return;
    } else if (request.url.startsWith("/wasm/")) {
      file = path.join(WASM_DIR, request.url.slice("/wasm/".length));
    } else if (request.url.startsWith("/schema/")) {
      file = path.join(DIST_DIR, request.url.slice("/schema/".length));
    }
    if (file === undefined || !existsSync(file)) {
      response.writeHead(404);
      response.end("not found");
      return;
    }
    response.writeHead(200, {
      "Content-Type": TYPES[path.extname(file)] ?? "application/octet-stream",
    });
    response.end(readFileSync(file));
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

test("didToContract feeds schemaFromContract in a headless page", async () => {
  assert.ok(
    existsSync(path.join(DIST_DIR, "contract.js")),
    "crates/candid-core-ts/ts/dist is missing; run `npm run build` there first",
  );
  assert.ok(
    existsSync(CHROMIUM),
    `no browser at ${CHROMIUM}; set CHROMIUM_PATH to a Chromium executable`,
  );
  const server = await serve();
  const { port } = server.address();
  const browser = await chromium.launch({ executablePath: CHROMIUM });
  try {
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${port}/`);
    const result = await page.waitForFunction(() => window.__result).then((h) => h.jsonValue());
    assert.deepStrictEqual(result, {
      ok: true,
      schemas: ["Payload"],
      actor: true,
      named: { ok: true },
    });
  } finally {
    await browser.close();
    server.close();
  }
});
