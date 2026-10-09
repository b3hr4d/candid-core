#!/usr/bin/env node
// The @candid-core/cli entry point:
//
//   candid-core-cli gen <service.did>... [-o <dir>] [--json] [--check]
//   candid-core-cli project <in.did> --methods <a,b,...> -o <out.did> [--json]
//   candid-core-cli check <written.did> --against <live.did> [--json]
//
// `project` writes a `.did` holding only the named methods and the types
// they reach; `check` exits 0 when the live interface is still a Candid
// subtype of the written one and 1 otherwise. Both read their inputs as
// `gen` does (the file and every `.did` beneath its directory) and never
// touch the network: fetching a live interface is the caller's job.
//
// The JS host does all the I/O: for each entry it reads the entry file and
// every `.did` beneath the entry's directory, hands them to the wasm
// compiler as data, and writes the two artifacts — `<stem>.ts` (the
// generated module) and `<stem>.envelope.json` (the one-document contract
// envelope with field names) — printing the content-addressed identities on
// stdout. Nothing is read from stdin, ever, and nothing is written outside
// `-o`.
//
// Conventions follow the native `candid-core` binary: anything outside the
// grammar is a usage error (exit 64, usage on stderr, nothing on stdout);
// any entry that fails (a compile or generation failure, an unreadable
// entry, a determinism mismatch) makes the run exit 1 while the other
// entries still run. Determinism is enforced, not assumed: every generation
// runs twice and the entry refuses on any byte mismatch.
//
// A module that had to leave declarations or actor methods out is still
// usable, so the run succeeds (exit 0): each omission is printed as
// a `warning: omitted …` line on stderr, in the order and wording of the
// module's own `// Omitted:` header.
//
// `--json` replaces every human line with exactly one JSON document on
// stdout (shape: README, `CliReport` in lib/index.d.ts); stderr stays empty.
// `--check` generates in memory, compares byte-for-byte with the files on
// disk, writes nothing, and exits 1 on any drift.

import { readFile, readdir, readlink, mkdir, realpath, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

import {
  checkCompatible,
  didToContract,
  didToModule,
  init,
  projectDid,
} from "../lib/index.js";

const USAGE = [
  "usage: candid-core-cli gen <service.did>... [-o <dir>] [--json] [--check]",
  "       candid-core-cli project <in.did> --methods <a,b,...> -o <out.did> [--json]",
  "       candid-core-cli check <written.did> --against <live.did> [--json]",
].join("\n");

// The `--json` document's version. It changes only when an existing field's
// meaning or shape changes; a consumer refuses a version it does not know.
const SCHEMA_VERSION = 1;

// The compiler's own source bounds (`Limits::default()` — the values the
// root README documents), enforced *while walking*: the entry's whole
// directory tree is the bundle this CLI hands over, so an oversized tree
// must fail with the structured resource diagnostic the compiler would
// produce, never by exhausting the JS heap before the wasm side can check.
const MAX_SOURCE_BYTES = 1_048_576; // max_source_bytes, per file
const MAX_BUNDLE_BYTES = 8_388_608; // max_bundle_bytes, aggregate
const MAX_SOURCES = 256; // max_sources, file count

function usage(problem) {
  if (problem !== undefined) {
    console.error(problem);
  }
  console.error(USAGE);
  process.exit(64);
}

/** The output stem an entry generates, e.g. `service` for `a/service.did`. */
function stemOf(entry) {
  return path.basename(entry).replace(/\.did$/, "");
}

function parseArguments(argv) {
  if (argv[0] === "project") {
    return parseProject(argv);
  }
  if (argv[0] === "check") {
    return parseCheck(argv);
  }
  if (argv.length === 0 || argv[0] !== "gen") {
    usage();
  }
  const entries = [];
  let outDir;
  let json = false;
  let check = false;
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "-o") {
      if (outDir !== undefined || index + 1 >= argv.length) {
        usage();
      }
      index += 1;
      outDir = argv[index];
    } else if (argument === "--json") {
      json = true;
    } else if (argument === "--check") {
      check = true;
    } else if (argument.startsWith("-")) {
      usage();
    } else {
      entries.push(argument);
    }
  }
  if (entries.length === 0) {
    usage();
  }
  // Decision D2 (a recommendation the maintainer can overturn): every entry
  // writes `<stem>.ts` into the one `-o`, so two entries with one stem would
  // overwrite each other. Refuse, as a usage error, before any work. Stems
  // compare case-insensitively: the output must check out the same on a
  // case-folding filesystem.
  const seen = new Map();
  for (const entry of entries) {
    const key = stemOf(entry).toLowerCase();
    if (seen.has(key)) {
      usage(
        `error: ${seen.get(key)} and ${entry} would both write ${stemOf(entry)}.ts; ` +
          "give each entry a distinct file name",
      );
    }
    seen.set(key, entry);
  }
  return { command: "gen", entries, outDir: outDir ?? ".", json, check };
}

