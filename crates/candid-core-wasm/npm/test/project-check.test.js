// `project` and `check`, through the real bin/cli.js and the compiled
// WebAssembly, plus the library functions behind them. The projection is
// compared byte-for-byte with the golden the Rust-native tests pin
// (crates/candid-core-wasm/tests/goldens/ledger.projection.did), so the
// same code compiled twice gives the same bytes.

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { checkCompatible, projectDid } from "../lib/index.js";

const HERE = new URL(".", import.meta.url).pathname;
const CLI = path.join(HERE, "..", "bin", "cli.js");
const REPO = path.join(HERE, "..", "..", "..", "..");
const LEDGER = readFileSync(
  path.join(REPO, "crates", "candid-core-ts", "tests", "fixtures", "ledger.did"),
  "utf8",
);
const GOLDEN = readFileSync(
  path.join(REPO, "crates", "candid-core-wasm", "tests", "goldens", "ledger.projection.did"),
  "utf8",
);

function cli(args, cwd) {
  return spawnSync(process.execPath, [CLI, ...args], { encoding: "utf8", cwd });
}

/** A scratch directory holding `live/ledger.did` and an empty `app/`. */
function scratch() {
  const root = mkdtempSync(path.join(tmpdir(), "candid-cli-project-"));
  mkdirSync(path.join(root, "live"));
  mkdirSync(path.join(root, "app"));
  writeFileSync(path.join(root, "live", "ledger.did"), LEDGER);
  return root;
}

test("project writes exactly the named methods and the types they reach, byte-stable", () => {
  const root = scratch();
  const first = cli(
    ["project", "live/ledger.did", "--methods", "transfer,balance_of", "-o", "app/ledger.did"],
    root,
  );
  assert.equal(first.status, 0, first.stderr);
  assert.match(first.stdout, /^input: {6}candid-core:interface:v1:sha256:[0-9a-f]{64}$/m);
  assert.match(first.stdout, /^projection: candid-core:interface:v1:sha256:[0-9a-f]{64}$/m);
  assert.match(first.stdout, /^wrote app\/ledger\.did$/m);
  const written = readFileSync(path.join(root, "app", "ledger.did"), "utf8");
  assert.equal(written, GOLDEN, "the wasm build must print the Rust-native golden");

  // A second run, and a run naming the methods in another order, change
  // nothing.
  for (const methods of ["transfer,balance_of", "balance_of,transfer,balance_of"]) {
    const again = cli(
      ["project", "live/ledger.did", "--methods", methods, "-o", "app/ledger.did", "--json"],
      root,
    );
    assert.equal(again.status, 0, again.stderr);
    assert.equal(again.stderr, "");
    const report = JSON.parse(again.stdout);
    assert.equal(report.ok, true);
    assert.equal(report.status, "unchanged");
    assert.equal(report.output, "app/ledger.did");
    assert.deepEqual(report.methods, ["balance_of", "transfer"]);
    assert.notEqual(report.input.interface_id, report.projection.interface_id);
    assert.equal(readFileSync(path.join(root, "app", "ledger.did"), "utf8"), written);
  }
});

test("gen on a projection lists only the projected methods, each with its mode", () => {
  const root = scratch();
  const projected = cli(
    ["project", "live/ledger.did", "--methods", "balance_of,transfer", "-o", "app/ledger.did"],
    root,
  );
  assert.equal(projected.status, 0, projected.stderr);
  const generated = cli(["gen", "app/ledger.did", "-o", "out"], root);
  assert.equal(generated.status, 0, generated.stderr);
  const module = readFileSync(path.join(root, "out", "ledger.ts"), "utf8");
  const actor = module.slice(module.indexOf("type $Actor = {"));
  const listed = actor
    .slice(0, actor.indexOf("};"))
    .split("\n")
    .slice(1)
    .map((line) => line.trim().split(":")[0])
    .filter((name) => name !== "");
  assert.deepEqual(listed, ["balance_of", "transfer"]);
  assert.match(module, /balance_of: \$\.c\.func\(\[\$Account\], \[\$Tokens\], "query"\)/);
  assert.match(module, /transfer: \$\.c\.func\(\[\$TransferArg\], \[\$TransferResult\], "update"\)/);
});

test("a service class with no declarations projects to its service", async () => {
  const root = scratch();
  const source = readFileSync(
    path.join(REPO, "tests", "fixtures", "conformance", "class.did"),
    "utf8",
  );
  writeFileSync(path.join(root, "live", "class.did"), source);
  const projected = cli(
    ["project", "live/class.did", "--methods", "get", "-o", "app/class.did"],
    root,
  );
  assert.equal(projected.status, 0, projected.stderr);
  assert.equal(
    readFileSync(path.join(root, "app", "class.did"), "utf8"),
    "service : {\n  get : () -> (nat) query;\n}\n",
  );
  const library = await projectDid(
    { source: "type S = service { get : () -> (nat) query };\nservice : (nat) -> S\n" },
    ["get"],
  );
  assert.equal(library.ok, true, JSON.stringify(library));
  assert.equal(library.did, "service : {\n  get : () -> (nat) query;\n}\n");
});

