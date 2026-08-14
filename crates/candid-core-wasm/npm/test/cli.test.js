// The issue #153 parity gate through the actual wasm artifact: everything
// here runs the real bin/cli.js under Node, which loads the compiled
// WebAssembly — the "same code compiled twice" half the host-side Rust
// parity tests cannot cover. Outputs are compared byte-for-byte against the
// repository's reviewed artifacts: the generator's golden `.ts` modules and
// the committed envelope fixture the native `candid-core compile --envelope`
// binary produced.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const HERE = new URL(".", import.meta.url).pathname;
const CLI = path.join(HERE, "..", "bin", "cli.js");
const REPO = path.join(HERE, "..", "..", "..", "..");
const FIXTURES = path.join(REPO, "crates", "candid-core-ts", "tests", "fixtures");
const GOLDENS = path.join(REPO, "crates", "candid-core-ts", "tests", "goldens");

function gen(args, options = {}) {
  return spawnSync(process.execPath, [CLI, ...args], { encoding: "utf8", ...options });
}

test("every golden fixture reproduces its reviewed module byte-for-byte", () => {
  // Each fixture compiles from a directory containing only itself, so the
  // bundle walk cannot smuggle unrelated sources into provenance.
  for (const name of [
    "primitives",
    "collections",
    "variants",
    "recursion",
    "quoting",
    "deferred",
    "proto",
    "ledger",
    "empties",
    "arms",
  ]) {
    const scratch = mkdtempSync(path.join(tmpdir(), `candid-cli-${name}-`));
    writeFileSync(
      path.join(scratch, `${name}.did`),
      readFileSync(path.join(FIXTURES, `${name}.did`), "utf8"),
    );
    const out = path.join(scratch, "out");
    const run = gen(["gen", path.join(scratch, `${name}.did`), "-o", out]);
    assert.strictEqual(run.status, 0, `${name}: ${run.stdout}${run.stderr}`);
    const produced = readFileSync(path.join(out, `${name}.ts`), "utf8");
    const golden = readFileSync(path.join(GOLDENS, `${name}.ts`), "utf8");
    assert.strictEqual(produced, golden, `${name}: module must equal the reviewed golden`);
  }
});

test("the envelope is byte-identical to the native CLI's committed output", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-envelope-"));
  writeFileSync(
    path.join(scratch, "basic.did"),
    readFileSync(path.join(REPO, "tests", "fixtures", "conformance", "basic.did"), "utf8"),
  );
  const run = gen(["gen", path.join(scratch, "basic.did"), "-o", scratch]);
  assert.strictEqual(run.status, 0, `${run.stdout}${run.stderr}`);
  const produced = readFileSync(path.join(scratch, "basic.envelope.json"), "utf8");
  const fixture = readFileSync(
    path.join(REPO, "tests", "fixtures", "envelope", "basic.envelope.json"),
    "utf8",
  );
  assert.strictEqual(produced, fixture, "wasm CLI and native CLI must emit identical bytes");
  // The printed identities are the content addresses from the envelope.
  assert.match(run.stdout, /contract: {2}candid-core:contract:v1:sha256:[0-9a-f]{64}/);
  assert.match(run.stdout, /interface: candid-core:interface:v1:sha256:[0-9a-f]{64}/);
});

test("multi-file bundles compile through the directory walk", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-bundle-"));
  mkdirSync(path.join(scratch, "shared"));
  writeFileSync(
    path.join(scratch, "entry.did"),
    'import "shared/types.did";\nservice : { get: () -> (Item) query };',
  );
  writeFileSync(path.join(scratch, "shared", "types.did"), "type Item = record { id: nat };");
  const run = gen(["gen", path.join(scratch, "entry.did"), "-o", scratch]);
  assert.strictEqual(run.status, 0, `${run.stdout}${run.stderr}`);
  const produced = readFileSync(path.join(scratch, "entry.ts"), "utf8");
  assert.match(produced, /export type Item/);
  const envelope = JSON.parse(readFileSync(path.join(scratch, "entry.envelope.json"), "utf8"));
  assert.ok(envelope.contract, "the envelope carries the contract");
  const names = envelope.extensions["org.candid-core.field-names/v1"];
  assert.ok(
    names.some((entry) => entry[2] === "id"),
    "field names travel in the envelope",
  );
});

test("compile failures print the diagnostics document and exit 1", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-fail-"));
  writeFileSync(path.join(scratch, "broken.did"), "service : {");
  const run = gen(["gen", path.join(scratch, "broken.did")]);
  assert.strictEqual(run.status, 1);
  const response = JSON.parse(run.stdout);
  assert.strictEqual(response.ok, false);
  assert.strictEqual(response.diagnostics[0].code, "did_parse_error");
});