/**
 * Read a command's options: each `--name` in `valued` takes the next
 * argument, at most once; `--json` is a flag; anything else starting with
 * `-` is a usage error, and the rest are positionals.
 */
function parseOptions(argv, valued) {
  const options = {};
  const positionals = [];
  let json = false;
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (valued.includes(argument)) {
      if (options[argument] !== undefined || index + 1 >= argv.length) {
        usage();
      }
      index += 1;
      options[argument] = argv[index];
    } else if (argument === "--json") {
      json = true;
    } else if (argument.startsWith("-")) {
      usage();
    } else {
      positionals.push(argument);
    }
  }
  return { options, positionals, json };
}

function parseProject(argv) {
  const { options, positionals, json } = parseOptions(argv, ["--methods", "-o"]);
  if (positionals.length !== 1 || options["--methods"] === undefined || options["-o"] === undefined) {
    usage();
  }
  const entry = positionals[0];
  const output = options["-o"];
  // `--methods ""` is the empty list, which the library refuses with a
  // diagnostic; any other value is split on commas, verbatim.
  const methods = options["--methods"] === "" ? [] : options["--methods"].split(",");
  return { command: "project", entry, output, methods, json };
}

function parseCheck(argv) {
  const { options, positionals, json } = parseOptions(argv, ["--against"]);
  if (positionals.length !== 1 || options["--against"] === undefined) {
    usage();
  }
  return { command: "check", written: positionals[0], live: options["--against"], json };
}

/** A diagnostic in the compiler's own item shape. */
function diagnostic(code, phase, message, extra = {}) {
  return { code, phase, severity: "error", message, ...extra };
}

/**
 * One entry's failure. `diagnostics` is what `--json` reports. Without
 * `--json` it prints as the `{ ok: false, diagnostics }` document on stdout
 * — the native binary's convention — unless `plain` is set, in which case
 * the failure has no document and `plain` is the stderr line it always had.
 */
class EntryFailure extends Error {
  constructor(diagnostics, plain) {
    super(diagnostics[0].message);
    this.diagnostics = diagnostics;
    this.plain = plain;
  }
}

