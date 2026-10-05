// The issue #153 parity gate through the actual wasm artifact: everything
// here runs the real bin/cli.js under Node, which loads the compiled
// WebAssembly — the "same code compiled twice" half the host-side Rust
// parity tests cannot cover. Outputs are compared byte-for-byte against the
// repository's reviewed artifacts: the generator's golden `.ts` modules and
// the committed envelope fixture the native `candid-core compile --envelope`
// binary produced.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  realpathSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
  mkdirSync,
  rmSync,
  utimesSync,
} from "node:fs";
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
    "options",
    "shadowing",
    "fidelity",
    "docs",
    "omissions",
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

// Issue #189: a module that leaves declarations out is usable, so the run
// succeeds; each omission is a warning on stderr worded as the module's own
// header line, and nothing about it reaches stdout's report. The library
// returns the native generator's list verbatim.
test("omissions are warnings, the run succeeds, and the library lists them", async () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-omissions-"));
  writeFileSync(
    path.join(scratch, "omissions.did"),
    readFileSync(path.join(FIXTURES, "omissions.did"), "utf8"),
  );
  const out = path.join(scratch, "out");
  const run = gen(["gen", path.join(scratch, "omissions.did"), "-o", out]);
  assert.strictEqual(run.status, 0, `${run.stdout}${run.stderr}`);
  const module = readFileSync(path.join(out, "omissions.ts"), "utf8");
  const header = module
    .split("\n")
    .filter((line) => line.startsWith("// Omitted: "))
    .map((line) => `warning: omitted ${line.slice("// Omitted: ".length)}`);
  assert.ok(header.length > 0);
  assert.deepStrictEqual(run.stderr.trimEnd().split("\n"), header);
  assert.doesNotMatch(run.stdout, /omitted/);
  assert.match(run.stdout, /^wrote .*omissions\.ts$/m);

  const { didToModule } = await import("../lib/index.js");
  const result = await didToModule(readFileSync(path.join(FIXTURES, "omissions.did"), "utf8"));
  assert.strictEqual(result.ok, true);
  assert.deepStrictEqual(
    result.omitted,
    JSON.parse(readFileSync(path.join(GOLDENS, "omissions.omitted.json"), "utf8")),
  );
  // A module that omits nothing says so with an empty list, not a missing key.
  const clean = await didToModule("service : { ping : () -> () }");
  assert.deepStrictEqual(clean.omitted, []);
});