test("usage errors exit 64 with the usage text on stderr", () => {
  for (const argv of [[], ["frobnicate"], ["gen"], ["gen", "a.did", "extra"], ["gen", "a.did", "-o"], ["gen", "--typo", "a.did"]]) {
    const run = gen(argv);
    assert.strictEqual(run.status, 64, JSON.stringify(argv));
    assert.strictEqual(run.stdout, "", JSON.stringify(argv));
    assert.match(run.stderr, /^usage: candid-core-cli gen/, JSON.stringify(argv));
  }
});

test("a missing entry file fails with an actionable error", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-missing-"));
  const run = gen(["gen", path.join(scratch, "absent.did")]);
  assert.strictEqual(run.status, 1);
  assert.match(run.stderr, /absent\.did/);
});

// Determinism is enforced inside the tool (every generation double-runs);
// this pins that two whole CLI invocations also agree byte-for-byte.
test("two runs produce byte-identical artifacts", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-determinism-"));
  writeFileSync(
    path.join(scratch, "ledger.did"),
    readFileSync(path.join(FIXTURES, "ledger.did"), "utf8"),
  );
  const first = path.join(scratch, "first");
  const second = path.join(scratch, "second");
  execFileSync(process.execPath, [CLI, "gen", path.join(scratch, "ledger.did"), "-o", first]);
  execFileSync(process.execPath, [CLI, "gen", path.join(scratch, "ledger.did"), "-o", second]);
  for (const artifact of ["ledger.ts", "ledger.envelope.json"]) {
    assert.deepStrictEqual(
      readFileSync(path.join(first, artifact)),
      readFileSync(path.join(second, artifact)),
      artifact,
    );
  }
});

// The bundle walk is bounded by the compiler's own limits *before* anything
// is read (review finding on this PR): an oversized tree fails with the
// structured resource diagnostic, never by exhausting the JS heap.
test("the bundle walk fails closed on the compiler's source bounds", () => {
  // One file over the per-file byte limit — never read, only statted.
  const oversized = mkdtempSync(path.join(tmpdir(), "candid-cli-oversized-"));
  writeFileSync(path.join(oversized, "entry.did"), "service : {};");
  writeFileSync(path.join(oversized, "huge.did"), " ".repeat(1_048_577));
  let run = gen(["gen", path.join(oversized, "entry.did")]);
  assert.strictEqual(run.status, 1);
  let response = JSON.parse(run.stdout);
  assert.strictEqual(response.ok, false);
  assert.strictEqual(response.diagnostics[0].code, "resource_limit_exceeded");
  assert.deepStrictEqual(response.diagnostics[0].resource_limit, {
    resource: "source_bytes",
    limit: 1_048_576,
    observed: 1_048_577,
  });

  // Nine one-MiB files: each under the per-file limit, the aggregate over
  // the 8 MiB bundle limit.
  const aggregate = mkdtempSync(path.join(tmpdir(), "candid-cli-aggregate-"));
  writeFileSync(path.join(aggregate, "entry.did"), "service : {};");
  for (let index = 0; index < 9; index += 1) {
    writeFileSync(path.join(aggregate, `pad${index}.did`), " ".repeat(1_048_576));
  }
  run = gen(["gen", path.join(aggregate, "entry.did")]);
  assert.strictEqual(run.status, 1);
  response = JSON.parse(run.stdout);
  assert.strictEqual(response.diagnostics[0].resource_limit.resource, "bundle_bytes");
  assert.strictEqual(response.diagnostics[0].resource_limit.limit, 8_388_608);

  // 257 files: one over the source-count limit…
  const crowded = mkdtempSync(path.join(tmpdir(), "candid-cli-crowded-"));
  writeFileSync(path.join(crowded, "entry.did"), "service : {};");
  for (let index = 0; index < 256; index += 1) {
    writeFileSync(path.join(crowded, `extra${index}.did`), "type T = nat;");
  }
  run = gen(["gen", path.join(crowded, "entry.did")]);
  assert.strictEqual(run.status, 1);
  response = JSON.parse(run.stdout);
  assert.deepStrictEqual(response.diagnostics[0].resource_limit, {
    resource: "sources",
    limit: 256,
    observed: 257,
  });

  // …and exactly at the limit the walk admits the bundle and the run
  // succeeds — the bound fires one over, not at.
  const outDir = path.join(crowded, "out");
  writeFileSync(path.join(crowded, "extra0.did"), ""); // still a .did, still counted
  const exact = gen(["gen", path.join(crowded, "entry.did"), "-o", outDir]);
  assert.strictEqual(exact.status, 1, "257 files stay refused");
  rmSync(path.join(crowded, "extra255.did"));
  const atLimit = gen(["gen", path.join(crowded, "entry.did"), "-o", outDir]);
  assert.strictEqual(atLimit.status, 0, `${atLimit.stdout}${atLimit.stderr}`);
});