test("an unknown method and an empty list each fail with a diagnostic", () => {
  const root = scratch();
  const unknown = cli(
    ["project", "live/ledger.did", "--methods", "balance_of,nope", "-o", "app/x.did"],
    root,
  );
  assert.equal(unknown.status, 1);
  const failure = JSON.parse(unknown.stdout);
  assert.equal(failure.ok, false);
  assert.equal(failure.diagnostics.length, 1);
  assert.equal(failure.diagnostics[0].code, "unknown_method");
  assert.deepEqual(failure.diagnostics[0].notes, [
    "balance_of",
    "decimals",
    "fee",
    "get_transactions",
    "name",
    "symbol",
    "total_supply",
    "transfer",
  ]);
  assert.match(failure.diagnostics[0].message, /"nope".*balance_of, decimals, fee/);
  assert.equal(existsSync(path.join(root, "app", "x.did")), false, "nothing is written");

  const empty = cli(["project", "live/ledger.did", "--methods", "", "-o", "app/x.did"], root);
  assert.equal(empty.status, 1);
  assert.equal(JSON.parse(empty.stdout).diagnostics[0].code, "empty_method_list");
  assert.equal(existsSync(path.join(root, "app", "x.did")), false);
});

/** Write `written.did` and `live.did` into their own directories and check. */
function checkPair(written, live, extra = []) {
  const root = mkdtempSync(path.join(tmpdir(), "candid-cli-check-"));
  mkdirSync(path.join(root, "w"));
  mkdirSync(path.join(root, "l"));
  writeFileSync(path.join(root, "w", "written.did"), written);
  writeFileSync(path.join(root, "l", "live.did"), live);
  return cli(["check", "w/written.did", "--against", "l/live.did", ...extra], root);
}

test("check exits 0 for extra live methods and compatible changes", () => {
  const written = "type R = record { x : nat };\nservice : { get : () -> (R) query }\n";
  const live =
    "type R = record { x : nat; added : opt text };\n" +
    "service : { get : () -> (R) query; more : () -> () }\n";
  const result = checkPair(written, live);
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /^written: candid-core:interface:v1:sha256:[0-9a-f]{64}$/m);
  assert.match(result.stdout, /^live: {4}candid-core:interface:v1:sha256:[0-9a-f]{64}$/m);
  assert.match(result.stdout, /^compatible: 0 error\(s\), 0 warning\(s\)$/m);
  assert.equal(result.stderr, "");
});

test("check exits 1 with a per-method diagnostic for a missing, re-moded or changed method", () => {
  const written =
    "service : {\n  gone : () -> ();\n  moded : () -> (nat) query;\n  changed : () -> (nat) query;\n  same : () -> ();\n}\n";
  const live =
    "service : {\n  moded : () -> (nat);\n  changed : () -> (text) query;\n  same : () -> ();\n}\n";
  const human = checkPair(written, live);
  assert.equal(human.status, 1);
  assert.equal(
    human.stderr,
    [
      "error: changed: method_incompatible at $results[0]: text is not a subtype of nat",
      'error: gone: method_missing: the live service has no method "gone"',
      "error: moded: mode_changed: the written method is query and the live one is update",
      "",
    ].join("\n"),
  );
  assert.match(human.stdout, /^incompatible: 3 error\(s\), 0 warning\(s\)$/m);

  const json = checkPair(written, live, ["--json"]);
  assert.equal(json.status, 1);
  assert.equal(json.stderr, "");
  const report = JSON.parse(json.stdout);
  assert.equal(report.ok, true);
  assert.equal(report.compatible, false);
  assert.deepEqual(
    report.diagnostics.map((item) => [item.code, item.method, item.path ?? null, item.severity]),
    [
      ["method_incompatible", "changed", "$results[0]", "error"],
      ["method_missing", "gone", null, "error"],
      ["mode_changed", "moded", null, "error"],
    ],
  );
});

test("a special-opt-rule change is a warning and still exits 0", () => {
  const result = checkPair(
    "service : { get : () -> (opt nat) query }\n",
    "service : { get : () -> (opt text) query }\n",
  );
  assert.equal(result.status, 0);
  assert.match(result.stderr, /^warning: get: special_opt_rule at \$results\[0\]: /m);
  assert.match(result.stdout, /^compatible: 0 error\(s\), 1 warning\(s\)$/m);
});

test("past the warning bound the first 1,000 are reported and the check still exits 0", () => {
  // 2^10 = 1,024 paths decode as null.
  const doubling = (content) => {
    let source = `type D0 = opt ${content};\n`;
    for (let level = 1; level <= 10; level += 1) {
      source += `type D${level} = record { l : D${level - 1}; r : D${level - 1} };\n`;
    }
    return `${source}service : { get : () -> (D10) }\n`;
  };
  const result = checkPair(doubling("nat"), doubling("text"));
  assert.equal(result.status, 0);
  assert.match(
    result.stderr,
    /^warning: get: resource_limit_exceeded: this method has more special_opt_rule warnings than its check_warnings bound of 1000; /m,
  );
  assert.match(result.stdout, /^compatible: 0 error\(s\), 1001 warning\(s\)$/m);
});

