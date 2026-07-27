#!/usr/bin/env node
// Checks the claims the demo page makes, without a browser.
//
// The page asserts specific relationships between specific releases — that the
// 1.2.1 repackage is the same Contract as 1.2.0, that 2.0.0 is the only
// breaking step, that one frontend serves the whole fleet. Those are properties
// of the `.did` files under `app/wallets/`, and an edit to any of them can
// falsify one silently. This runs the page's own modules against the same
// sources in Node and fails loudly instead.
//
//     node website/verify.mjs
//
// Requires `website/build.sh` to have produced the module first.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { loadCandidCore } from "./app/js/candid-core.js";
import { loadRegistry } from "./app/js/registry.js";
import { compareInterfaces, shortId } from "./app/js/contract.js";
import { negotiate, ALL_LINES } from "./app/js/dapp.js";

const here = dirname(fileURLToPath(import.meta.url));
const app = join(here, "app");

const failures = [];
function check(description, condition, detail = "") {
  if (condition) {
    console.log(`  ok   ${description}`);
  } else {
    console.log(`  FAIL ${description}${detail ? ` — ${detail}` : ""}`);
    failures.push(description);
  }
}

const module = await readFile(join(app, "candid_core_web.wasm")).catch(() => {
  console.error("website/app/candid_core_web.wasm is missing — run website/build.sh first.");
  process.exit(2);
});
const core = await loadCandidCore(module);
const producer = core.producer();
console.log(
  `candid-core ${producer.version} (candid ${producer.candid_version}, ` +
    `candid_parser ${producer.candid_parser_version})\n`,
);

const registry = await loadRegistry(core, "wallets", (path) => readFile(join(app, path), "utf8"));

console.log("Releases");
for (const version of registry.versions) {
  if (!version.ok) {
    console.log(`  FAIL ${version.id} did not compile`);
    console.log(JSON.stringify(version.failure, null, 2));
    failures.push(`${version.id} compiles`);
    continue;
  }
  console.log(
    `  ok   ${version.id.padEnd(8)} interface ${shortId(version.interfaceId)}  ` +
      `contract ${shortId(version.contractId)}  bundle ${shortId(version.sourceBundleId)}  ` +
      `${version.methods.length} methods`,
  );
}
if (failures.length > 0) process.exit(1);

const at = (id) => registry.byId.get(id);

console.log("\nThe repackage claim: 1.2.1 is 1.2.0 with the source rewritten");
check("contract_id is unchanged", at("v1.2.0").contractId === at("v1.2.1").contractId);
check("interface_id is unchanged", at("v1.2.0").interfaceId === at("v1.2.1").interfaceId);
check(
  "artifact_id is unchanged — the Contract documents are byte-identical",
  at("v1.2.0").artifactId === at("v1.2.1").artifactId,
);
check(
  "source_bundle_id moves — the raw sources really did change",
  at("v1.2.0").sourceBundleId !== at("v1.2.1").sourceBundleId,
);
check(
  "the two bundles do not even have the same file names",
  JSON.stringify(Object.keys(at("v1.2.0").sources)) !==
    JSON.stringify(Object.keys(at("v1.2.1").sources)),
);

console.log("\nUpgrade steps");
const steps = [
  ["v1.0.0", "v1.1.0", "compatible"],
  ["v1.1.0", "v1.2.0", "compatible"],
  ["v1.2.0", "v1.2.1", "identical"],
  ["v1.2.1", "v2.0.0", "breaking"],
  ["v2.0.0", "v2.1.0", "compatible"],
];
for (const [from, to, expected] of steps) {
  const comparison = compareInterfaces(at(from), at(to));
  check(`${from} → ${to} is ${expected}`, comparison.verdict === expected, comparison.verdict);
}

const breaking = compareInterfaces(at("v1.2.1"), at("v2.0.0"));
check("2.0.0 removes owner", breaking.removed.join(",") === "owner", breaking.removed.join(","));
check("2.0.0 changes transfer", breaking.changed.join(",") === "transfer", breaking.changed.join(","));
check("2.0.0 adds controllers", breaking.added.join(",") === "controllers", breaking.added.join(","));

console.log("\nThe fleet, against a frontend that speaks both lines");
for (const wallet of registry.fleet) {
  const result = negotiate(wallet.release, ALL_LINES, registry.byId);
  const optional = result.failed.map((entry) => entry.capability.id).join(", ");
  check(
    `${wallet.holder.padEnd(5)} on ${wallet.version.padEnd(7)} is not blocked` +
      (optional ? ` (no ${optional})` : " (every capability served)"),
    result.status !== "blocked",
    result.blocking.map((entry) => entry.capability.id).join(", "),
  );
}

console.log("\nThe same fleet, if the frontend dropped 1.x support");
const dropped = registry.fleet.filter(
  (wallet) => negotiate(wallet.release, ["2.x"], registry.byId).status === "blocked",
);
check(
  "every 1.x wallet is blocked, and no 2.x wallet is",
  dropped.length === registry.fleet.filter((wallet) => wallet.release.line === "1.x").length &&
    dropped.every((wallet) => wallet.release.line === "1.x"),
  `${dropped.length} blocked`,
);

console.log("\nBounds still apply in the browser");
const oversized = core.compile({
  entry: "memory:/wallet.did",
  sources: { "memory:/wallet.did": `service : { m : () -> () };\n${"// pad\n".repeat(160_000)}` },
});
check(
  "an over-limit source is rejected structurally, not by running out of memory",
  oversized.ok === false && oversized.diagnostics?.[0]?.code === "resource_limit_exceeded",
  JSON.stringify(oversized.diagnostics?.[0] ?? oversized).slice(0, 160),
);

const unresolved = core.compile({
  entry: "memory:/wallet.did",
  sources: { "memory:/wallet.did": 'import "nowhere.did"; service : { m : () -> () };' },
});
check(
  "an import the page did not supply is an error, not a fetch",
  unresolved.ok === false,
  JSON.stringify(unresolved.diagnostics?.[0] ?? {}).slice(0, 160),
);

console.log("");
if (failures.length > 0) {
  console.error(`${failures.length} check(s) failed.`);
  process.exit(1);
}
console.log("All checks passed.");