function resourceFailure(resource, limit, observed, message) {
  return new EntryFailure([
    diagnostic("resource_limit_exceeded", "load", message, {
      resource_limit: { resource, limit, observed },
    }),
  ]);
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
    throw resourceFailure(
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
      throw resourceFailure(
        "source_bytes",
        MAX_SOURCE_BYTES,
        size,
        `${candidate.relative} is ${size} bytes, over the ${MAX_SOURCE_BYTES}-byte source limit`,
      );
    }
    bundleBytes += size;
    if (bundleBytes > MAX_BUNDLE_BYTES) {
      throw resourceFailure(
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

// Decision D1 (a recommendation the maintainer can overturn): the bundle
// root of an entry is the entry's own directory, and the 256-file / 1 MiB /
// 8 MiB bounds apply to each entry's bundle on its own. Entries that share a
// directory share one walk here — the tree is read and counted once, not
// once per entry — so they never double-count against each other.
//
// Every path this CLI reports is spelled as the user passed it (an entry, its
// directory, `-o` joined with a file name), never `path.resolve`d: the same
// invocation must print the same bytes from any working directory. So the
// walk reads through the directory as given, and only the memo key resolves.
const bundles = new Map();
function bundleOf(directory) {
  const key = path.resolve(directory);
  if (!bundles.has(key)) {
    bundles.set(key, didFiles(directory));
  }
  return bundles.get(key);
}

/**
 * The sources request for one `.did` file: the file and every `.did`
 * beneath its directory, as `gen` reads an entry. Throws `EntryFailure`.
 */
async function sourcesFor(file) {
  const directory = path.dirname(file);
  const name = path.basename(file);
  let files;
  try {
    files = await bundleOf(directory);
  } catch (error) {
    if (error instanceof EntryFailure) {
      throw error;
    }
    const message = `cannot read ${directory}: ${error.message}`;
    throw new EntryFailure([diagnostic("did_file_read_error", "load", message)], message);
  }
  if (files[name] === undefined) {
    const message = `cannot read ${file}: no such .did file`;
    throw new EntryFailure([diagnostic("did_source_not_found", "load", message)], message);
  }
  return { entry: name, files };
}

/**
 * Where a write to `file` lands, with every symlink resolved: the file's
 * real path when it exists; else a dangling symlink's target, followed; else
 * the deepest existing ancestor's real path joined with the rest. On a
 * case-insensitive file system the real path is the on-disk spelling.
 */
async function landing(file) {
  let current = path.resolve(file);
  for (let hops = 0; hops < 40; hops += 1) {
    try {
      return await realpath(current);
    } catch {
      // Not there: a dangling symlink, or a file not written yet.
    }
    let target;
    try {
      target = await readlink(current);
    } catch {
      break;
    }
    current = path.resolve(path.dirname(current), target);
  }
  const rest = [];
  let directory = current;
  while (directory !== path.dirname(directory)) {
    rest.unshift(path.basename(directory));
    directory = path.dirname(directory);
    try {
      return path.join(await realpath(directory), ...rest);
    } catch {
      // Keep climbing to an ancestor that exists.
    }
  }
  return current;
}

/**
 * What names a file on disk, as far as it can be told without writing:
 * the lexical absolute path, where a write to it lands (`landing`), and the
 * device and inode when the file exists — which is what catches a hard link,
 * and a different spelling on a case-insensitive file system.
 */
async function fileIdentity(file) {
  const resolved = path.resolve(file);
  let inode;
  try {
    const found = await stat(file, { bigint: true });
    // A file system without inode numbers reports 0 for every file, which
    // would make every file the same one.
    if (found.ino !== 0n) {
      inode = `${found.dev}:${found.ino}`;
    }
  } catch {
    // Not there (or not statable): only the path forms below can match.
  }
  let real;
  try {
    real = await landing(file);
  } catch {
    real = undefined;
  }
  return { resolved, real, inode };
}

function sameFile(a, b) {
  return (
    a.resolved === b.resolved ||
    (a.real !== undefined && a.real === b.real) ||
    (a.inode !== undefined && a.inode === b.inode)
  );
}

/** Whether `inner` is strictly beneath `outer`; both absolute. */
function beneath(outer, inner) {
  const relative = path.relative(outer, inner);
  return (
    relative !== "" && !path.isAbsolute(relative) && relative.split(path.sep)[0] !== ".."
  );
}

/**
 * Refuse an output that is, or would become, a source of the input bundle,
 * under any name. `sourcesFor` hands the compiler the entry and every `.did`
 * beneath the entry's directory, so writing one of those files would replace
 * a file the projection was cut from, and writing a new `.did` beneath that
 * directory would make the projection a source of every later run. Throws
 * `EntryFailure` (`output_is_input`), whose `notes` list the existing
 * sources the output names — empty for a file not written yet.
 */
async function refuseOutputInBundle(output, directory, files) {
  const target = await fileIdentity(output);
  const sources = [];
  for (const relative of Object.keys(files).sort()) {
    const source = path.join(directory, ...relative.split("/"));
    if (sameFile(target, await fileIdentity(source))) {
      sources.push(source);
    }
  }
  let message;
  if (sources.length > 0) {
    message = `-o ${output} is ${sources.join(", ")}, a source of the input`;
  } else if (
    target.real !== undefined &&
    target.real.endsWith(".did") &&
    beneath(await realpath(directory), target.real)
  ) {
    message = `-o ${output} is a .did beneath ${directory}, so it would be a source of the input`;
  }
  if (message !== undefined) {
    throw new EntryFailure([
      diagnostic(
        "output_is_input",
        "write",
        `${message}; write the projection outside ${directory}`,
        { path: output, notes: sources },
      ),
    ]);
  }
}

/**
 * An output that could not be read or written, as a failure: the path as
 * given, and the system error code (such as `EISDIR` or `EACCES`) in `notes`.
 */
function writeFailure(output, error) {
  const message = `cannot write ${output}: ${error.message}`;
  const notes = typeof error.code === "string" ? { notes: [error.code] } : {};
  return new EntryFailure(
    [diagnostic("output_write_failed", "write", message, { path: output, ...notes })],
    message,
  );
}

/** An unexpected error as the one failure shape. */
function asFailure(error) {
  return error instanceof EntryFailure
    ? error
    : new EntryFailure([
        { code: "internal_error", severity: "error", message: String(error.message ?? error) },
      ]);
}

/**
 * Print a failure the way `gen` does: the `{ ok: false, diagnostics }`
 * document on stdout, or the plain stderr line of a read error; with
 * `--json`, always the document.
 */
function printFailure(failure, json, extra = {}) {
  if (failure.plain !== undefined && !json) {
    console.error(failure.plain);
  } else {
    console.log(JSON.stringify({ ok: false, ...extra, diagnostics: failure.diagnostics }, null, 2));
  }
}

/** `candid-core-cli project`. Returns the exit code. */
async function runProject({ entry, output, methods, json }) {
  let result;
  try {
    const sources = await sourcesFor(entry);
    await refuseOutputInBundle(output, path.dirname(entry), sources.files);
    result = await deterministic("projection", () => projectDid(sources, methods));
  } catch (error) {
    printFailure(asFailure(error), json);
    return 1;
  }
  if (!result.ok) {
    console.log(JSON.stringify(result, null, 2));
    return 1;
  }
  const bytes = Buffer.from(result.did);
  let status;
  try {
    const existing = await onDisk(output);
    status = existing !== null && existing.equals(bytes) ? "unchanged" : "written";
    if (status === "written") {
      await mkdir(path.dirname(output), { recursive: true });
      await writeFile(output, bytes);
    }
  } catch (error) {
    printFailure(writeFailure(output, error), json);
    return 1;
  }
  if (json) {
    const { ok, methods: kept, input, projection } = result;
    console.log(JSON.stringify({ ok, output, status, methods: kept, input, projection }, null, 2));
  } else {
    console.log(`input:      ${result.input.interface_id}`);
    console.log(`projection: ${result.projection.interface_id}`);
    console.log(`${status === "written" ? "wrote" : "unchanged"} ${output}`);
  }
  return 0;
}

/** `candid-core-cli check`. Returns the exit code. */
async function runCheck({ written, live, json }) {
  let report;
  try {
    const sides = [await sourcesFor(written), await sourcesFor(live)];
    report = await deterministic("check", () => checkCompatible(...sides));
  } catch (error) {
    printFailure(asFailure(error), json);
    return 1;
  }
  if (json || !report.ok) {
    console.log(JSON.stringify(report, null, 2));
    return report.ok && report.compatible ? 0 : 1;
  }
  console.log(`written: ${report.written.interface_id}`);
  console.log(`live:    ${report.live.interface_id}`);
  let errors = 0;
  let warnings = 0;
  for (const item of report.diagnostics) {
    const at = item.path === undefined ? "" : ` at ${item.path}`;
    console.error(`${item.severity}: ${listedName(item.method)}: ${item.code}${at}: ${item.message}`);
    if (item.severity === "error") {
      errors += 1;
    } else {
      warnings += 1;
    }
  }
  console.log(
    `${report.compatible ? "compatible" : "incompatible"}: ${errors} error(s), ${warnings} warning(s)`,
  );
  return report.compatible ? 0 : 1;
}

/**
 * A name as the module header lists it, character for character: bare when
 * identifier-shaped, else quoted exactly as the generator quotes it — `"`,
 * `\\`, `\n`, `\r` and `\t` escaped, every other control character as a
 * lowercase `\u00XX`, and U+2028 / U+2029 as `\u2028` / `\u2029`.
 * `JSON.stringify` is not that: it leaves U+2028 and U+2029 raw — both end
 * a line — and writes `\b` and `\f`. A quoted Candid method name can hold
 * any of them, and each warning must stay one line equal to its header line.
 */
function listedName(name) {
  if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name)) {
    return name;
  }
  let quoted = '"';
  for (const character of name) {
    const code = character.codePointAt(0);
    if (character === '"') {
      quoted += '\\"';
    } else if (character === "\\") {
      quoted += "\\\\";
    } else if (character === "\n") {
      quoted += "\\n";
    } else if (character === "\r") {
      quoted += "\\r";
    } else if (character === "\t") {
      quoted += "\\t";
    } else if (code < 0x20 || code === 0x2028 || code === 0x2029) {
      quoted += `\\u${code.toString(16).padStart(4, "0")}`;
    } else {
      quoted += character;
    }
  }
  return `${quoted}"`;
}

/** One omission, worded as the module header's `// Omitted:` line. */
function describeOmission(entry) {
  const kind = entry.kind === "method" ? "method" : "type";
  const via = entry.via === undefined ? "" : ` via ${listedName(entry.via)}`;
  return `${kind} ${listedName(entry.name)} (${entry.reason}${via})`;
}

/** Run a generation twice and refuse on any byte mismatch. */
async function deterministic(label, produce) {
  const first = await produce();
  const second = await produce();
  if (JSON.stringify(first) !== JSON.stringify(second)) {
    const message = `determinism check failed: two ${label} runs disagreed; refusing to write`;
    throw new EntryFailure([diagnostic("nondeterministic_output", "generate", message)], message);
  }
  return first;
}

/** The file's bytes, or `null` when it does not exist. */
async function onDisk(file) {
  try {
    return await readFile(file);
  } catch (error) {
    if (error.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

/** Generate one entry and write it, or (with `check`) compare it. Never throws. */
async function processEntry(entry, { outDir, check }) {
  const stem = stemOf(entry);
  const modulePath = path.join(outDir, `${stem}.ts`);
  const envelopePath = path.join(outDir, `${stem}.envelope.json`);
  const report = {
    entry,
    status: "failed",
    module: modulePath,
    envelope: envelopePath,
    omitted: [],
    diagnostics: [],
  };
  // What the human path prints for this entry, in order; `--json` drops it.
  const human = { out: [], err: [] };
  const drift = [];
  try {
    // The directory and file name as given: `dirname("a.did")` is ".".
    const sources = await sourcesFor(entry);
    const envelope = await deterministic("contract", () => didToContract(sources));
    if (!("contract" in envelope)) {
      throw new EntryFailure(envelope.diagnostics);
    }
    const generated = await deterministic("module", () => didToModule(sources));
    if (!generated.ok) {
      throw new EntryFailure(generated.diagnostics);
    }

    const artifacts = [
      [modulePath, Buffer.from(generated.module)],
      [envelopePath, Buffer.from(`${JSON.stringify(envelope, null, 2)}\n`)],
    ];
    let wrote = false;
    for (const [file, bytes] of artifacts) {
      const existing = await onDisk(file);
      if (existing !== null && existing.equals(bytes)) {
        human.out.push(`unchanged ${file}`);
      } else if (check) {
        drift.push(file);
        human.err.push(`${existing === null ? "missing" : "drifted"} ${file}`);
      } else {
        await mkdir(outDir, { recursive: true });
        await writeFile(file, bytes);
        wrote = true;
        human.out.push(`wrote ${file}`);
      }
    }
    if (!check) {
      const identities = envelope.contract.identities ?? {};
      const lines = [];
      if (identities.contract !== undefined) {
        lines.push(`contract:  ${identities.contract}`);
      }
      if (identities.interface !== undefined) {
        lines.push(`interface: ${identities.interface}`);
      }
      human.out.unshift(...lines);
    }
    for (const omission of generated.omitted) {
      human.err.push(`warning: omitted ${describeOmission(omission)}`);
    }
    report.omitted = generated.omitted;
    report.status = drift.length > 0 ? "drifted" : wrote ? "written" : "unchanged";
  } catch (error) {
    const failure = asFailure(error);
    report.diagnostics = failure.diagnostics;
    human.out.length = 0;
    human.err.length = 0;
    drift.length = 0;
    if (failure.plain !== undefined) {
      human.err.push(failure.plain);
    } else {
      human.out.push(JSON.stringify({ ok: false, diagnostics: failure.diagnostics }, null, 2));
    }
  }
  return { report, human, drift };
}

const parsed = parseArguments(process.argv.slice(2));

await init();

if (parsed.command === "project") {
  process.exitCode = await runProject(parsed);
} else if (parsed.command === "check") {
  process.exitCode = await runCheck(parsed);
} else {
  process.exitCode = await runGen(parsed);
}

/** `candid-core-cli gen`. Returns the exit code. */
async function runGen({ entries, outDir, json, check }) {
  const reports = [];
  const drift = [];
  for (const entry of entries) {
    const result = await processEntry(entry, { outDir, check });
    reports.push(result.report);
    drift.push(...result.drift);
    if (!json) {
      for (const line of result.human.out) {
        console.log(line);
      }
      for (const line of result.human.err) {
        console.error(line);
      }
      if (result.report.status === "failed" && result.human.err.length === 0 && entries.length > 1) {
        console.error(`error: ${entry}: failed; its diagnostics are on stdout`);
      }
    }
  }

  const ok = reports.every((report) => report.status !== "failed" && report.status !== "drifted");
  if (json) {
    const document = { schemaVersion: SCHEMA_VERSION, ok, check, entries: reports, drift };
    console.log(JSON.stringify(document, null, 2));
  } else if (drift.length > 0) {
    console.error(
      `${drift.length} file(s) differ from what the current sources generate; ` +
        "run without --check to regenerate",
    );
  }
  // Not `process.exit`: on some platforms a pipe drains asynchronously, and the
  // document must reach the reader whole.
  return ok ? 0 : 1;
}
