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

import { readFile, readdir, mkdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

import { didToContract, didToModule, init } from "../lib/index.js";

const USAGE = "usage: candid-core-cli gen <service.did> [-o <dir>]";

// The compiler's own source bounds (`Limits::default()` — the values the
// root README documents), enforced *while walking*: the entry's whole
// directory tree is the bundle this CLI hands over, so an oversized tree
// must fail with the structured resource diagnostic the compiler would
// produce, never by exhausting the JS heap before the wasm side can check.
const MAX_SOURCE_BYTES = 1_048_576; // max_source_bytes, per file
const MAX_BUNDLE_BYTES = 8_388_608; // max_bundle_bytes, aggregate
const MAX_SOURCES = 256; // max_sources, file count

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

/** Print a structured resource refusal on stdout and exit 1 — the same
 * channel and item shape a compile failure uses. */
function resourceFailure(resource, limit, observed, message) {
  const document = {
    ok: false,
    diagnostics: [
      {
        code: "resource_limit_exceeded",
        phase: "load",
        severity: "error",
        message,
        resource_limit: { resource, limit, observed },
      },
    ],
  };
  console.log(JSON.stringify(document, null, 2));
  process.exit(1);
}

/**
 * Every `.did` under `root`, keyed by `/`-separated path relative to it —
 * bounded before anything is read: file count, per-file bytes (by `stat`),
 * and aggregate bytes are all checked against the compiler's limits first,
 * so the refusal is a diagnostic, not an out-of-memory abort.
 */
async function didFiles(root) {
  const candidates = [];
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  for (const item of entries) {
    if (!item.isFile() || !item.name.endsWith(".did")) {
      continue;
    }
    const absolute = path.join(item.parentPath ?? item.path, item.name);
    const relative = path.relative(root, absolute).split(path.sep).join("/");
    candidates.push({ absolute, relative });
  }
  if (candidates.length > MAX_SOURCES) {
    resourceFailure(
      "sources",
      MAX_SOURCES,
      candidates.length,
      `the bundle directory holds ${candidates.length} .did files, over the ${MAX_SOURCES}-source limit`,
    );
  }
  let bundleBytes = 0;
  for (const candidate of candidates) {
    const { size } = await stat(candidate.absolute);
    if (size > MAX_SOURCE_BYTES) {
      resourceFailure(
        "source_bytes",
        MAX_SOURCE_BYTES,
        size,
        `${candidate.relative} is ${size} bytes, over the ${MAX_SOURCE_BYTES}-byte source limit`,
      );
    }
    bundleBytes += size;
    if (bundleBytes > MAX_BUNDLE_BYTES) {
      resourceFailure(
        "bundle_bytes",
        MAX_BUNDLE_BYTES,
        bundleBytes,
        `the bundle directory exceeds the ${MAX_BUNDLE_BYTES}-byte aggregate limit`,
      );
    }
  }
  const files = {};
  for (const candidate of candidates) {
    files[candidate.relative] = await readFile(candidate.absolute, "utf8");
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