test("check reports which side failed to compile, and a missing file", () => {
  const broken = checkPair("service : { get : () -> (Missing) }\n", "service : {}\n");
  assert.equal(broken.status, 1);
  const failure = JSON.parse(broken.stdout);
  assert.equal(failure.ok, false);
  assert.equal(failure.input, "written");
  assert.equal(failure.diagnostics[0].code, "did_type_check_error");

  const root = mkdtempSync(path.join(tmpdir(), "candid-cli-check-"));
  writeFileSync(path.join(root, "w.did"), "service : {}\n");
  const missing = cli(["check", "w.did", "--against", "nowhere/live.did"], root);
  assert.equal(missing.status, 1);
  assert.match(missing.stderr, /cannot read nowhere/);
});

test("usage errors exit 64 without touching anything", () => {
  const root = scratch();
  for (const args of [
    ["project", "live/ledger.did", "-o", "app/x.did"],
    ["project", "live/ledger.did", "--methods", "fee"],
    ["project", "--methods", "fee", "-o", "app/x.did"],
    ["project", "live/ledger.did", "live/ledger.did", "--methods", "fee", "-o", "app/x.did"],
    ["project", "live/ledger.did", "--methods", "fee", "--methods", "name", "-o", "app/x.did"],
    ["project", "live/ledger.did", "--methods", "fee", "-o", "app/x.did", "--check"],
    ["project", "live/ledger.did", "--methods", "fee", "-o", "live/ledger.did"],
    ["check", "live/ledger.did"],
    ["check", "--against", "live/ledger.did"],
    ["check", "a.did", "b.did", "--against", "live/ledger.did"],
  ]) {
    const result = cli(args, root);
    assert.equal(result.status, 64, args.join(" "));
    assert.equal(result.stdout, "", args.join(" "));
    assert.match(result.stderr, /usage: candid-core-cli gen/, args.join(" "));
  }
  assert.equal(readFileSync(path.join(root, "live", "ledger.did"), "utf8"), LEDGER);
  assert.equal(existsSync(path.join(root, "app", "x.did")), false);
});

test("the library: projectDid and checkCompatible, data in and data out", async () => {
  const projected = await projectDid(LEDGER, ["transfer", "balance_of"]);
  assert.equal(projected.ok, true);
  assert.equal(projected.did, GOLDEN);
  assert.deepEqual(projected.methods, ["balance_of", "transfer"]);

  const bundle = await projectDid(
    {
      entry: "app.did",
      files: {
        "app.did": 'import "types.did";\nservice : { get : () -> (Item) query; put : (Item) -> () }',
        "types.did": "type Item = record { id : nat };",
      },
    },
    ["get"],
  );
  assert.equal(bundle.ok, true);
  assert.equal(bundle.did, "type Item = record {\n  id : nat;\n};\nservice : {\n  get : () -> (Item) query;\n}\n");

  const report = await checkCompatible(projected.did, LEDGER);
  assert.equal(report.ok, true);
  assert.equal(report.compatible, true);
  assert.deepEqual(report.diagnostics, []);
  assert.equal(report.live.interface_id, projected.input.interface_id);
  assert.equal(report.written.interface_id, projected.projection.interface_id);

  const reverse = await checkCompatible(LEDGER, projected.did);
  assert.equal(reverse.compatible, false);
  assert.ok(reverse.diagnostics.every((item) => item.code === "method_missing"));

  const unknown = await projectDid(LEDGER, ["nope"]);
  assert.equal(unknown.ok, false);
  assert.equal(unknown.diagnostics[0].code, "unknown_method");

  const invalid = await checkCompatible({ source: "service : {}", methods: [] }, LEDGER);
  assert.equal(invalid.ok, false);
  assert.equal(invalid.diagnostics[0].code, "invalid_request");
});

test("the compiled check reaches its depth bound and fails closed, inside the wasm stack", async () => {
  // Two rings of records with coprime lengths, 19 and 23: every pair holds,
  // and the pairs repeat only after 437 steps, past the 384-pair bound. A
  // stack overflow here would trap instead of returning a document.
  const ring = (prefix, marker, length) => {
    let source = "";
    for (let i = 0; i < length; i += 1) {
      const mark = i === 0 ? `; ${marker} : reserved` : "";
      source += `type ${prefix}${i} = record { n : ${prefix}${(i + 1) % length}${mark} };\n`;
    }
    return `${source}service : { get : () -> (${prefix}0) query }\n`;
  };
  const report = await checkCompatible(ring("W", "k", 19), ring("L", "j", 23));
  assert.equal(report.ok, true);
  assert.equal(report.compatible, false);
  assert.equal(report.diagnostics[0].code, "resource_limit_exceeded");
  assert.deepEqual(report.diagnostics[0].resource_limit, {
    resource: "check_depth",
    limit: 384,
    observed: 385,
  });
});

test("the package still has no runtime dependencies", () => {
  const manifest = JSON.parse(readFileSync(path.join(HERE, "..", "package.json"), "utf8"));
  assert.equal(manifest.dependencies, undefined);
  assert.equal(manifest.optionalDependencies, undefined);
});
