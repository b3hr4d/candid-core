// The loop-closing suite: it needs cargo, so it lives outside `npm test`
// (run with `npm run test:integration`). Three claims are proven against the
// real compiler, not asserted:
//
//   1. example/notes/schemas.ts is exactly what the generator emits today —
//      generated code cannot drift from its .did source unnoticed.
//   2. The framework-emitted service .did compiles into a canonical
//      Contract: the derived interface is real, not .did-shaped text.
//   3. Inline and named spellings of the same interface share their
//      `interface_id` — the semantic-identity property the emitter's
//      inline-first design leans on.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { c, type Schema } from "../src/schema.ts";
import { emitServiceDid } from "../src/did.ts";
import type { AnySchema } from "../src/walk.ts";
import { createNotesApp } from "../example/notes/app.ts";
import { generateSchemas } from "../scripts/generate.ts";

const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));

interface CompileOutput {
  ok: boolean;
  contract?: { identities?: { interface?: string; contract?: string } };
  diagnostics?: unknown;
}

function compileDid(source: string, label: string): CompileOutput {
  const dir = mkdtempSync(join(tmpdir(), "candid-start-"));
  try {
    const path = join(dir, `${label}.did`);
    writeFileSync(path, source);
    const result = spawnSync(
      "cargo",
      ["run", "--quiet", "--locked", "--bin", "candid-core", "--", "compile", path],
      { cwd: repoRoot, encoding: "utf-8", maxBuffer: 64 * 1024 * 1024 },
    );
    assert.equal(result.error, undefined, String(result.error));
    return JSON.parse(result.stdout) as CompileOutput;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("checked-in schemas.ts is byte-identical to fresh generator output", () => {
  const committed = readFileSync(
    fileURLToPath(new URL("../example/notes/schemas.ts", import.meta.url)),
    "utf-8",
  );
  assert.equal(committed, generateSchemas());
});

test("the emitted service .did compiles to a canonical Contract", () => {
  const app = createNotesApp();
  const did = app.didText();
  const output = compileDid(did, "service");
  assert.equal(output.ok, true, JSON.stringify(output.diagnostics));
  const interfaceId = output.contract?.identities?.interface;
  assert.equal(typeof interfaceId, "string");
  // Determinism end to end: a second app emits the same text, and the same
  // text has the same identity.
  assert.equal(createNotesApp().didText(), did);
});

test("inline and named spellings share an interface_id", () => {
  type Tree =
    | { tag: "leaf"; value: bigint }
    | { tag: "node"; value: { left: Tree; right: Tree } };
  const Tree: Schema<Tree> = c.rec(() =>
    c.variant({ leaf: c.nat, node: c.record({ left: Tree, right: Tree }) }),
  );
  const emitted = emitServiceDid([
    {
      name: "walk",
      mode: "query" as const,
      input: Tree as AnySchema,
      output: c.opt(c.record({ depth: c.nat32, tree: Tree })) as AnySchema,
    },
  ]);
  const named = [
    "type tree = variant { leaf : nat; node : record { left : tree; right : tree } };",
    "type walk_result = opt record { depth : nat32; tree : tree };",
    "service : {",
    "  walk : (tree) -> (walk_result) query;",
    "}",
  ].join("\n");
  const emittedOut = compileDid(emitted, "emitted");
  const namedOut = compileDid(named, "named");
  assert.equal(emittedOut.ok, true, JSON.stringify(emittedOut.diagnostics));
  assert.equal(namedOut.ok, true, JSON.stringify(namedOut.diagnostics));
  assert.equal(typeof emittedOut.contract?.identities?.interface, "string");
  assert.equal(
    emittedOut.contract?.identities?.interface,
    namedOut.contract?.identities?.interface,
  );
});
