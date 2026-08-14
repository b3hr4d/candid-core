#!/usr/bin/env node
// The @candid-core/cli entry point (issue #153):
//
//   candid-core-cli gen <service.did> [-o <dir>]
//
// The JS host does all the I/O: it reads the entry file and every `.did`
// beneath the entry's directory, hands them to the wasm compiler as data,
// and writes the two artifacts — `<stem>.ts` (the generated module) and
// `<stem>.envelope.json` (the one-document contract envelope with field
// names) — printing the content-addressed identities on stdout.
//
// Conventions follow the native `candid-core` binary: anything outside the
// grammar is a usage error (exit 64, usage on stderr, nothing on stdout);
// a compile or generation failure prints its JSON diagnostics document on
// stdout and exits 1. Determinism is enforced, not assumed: every
// generation runs twice and the run refuses on any byte mismatch.

import { readFile, readdir, mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

import { didToContract, didToModule, init } from "../lib/index.js";

const USAGE = "usage: candid-core-cli gen <service.did> [-o <dir>]";

function usage() {
  console.error(USAGE);
  process.exit(64);
}

function parseArguments(argv) {
  if (argv.length === 0 || argv[0] !== "gen") {
    usage();
  }
  let entry;
  let outDir;
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "-o") {
      if (outDir !== undefined || index + 1 >= argv.length) {
        usage();
      }
      index += 1;
      outDir = argv[index];
    } else if (argument.startsWith("-")) {
      usage();
    } else if (entry === undefined) {
      entry = argument;
    } else {
      usage();
    }
  }
  if (entry === undefined) {
    usage();
  }
  return { entry, outDir: outDir ?? "." };
}

/** Every `.did` under `root`, keyed by `/`-separated path relative to it. */
async function didFiles(root) {
  const files = {};
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  for (const item of entries) {
    if (!item.isFile() || !item.name.endsWith(".did")) {
      continue;
    }
    const absolute = path.join(item.parentPath ?? item.path, item.name);
    const relative = path.relative(root, absolute).split(path.sep).join("/");
    files[relative] = await readFile(absolute, "utf8");
  }
  return files;
}

/** Run a generation twice and refuse on any byte mismatch. */
async function deterministic(label, produce) {
  const first = await produce();
  const second = await produce();
  if (JSON.stringify(first) !== JSON.stringify(second)) {
    console.error(`determinism check failed: two ${label} runs disagreed; refusing to write`);
    process.exit(1);
  }
  return first;
}

const { entry, outDir } = parseArguments(process.argv.slice(2));

const entryPath = path.resolve(entry);
const root = path.dirname(entryPath);
const entryName = path.basename(entryPath);
let files;
try {
  files = await didFiles(root);
} catch (error) {
  console.error(`cannot read ${root}: ${error.message}`);
  process.exit(1);
}
if (files[entryName] === undefined) {
  console.error(`cannot read ${entryPath}: no such .did file`);
  process.exit(1);
}

await init();
const sources = { entry: entryName, files };

const envelope = await deterministic("contract", () => didToContract(sources));
if (!("contract" in envelope)) {
  console.log(JSON.stringify(envelope, null, 2));
  process.exit(1);
}
const generated = await deterministic("module", () => didToModule(sources));
if (!generated.ok) {
  console.log(JSON.stringify(generated, null, 2));
  process.exit(1);
}

const stem = entryName.replace(/\.did$/, "");
await mkdir(outDir, { recursive: true });
const modulePath = path.join(outDir, `${stem}.ts`);
const envelopePath = path.join(outDir, `${stem}.envelope.json`);
await writeFile(modulePath, generated.module);
await writeFile(envelopePath, `${JSON.stringify(envelope, null, 2)}\n`);

const identities = envelope.contract.identities ?? {};
if (identities.contract !== undefined) {
  console.log(`contract:  ${identities.contract}`);
}
if (identities.interface !== undefined) {
  console.log(`interface: ${identities.interface}`);
}
console.log(`wrote ${modulePath}`);
console.log(`wrote ${envelopePath}`);
