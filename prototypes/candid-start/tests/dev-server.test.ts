import { test } from "node:test";
import assert from "node:assert/strict";
import { startDevServer } from "../src/dev-server.ts";
import { createNotesApp } from "../example/notes/app.ts";

test("the dev server plays the gateway, upgrade dance included", async () => {
  const app = createNotesApp();
  const handle = await startDevServer(app, { port: 0 });
  try {
    const base = `http://127.0.0.1:${handle.port}`;

    const page = await fetch(`${base}/notes/1`);
    assert.equal(page.status, 200);
    assert.match(await page.text(), /Canister-side rendering/);

    const queryCall = await fetch(`${base}/__rpc/list_notes`, {
      method: "POST",
      body: "{}",
    });
    const listed = (await queryCall.json()) as { ok: boolean; value: unknown[] };
    assert.equal(listed.ok, true);
    assert.equal(listed.value.length, 2);

    // An update through the plain query endpoint: the dev server must
    // notice the upgrade flag and re-dispatch, exactly like the IC gateway.
    const updateCall = await fetch(`${base}/__rpc/add_note`, {
      method: "POST",
      body: JSON.stringify({
        arg: { title: "Via gateway", body: "b", tags: [], attachment: null },
      }),
    });
    const added = (await updateCall.json()) as {
      ok: boolean;
      value: { title: string };
    };
    assert.equal(added.ok, true);
    assert.equal(added.value.title, "Via gateway");

    const after = (await (
      await fetch(`${base}/__rpc/list_notes`, { method: "POST", body: "{}" })
    ).json()) as { value: unknown[] };
    assert.equal(after.value.length, 3);
  } finally {
    await handle.close();
  }
});