// A quoted Candid method name can hold any character — among them the four
// that end a line in ECMAScript (LF, CR, U+2028, U+2029), other controls, and
// the two `JSON.stringify` leaves raw or spells differently. Each warning is
// still one stderr line, character for character the module's header line
// for that omission (PR #210 review).
test("an omitted method's name is quoted exactly as the module header quotes it", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-quoted-"));
  // Candid text escapes: the method name is  a"b\c <LF> <CR> <TAB> <BS> <FF>
  // <US> <LS> <PS> é 😀  once the compiler reads it.
  const method = String.raw`"a\"b\\c\n\r\t\u{8}\u{c}\u{1f}\u{2028}\u{2029}\u{e9}\u{1f600}"`;
  writeFileSync(
    path.join(scratch, "quoted.did"),
    `type Bad = record { _0_ : nat };\nservice : { ${method} : (Bad) -> (); ok : () -> () }\n`,
  );
  const out = path.join(scratch, "out");
  const run = gen(["gen", path.join(scratch, "quoted.did"), "-o", out]);
  assert.strictEqual(run.status, 0, `${run.stdout}${run.stderr}`);
  const header = readFileSync(path.join(out, "quoted.ts"), "utf8")
    .split("\n")
    .filter((line) => line.startsWith("// Omitted: "))
    .map((line) => `warning: omitted ${line.slice("// Omitted: ".length)}`);
  assert.deepStrictEqual(header, [
    "warning: omitted type Bad (reserved_field_name)",
    String.raw`warning: omitted method "a\"b\\c\n\r\t\u0008\u000c\u001f\u2028\u2029é😀" (references_omitted via Bad)`,
  ]);
  assert.deepStrictEqual(run.stderr.split("\n"), [...header, ""]);
  assert.doesNotMatch(run.stderr, /[\r\u2028\u2029]/);

  // Under --json the name is data, raw as didToModule returns it (not the
  // header's quoting), and stderr stays empty.
  const json = gen(["gen", path.join(scratch, "quoted.did"), "-o", out, "--json"]);
  assert.strictEqual(json.status, 0, `${json.stdout}${json.stderr}`);
  assert.strictEqual(json.stderr, "");
  const [entry] = JSON.parse(json.stdout).entries;
  assert.deepStrictEqual(
    entry.omitted.map((item) => item.name),
    ["Bad", 'a"b\\c\n\r\t\b\f\u001f\u2028\u2029\u00e9\u{1f600}'],
  );
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
  assert.match(produced, /export \{ \$Item as Item \};/);
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

// Issue #234: the embedded compiler refuses a type on a cycle through `opt`
// alone, which `gen` accepted before.
test("an opt-only cycle is refused with did_type_check_error naming the type", () => {
  const scratch = mkdtempSync(path.join(tmpdir(), "candid-cli-opt-cycle-"));
  writeFileSync(path.join(scratch, "cycle.did"), "type T = opt T;\nservice : { m : (T) -> () };\n");
  const run = gen(["gen", path.join(scratch, "cycle.did")]);
  assert.strictEqual(run.status, 1, run.stderr);
  const response = JSON.parse(run.stdout);
  assert.strictEqual(response.ok, false);
  assert.strictEqual(response.diagnostics[0].code, "did_type_check_error");
  assert.match(
    response.diagnostics[0].message,
    /^type T lies on a cycle that passes only through opt/,
  );
  assert.deepStrictEqual(readdirSync(scratch), ["cycle.did"], "nothing is written");
});

test("usage errors exit 64 with the usage text on stderr", () => {
  for (const argv of [
    [],
    ["frobnicate"],
    ["gen"],
    ["gen", "--json"],
    ["gen", "--check", "-o", "out"],
    ["gen", "a.did", "-o"],
    ["gen", "a.did", "-o", "x", "-o", "y"],
    ["gen", "--typo", "a.did"],
  ]) {
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

// ---------------------------------------------------------------------------
// Several entries, `--json`, `--check`, and the stdin constraint.
//
// Every run below passes *relative* paths with a scratch `cwd`, so the
// documents and file names the CLI prints are machine-independent and can be
// pinned literally.
// ---------------------------------------------------------------------------

const sep = path.sep;
const fixture = (name) => readFileSync(path.join(FIXTURES, `${name}.did`), "utf8");

/** A scratch directory holding `files` (relative path → text). */
function scratchWith(label, files) {
  const dir = mkdtempSync(path.join(tmpdir(), `candid-cli-${label}-`));
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(path.dirname(path.join(dir, name)), { recursive: true });
    writeFileSync(path.join(dir, name), text);
  }
  return dir;
}

/** Run the CLI in `cwd`; `--json` runs return the parsed document too. */
function run(cwd, args) {
  const result = gen(args, { cwd });
  const document = args.includes("--json") ? JSON.parse(result.stdout) : undefined;
  return { ...result, document };
}

const read = (...parts) => readFileSync(path.join(...parts), "utf8");
const snapshot = (dir) =>
  existsSync(dir)
    ? readdirSync(dir).map((name) => {
        const info = statSync(path.join(dir, name));
        return [name, read(dir, name), info.mtimeMs];
      })
    : null;

test("several entries generate in one run, each equal to its single-entry run", () => {
  const scratch = scratchWith("multi", {
    "a/primitives.did": fixture("primitives"),
    "b/ledger.did": fixture("ledger"),
    "c/omissions.did": fixture("omissions"),
  });
  const together = run(scratch, [
    "gen",
    "a/primitives.did",
    "b/ledger.did",
    "c/omissions.did",
    "-o",
    "out",
  ]);
  assert.strictEqual(together.status, 0, `${together.stdout}${together.stderr}`);
  assert.deepStrictEqual(readdirSync(path.join(scratch, "out")).sort(), [
    "ledger.envelope.json",
    "ledger.ts",
    "omissions.envelope.json",
    "omissions.ts",
    "primitives.envelope.json",
    "primitives.ts",
  ]);
  for (const [dir, stem] of [
    ["a", "primitives"],
    ["b", "ledger"],
    ["c", "omissions"],
  ]) {
    const single = run(scratch, ["gen", `${dir}/${stem}.did`, "-o", `single-${stem}`]);
    assert.strictEqual(single.status, 0, `${single.stdout}${single.stderr}`);
    for (const artifact of [`${stem}.ts`, `${stem}.envelope.json`]) {
      assert.deepStrictEqual(
        readFileSync(path.join(scratch, "out", artifact)),
        readFileSync(path.join(scratch, `single-${stem}`, artifact)),
        artifact,
      );
    }
  }
  // The human report: each entry prints its own identities and `wrote` lines
  // in command-line order, and the omissions fixture's warnings go to stderr.
  const wrote = together.stdout.split("\n").filter((line) => line.startsWith("wrote "));
  assert.deepStrictEqual(wrote, [
    `wrote out${sep}primitives.ts`,
    `wrote out${sep}primitives.envelope.json`,
    `wrote out${sep}ledger.ts`,
    `wrote out${sep}ledger.envelope.json`,
    `wrote out${sep}omissions.ts`,
    `wrote out${sep}omissions.envelope.json`,
  ]);
  assert.match(together.stderr, /^warning: omitted /m);
});

test("entries sharing a directory share one bundle and stay independent", () => {
  const scratch = scratchWith("shared", {
    "svc/one.did": 'import "types.did";\nservice : { get : () -> (Item) query };',
    "svc/two.did": 'import "types.did";\nservice : { put : (Item) -> () };',
    "svc/types.did": "type Item = record { id : nat };",
  });
  const together = run(scratch, ["gen", "svc/one.did", "svc/two.did", "-o", "out", "--json"]);
  assert.strictEqual(together.status, 0, `${together.stdout}${together.stderr}`);
  assert.deepStrictEqual(
    together.document.entries.map((entry) => [entry.entry, entry.status]),
    [
      ["svc/one.did", "written"],
      ["svc/two.did", "written"],
    ],
  );
  for (const stem of ["one", "two"]) {
    const single = run(scratch, ["gen", `svc/${stem}.did`, "-o", `single-${stem}`]);
    assert.strictEqual(single.status, 0);
    assert.strictEqual(
      read(scratch, "out", `${stem}.ts`),
      read(scratch, `single-${stem}`, `${stem}.ts`),
    );
  }
});

test("a failing entry is reported on its own and the others still run", () => {
  // `crowded/` is over the 256-source limit: a bundle bound of *its* directory
  // alone, which `good/` (a different root) must not inherit or be charged for.
  const files = {
    "good/good.did": "service : { ping : () -> () };",
    "broken/broken.did": "service : {",
    "crowded/entry.did": "service : {};",
  };
  for (let index = 0; index < 256; index += 1) {
    files[`crowded/extra${index}.did`] = "type T = nat;";
  }
  const scratch = scratchWith("partial-failure", files);
  const result = run(scratch, [
    "gen",
    "broken/broken.did",
    "good/good.did",
    "crowded/entry.did",
    "-o",
    "out",
    "--json",
  ]);
  assert.strictEqual(result.status, 1);
  assert.strictEqual(result.stderr, "");
  const { document } = result;
  assert.strictEqual(document.ok, false);
  assert.deepStrictEqual(
    document.entries.map((entry) => [entry.entry, entry.status]),
    [
      ["broken/broken.did", "failed"],
      ["good/good.did", "written"],
      ["crowded/entry.did", "failed"],
    ],
  );
  assert.strictEqual(document.entries[0].diagnostics[0].code, "did_parse_error");
  assert.deepStrictEqual(document.entries[2].diagnostics[0].resource_limit, {
    resource: "sources",
    limit: 256,
    observed: 257,
  });
  assert.deepStrictEqual(document.entries[1].diagnostics, []);
  // Only the good entry reached the disk.
  assert.deepStrictEqual(readdirSync(path.join(scratch, "out")).sort(), [
    "good.envelope.json",
    "good.ts",
  ]);

  // Without --json the failure documents are on stdout as before, the
  // process still exits 1, and the good entry is still written.
  const human = run(scratch, ["gen", "broken/broken.did", "good/good.did", "-o", "out-human"]);
  assert.strictEqual(human.status, 1);
  assert.match(human.stdout, /"code": "did_parse_error"/);
  assert.match(human.stdout, /^wrote out-human\/good\.ts$/m);
  assert.match(human.stderr, /broken\/broken\.did/);
});

test("duplicate output stems are a usage error before any work", () => {
  const scratch = scratchWith("dup", {
    "a/service.did": "service : { a : () -> () };",
    "b/service.did": "service : { b : () -> () };",
    "c/Service.did": "service : { c : () -> () };",
    "d/other.did": "service : {};",
  });
  for (const args of [
    ["gen", "a/service.did", "b/service.did", "-o", "out"],
    ["gen", "a/service.did", "d/other.did", "a/service.did", "-o", "out", "--json"],
    // A case-folding filesystem would merge these two outputs.
    ["gen", "a/service.did", "c/Service.did", "-o", "out", "--check"],
  ]) {
    const result = gen(args, { cwd: scratch });
    assert.strictEqual(result.status, 64, JSON.stringify(args));
    assert.strictEqual(result.stdout, "", JSON.stringify(args));
    assert.match(result.stderr, /would both write .*service\.ts/i, JSON.stringify(args));
    assert.match(result.stderr, /^usage: candid-core-cli gen/m, JSON.stringify(args));
    assert.strictEqual(existsSync(path.join(scratch, "out")), false, "nothing was written");
  }
  // Distinct stems from different directories are fine.
  const fine = run(scratch, ["gen", "a/service.did", "d/other.did", "-o", "out"]);
  assert.strictEqual(fine.status, 0, `${fine.stdout}${fine.stderr}`);
});

test("--json prints exactly one document on stdout, pinned for success", () => {
  const scratch = scratchWith("json-ok", { "a/basic.did": "service : { ping : () -> () };" });
  const result = run(scratch, ["gen", "a/basic.did", "-o", "out", "--json"]);
  assert.strictEqual(result.status, 0, `${result.stdout}${result.stderr}`);
  assert.strictEqual(result.stderr, "", "nothing but the document, and it is on stdout");
  assert.strictEqual(result.stdout, `${JSON.stringify(result.document, null, 2)}\n`);
  assert.deepStrictEqual(result.document, {
    schemaVersion: 1,
    ok: true,
    check: false,
    entries: [
      {
        entry: "a/basic.did",
        status: "written",
        module: path.join("out", "basic.ts"),
        envelope: path.join("out", "basic.envelope.json"),
        omitted: [],
        diagnostics: [],
      },
    ],
    drift: [],
  });
  // Identities are not part of the document.
  assert.doesNotMatch(result.stdout, /candid-core:(contract|interface):/);
  // The same run again leaves the files alone and says so.
  const again = run(scratch, ["gen", "a/basic.did", "-o", "out", "--json"]);
  assert.strictEqual(again.document.entries[0].status, "unchanged");
  assert.strictEqual(again.document.ok, true);
});

test("--json on a compile error: one document, failed entry, exit 1, nothing written", () => {
  const scratch = scratchWith("json-fail", { "broken.did": "service : {" });
  const result = run(scratch, ["gen", "broken.did", "-o", "out", "--json"]);
  assert.strictEqual(result.status, 1);
  assert.strictEqual(result.stderr, "");
  const { document } = result;
  assert.deepStrictEqual(Object.keys(document), [
    "schemaVersion",
    "ok",
    "check",
    "entries",
    "drift",
  ]);
  assert.strictEqual(document.ok, false);
  const [entry] = document.entries;
  assert.deepStrictEqual(Object.keys(entry), [
    "entry",
    "status",
    "module",
    "envelope",
    "omitted",
    "diagnostics",
  ]);
  assert.strictEqual(entry.status, "failed");
  assert.deepStrictEqual(entry.omitted, []);
  // The wasm's own diagnostics, unchanged in shape.
  assert.strictEqual(entry.diagnostics[0].code, "did_parse_error");
  assert.strictEqual(entry.diagnostics[0].phase, "parse");
  assert.strictEqual(entry.diagnostics[0].severity, "error");
  assert.strictEqual(existsSync(path.join(scratch, "out")), false);

  // A missing entry is a per-entry diagnostic too, not a bare stderr line.
  const missing = run(scratch, ["gen", "absent.did", "--json"]);
  assert.strictEqual(missing.status, 1);
  assert.strictEqual(missing.stderr, "");
  assert.strictEqual(missing.document.entries[0].status, "failed");
  assert.strictEqual(missing.document.entries[0].diagnostics[0].code, "did_source_not_found");
});

test("--json lists omissions and exit 0: the omissions fixture matches the library's list", () => {
  const scratch = scratchWith("json-omit", { "omissions.did": fixture("omissions") });
  const result = run(scratch, ["gen", "omissions.did", "-o", "out", "--json"]);
  assert.strictEqual(result.status, 0, `${result.stdout}${result.stderr}`);
  assert.strictEqual(result.stderr, "");
  assert.strictEqual(result.document.ok, true);
  const [entry] = result.document.entries;
  assert.strictEqual(entry.status, "written");
  assert.deepStrictEqual(
    entry.omitted,
    JSON.parse(readFileSync(path.join(GOLDENS, "omissions.omitted.json"), "utf8")),
  );
  assert.ok(entry.omitted.length > 0);
  assert.deepStrictEqual(entry.diagnostics, []);
});

test("--check: clean tree exits 0, one touched byte exits 1, and nothing is ever written", () => {
  const scratch = scratchWith("check", {
    "a/primitives.did": fixture("primitives"),
    "b/ledger.did": fixture("ledger"),
  });
  const entries = ["a/primitives.did", "b/ledger.did"];
  assert.strictEqual(run(scratch, ["gen", ...entries, "-o", "out"]).status, 0);

  const clean = run(scratch, ["gen", ...entries, "-o", "out", "--check", "--json"]);
  assert.strictEqual(clean.status, 0, `${clean.stdout}${clean.stderr}`);
  assert.strictEqual(clean.stderr, "");
  assert.strictEqual(clean.document.ok, true);
  assert.strictEqual(clean.document.check, true);
  assert.deepStrictEqual(
    clean.document.entries.map((entry) => entry.status),
    ["unchanged", "unchanged"],
  );
  assert.deepStrictEqual(clean.document.drift, []);

  // Age the files so a rewrite would be visible in mtime, then drift one byte.
  const out = path.join(scratch, "out");
  const past = new Date(Date.now() - 3_600_000);
  for (const name of readdirSync(out)) {
    utimesSync(path.join(out, name), past, past);
  }
  const target = path.join(out, "ledger.ts");
  writeFileSync(target, `${read(target)} `);
  utimesSync(target, past, past);
  const before = snapshot(out);

  const drifted = run(scratch, ["gen", ...entries, "-o", "out", "--check", "--json"]);
  assert.strictEqual(drifted.status, 1);
  assert.strictEqual(drifted.stderr, "");
  assert.strictEqual(drifted.document.ok, false);
  assert.deepStrictEqual(
    drifted.document.entries.map((entry) => entry.status),
    ["unchanged", "drifted"],
  );
  assert.deepStrictEqual(drifted.document.drift, [path.join("out", "ledger.ts")]);
  assert.deepStrictEqual(snapshot(out), before, "--check changed a file or its mtime");

  // The human report lists the drifted path on stderr and exits 1 too.
  const human = run(scratch, ["gen", ...entries, "-o", "out", "--check"]);
  assert.strictEqual(human.status, 1);
  assert.match(human.stderr, new RegExp(`^drifted out\\${sep}ledger\\.ts$`, "m"));
  assert.match(human.stderr, /1 file\(s\) differ/);
  assert.deepStrictEqual(snapshot(out), before);

  // A missing file is drift as well, and --check does not create the tree.
  const absent = run(scratch, ["gen", ...entries, "-o", "fresh", "--check", "--json"]);
  assert.strictEqual(absent.status, 1);
  assert.strictEqual(absent.document.drift.length, 4);
  assert.strictEqual(existsSync(path.join(scratch, "fresh")), false);
  const absentHuman = run(scratch, ["gen", entries[0], "-o", "fresh", "--check"]);
  assert.match(absentHuman.stderr, /^missing fresh\/primitives\.ts$/m);
  assert.strictEqual(existsSync(path.join(scratch, "fresh")), false);

  // Regenerating repairs the drift, after which the check is clean again.
  assert.strictEqual(run(scratch, ["gen", ...entries, "-o", "out"]).status, 0);
  assert.strictEqual(run(scratch, ["gen", ...entries, "-o", "out", "--check"]).status, 0);
});

test("--check reports a compile error as a failed entry, not as drift", () => {
  const scratch = scratchWith("check-fail", { "broken.did": "service : {" });
  const result = run(scratch, ["gen", "broken.did", "-o", "out", "--check", "--json"]);
  assert.strictEqual(result.status, 1);
  assert.strictEqual(result.document.entries[0].status, "failed");
  assert.deepStrictEqual(result.document.drift, []);
});

test("a second run changes nothing: identical files, identical documents, no rewrite", () => {
  const files = { "x/ledger.did": fixture("ledger"), "y/omissions.did": fixture("omissions") };
  const args = ["gen", "x/ledger.did", "y/omissions.did", "-o", "out", "--json"];
  // Two independent directories, same relative layout: the documents must be
  // byte-for-byte equal — no timestamps, no absolute paths, no ordering luck.
  const first = scratchWith("det-a", files);
  const second = scratchWith("det-b", files);
  const a = gen(args, { cwd: first });
  const b = gen(args, { cwd: second });
  assert.strictEqual(a.status, 0);
  assert.strictEqual(a.stdout, b.stdout);
  for (const name of readdirSync(path.join(first, "out"))) {
    assert.deepStrictEqual(
      readFileSync(path.join(first, "out", name)),
      readFileSync(path.join(second, "out", name)),
      name,
    );
  }
  // Re-running in place leaves every file (and mtime) alone; the only
  // difference in the document is each entry's status.
  const past = new Date(Date.now() - 3_600_000);
  for (const name of readdirSync(path.join(first, "out"))) {
    utimesSync(path.join(first, "out", name), past, past);
  }
  const before = snapshot(path.join(first, "out"));
  const again = gen(args, { cwd: first });
  assert.strictEqual(again.status, 0);
  assert.deepStrictEqual(snapshot(path.join(first, "out")), before);
  const expected = JSON.parse(a.stdout);
  for (const entry of expected.entries) {
    entry.status = "unchanged";
  }
  assert.deepStrictEqual(JSON.parse(again.stdout), expected);
  // And the check document is itself reproducible.
  const checkArgs = [...args, "--check"];
  assert.strictEqual(gen(checkArgs, { cwd: first }).stdout, gen(checkArgs, { cwd: second }).stdout);
});

// A report must not depend on where it was produced: the same failing
// invocation from two checkouts prints the same bytes, and no absolute host
// path appears unless the user passed one (review finding on this PR).
test("every reported path is as the user passed it, never an absolute host path", () => {
  // `gen missing.did` walks "." recursively, so the over-limit tree lives in
  // its own pair of checkouts.
  const small = {
    "good/good.did": "service : { ping : () -> () };",
    "broken/broken.did": "service : {",
    "stale/stale.did": "service : { ping : () -> () };",
  };
  const crowded = { "crowded/entry.did": "service : {};" };
  for (let index = 0; index < 256; index += 1) {
    crowded[`crowded/extra${index}.did`] = "type T = nat;";
  }
  const checkouts = {
    small: [scratchWith("paths-a", small), scratchWith("paths-b", small)],
    crowded: [scratchWith("paths-c", crowded), scratchWith("paths-d", crowded)],
  };
  const invocations = [
    ["small", ["gen", "missing.did", "--json"]],
    ["small", ["gen", "./nodir/x.did", "-o", "./out", "--json"]],
    ["small", ["gen", "broken/broken.did", "good/good.did", "-o", "out", "--json"]],
    ["crowded", ["gen", "crowded/entry.did", "--json"]],
    // --check: stale.did has no output yet, so its files are missing drift.
    ["small", ["gen", "stale/stale.did", "-o", "out", "--check", "--json"]],
    // The human report says the same thing the document does.
    ["small", ["gen", "missing.did"]],
    ["small", ["gen", "./nodir/x.did"]],
    ["crowded", ["gen", "crowded/entry.did"]],
    ["small", ["gen", "stale/stale.did", "-o", "out", "--check"]],
  ];
  for (const [which, args] of invocations) {
    const [first, second] = checkouts[which];
    const a = gen(args, { cwd: first });
    const b = gen(args, { cwd: second });
    const label = args.join(" ");
    assert.notStrictEqual(a.status, 64, label);
    assert.strictEqual(a.status, b.status, label);
    assert.strictEqual(a.stdout, b.stdout, `${label}: stdout differs between checkouts`);
    assert.strictEqual(a.stderr, b.stderr, `${label}: stderr differs between checkouts`);
    const hosts = [first, realpathSync(first), path.dirname(first)];
    for (const text of [a.stdout, a.stderr]) {
      for (const host of hosts) {
        assert.ok(!text.includes(host), `${label} leaks ${host}: ${text}`);
      }
      assert.doesNotMatch(text, /(^|[\s'"(])\/[\w.-]/, `${label} holds an absolute path`);
    }
  }

  // The messages name what the user typed.
  const [first, second] = checkouts.small;
  const missing = run(first, ["gen", "missing.did", "--json"]);
  assert.strictEqual(
    missing.document.entries[0].diagnostics[0].message,
    "cannot read missing.did: no such .did file",
  );
  assert.match(
    run(first, ["gen", "./nodir/x.did", "--json"]).document.entries[0].diagnostics[0].message,
    /^cannot read \.\/nodir: ENOENT/,
  );
  assert.deepStrictEqual(
    run(first, ["gen", "stale/stale.did", "-o", "out", "--check", "--json"]).document.drift,
    [path.join("out", "stale.ts"), path.join("out", "stale.envelope.json")],
  );
  // An absolute path the user did pass is reported as passed.
  const absolute = path.join(first, "absent.did");
  const given = run(second, ["gen", absolute, "--json"]);
  assert.strictEqual(given.document.entries[0].entry, absolute);
  assert.strictEqual(
    given.document.entries[0].diagnostics[0].message,
    `cannot read ${absolute}: no such .did file`,
  );
});

// The CLI reads files and nothing else. A stdin that is an open pipe nobody
// ever writes to or closes would hang any read, so completing proves none
// happened; a stdin that is closed outright (`/dev/null`) must be as harmless.
test("stdin is never read: an open, silent pipe and a closed stdin both complete", async () => {
  const scratch = scratchWith("stdin", { "a.did": "service : { ping : () -> () };" });
  for (const argv of [
    ["gen", "a.did", "-o", "out"],
    ["gen", "a.did", "-o", "out", "--json"],
    ["gen", "a.did", "-o", "out", "--check", "--json"],
  ]) {
    const child = spawn(process.execPath, [CLI, ...argv], {
      cwd: scratch,
      stdio: ["pipe", "pipe", "pipe"], // stdin stays open; nothing is written to it
    });
    let stdout = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    const outcome = await new Promise((resolve) => {
      const timer = setTimeout(() => {
        child.kill("SIGKILL");
        resolve({ hung: true });
      }, 20_000);
      child.on("close", (status, signal) => {
        clearTimeout(timer);
        resolve({ hung: false, status, signal });
      });
    });
    child.stdin.destroy();
    assert.strictEqual(outcome.hung, false, `${argv.join(" ")} blocked on stdin`);
    // The `--check` run is last, so it finds the files the first run wrote.
    assert.strictEqual(outcome.status, 0, `${argv.join(" ")}: ${stdout}`);
  }
  const closed = gen(["gen", "a.did", "-o", "out", "--json"], {
    cwd: scratch,
    stdio: ["ignore", "pipe", "pipe"],
  });
  assert.strictEqual(closed.status, 0);
  assert.strictEqual(JSON.parse(closed.stdout).ok, true);
});
