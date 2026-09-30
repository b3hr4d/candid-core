/* Snippet gates: the TypeScript on the site is held to the tree it documents.
 *
 * Zero dependencies of its own: it writes the page snippets to a scratch
 * directory and runs the compiler this repository already pins, from
 * crates/candid-core-ts/ts (`npm ci` there installs it; the site itself takes
 * no dependency). The compiler is asked to resolve `@candid-core/schema` and
 * its three subpaths to the SOURCE in that directory, and `@candid-core/cli`
 * to its hand-written declarations, so a snippet is checked against what the
 * repository builds — exactly the four subpaths the package exports, no more.
 *
 * Every `<pre>` whose language is ts or js is in exactly one of four classes,
 * and a block in none of them fails the gate, so an unverified snippet cannot
 * be added by omission:
 *
 *   data-file="path"       an excerpt of a repository file. Each non-blank line
 *                          run must appear in that file verbatim (whitespace
 *                          trimmed), so a quoted signature cannot drift from
 *                          the source it quotes. Not compiled: it is a
 *                          fragment of something that compiles elsewhere.
 *   data-check             compiled on its own, must have no diagnostics.
 *   data-check="name"      blocks of one page sharing a name are concatenated,
 *                          in page order, into one file: the way a page builds
 *                          an example across several blocks.
 *   data-check-fails="TS2339@7"
 *                          must NOT compile: the compiler must report that code
 *                          at that line of the block (1-based), and nothing
 *                          else anywhere in it. Naming the line is what stops a
 *                          block whose showcased expression was fixed from
 *                          staying "proven stale" on the strength of some other
 *                          line with the same error. A "before" snippet in the
 *                          migration notes is proven stale this way.
 *   data-unchecked="why"   the visible opt-out, with its reason. It is counted
 *                          in the summary line so an exemption cannot hide.
 *
 * A `js` block is written to a `.js` file and compiled with allowJs and
 * checkJs, so TypeScript-only syntax in it (an annotation, `declare`, `as`)
 * is refused as the syntax error a reader copying it would get.
 *
 * A compiled block may import a few stand-in modules of generated code
 * (MODULES below): the page says `./ledger.ts` and the gate supplies the
 * checked-in golden of that name, so the import resolves to real generated
 * output rather than to `any`.
 */

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";

const LANGS = new Set(["ts", "tsx", "js", "javascript", "typescript"]);

/* Stand-in modules a snippet may import, each a checked-in golden. The key is
 * the path a snippet writes, relative to the snippet; the value is the golden
 * under crates/candid-core-ts/tests/goldens. */
const MODULES = {
  "generated/interface.ts": "variants.ts",
  "ledger.ts": "ledger.ts",
  "shadowing.ts": "shadowing.ts",
};

/* Node's `process`, which the snippets use in a scripted `main`. `node:fs`
 * comes from the runtime's own hermetic stub, for the same reason that stub
 * exists: the gate takes no @types/node. */
const AMBIENT = `declare const process: { exit(code?: number): never };
declare module "node:fs" {
  export function readFileSync(path: URL | string, encoding: "utf8"): string;
}
`;

function attributes(text) {
  const found = {};
  for (const match of text.matchAll(/([\w-]+)(?:="([^"]*)")?/g)) {
    found[match[1]] = match[2] === undefined ? true : match[2];
  }
  return found;
}

/** Every ts/js block of a page, in order, with what the gate needs of it. */
export function blocksOf(source) {
  const blocks = [];
  let index = 0;
  for (const match of source.matchAll(/<pre([^>]*)>([\s\S]*?)<\/pre>/g)) {
    index += 1;
    const attrs = attributes(match[1]);
    if (!LANGS.has(attrs["data-lang"])) continue;
    blocks.push({
      number: index,
      attrs,
      code: match[2].replace(/^\n/, "").replace(/\s+$/, "") + "\n",
    });
  }
  return blocks;
}

const normalise = (line) => line.trim().replace(/\s+/g, " ");

/* A signature quoted from a source file drops the opening brace or the
 * trailing semicolon the declaration form would carry. Both sides are reduced
 * the same way, so `opt<T>(inner: Schema<T>): OptSchema<T>` quotes
 * `opt<T>(inner: Schema<T>): OptSchema<T> {` and nothing looser. */
const comparable = (line) => normalise(line).replace(/ \{$/, "").replace(/;$/, "");

/**
 * Is the excerpt a verbatim run of the file? Blank lines are ignored on both
 * sides, and a line that is only `…` or `// …` ends one run and starts the
 * next (an elision the page shows on purpose). Within a run every line must
 * follow the previous one in the file, unless the block is `partial`, which
 * asks only that its lines appear in the file in the same order.
 */
export function excerptProblem(excerpt, fileText, partial = false) {
  const file = fileText.split("\n").map(comparable).filter(Boolean);
  const runs = [[]];
  for (const raw of excerpt.split("\n")) {
    const line = comparable(raw);
    if (!line) continue;
    if (line === "…" || line === "// …" || line === "/* … */") runs.push([]);
    else runs[runs.length - 1].push(line);
  }
  for (const run of runs.filter((r) => r.length)) {
    if (partial) {
      let at = 0;
      for (const line of run) {
        const found = file.indexOf(line, at);
        if (found < 0) return `line not found in order in the file: ${JSON.stringify(line)}`;
        at = found + 1;
      }
      continue;
    }
    // The longest prefix of the run that exists anywhere, to name where it breaks.
    let best = { length: 0 };
    for (let at = 0; at < file.length; at++) {
      let n = 0;
      while (n < run.length && file[at + n] === run[n]) n++;
      if (n === run.length) { best = null; break; }
      if (n > best.length) best = { length: n };
    }
    if (best) {
      return best.length === 0
        ? `first line is not in the file: ${JSON.stringify(run[0])}`
        : `matches the file for ${best.length} line(s), then ${JSON.stringify(run[best.length])} does not follow ` +
            `${JSON.stringify(run[best.length - 1])} there (mark an intended gap with a line \`// …\`, or data-partial)`;
    }
  }
  return null;
}

function tsconfig(root, cli) {
  const ts = join(root, "crates", "candid-core-ts", "ts");
  return {
    compilerOptions: {
      strict: true,
      noEmit: true,
      target: "es2022",
      module: "esnext",
      moduleResolution: "bundler",
      allowImportingTsExtensions: true,
      allowJs: true,
      checkJs: true,
      lib: ["es2022", "dom"],
      types: [],
      skipLibCheck: false,
      paths: {
        "@candid-core/schema": [join(ts, "schema.ts")],
        "@candid-core/schema/validate": [join(ts, "validate.ts")],
        "@candid-core/schema/contract": [join(ts, "contract.ts")],
        "@candid-core/schema/codec": [join(ts, "codec.ts")],
        "@candid-core/cli": [cli],
      },
    },
    include: ["**/*.ts", "**/*.js"],
  };
}

/**
 * Run the snippet gates. `pages` is [{ slug, source }]. Returns
 * { summary, problems: [{ page, message }] }.
 */
export async function checkSnippets({ pages, root, skipCompile = false }) {
  const problems = [];
  const report = (page, message) => problems.push({ page, message });
  const tally = { excerpts: 0, compiled: 0, fails: 0, unchecked: [] };
  const files = []; // { name, code, expect, page, origin }

  for (const { slug, source } of pages) {
    const page = `content/${slug}.html`;
    const groups = new Map();
    for (const block of blocksOf(source)) {
      const { attrs } = block;
      const where = `ts/js block #${block.number}`;
      const isExcerpt = attrs["data-file"] !== undefined;
      const ext = ["js", "javascript"].includes(attrs["data-lang"]) ? "js" : "ts";
      if (attrs["data-unchecked"] !== undefined) {
        // The visible opt-out wins over everything: a block that carries a
        // data-file beside it is displayed as that file's excerpt, unverified.
        if (attrs["data-unchecked"] === true || String(attrs["data-unchecked"]).trim().length < 12) {
          report(page, `${where}: data-unchecked needs a reason of at least a few words`);
        } else {
          tally.unchecked.push(`${slug}#${block.number}`);
        }
        continue;
      }
      const classes = ["data-file", "data-check", "data-check-fails"].filter((name) => attrs[name] !== undefined);
      if (classes.length === 0) {
        report(page, `${where} is not verified: add data-file, data-check, data-check-fails or data-unchecked="reason"`);
        continue;
      }
      // `data-file` may sit beside `data-check` (an excerpt that also compiles)
      // but not beside an expected failure.
      if (attrs["data-check-fails"] !== undefined && classes.length > 1) {
        report(page, `${where} carries conflicting verification attributes: ${classes.join(", ")}`);
        continue;
      }
      if (isExcerpt) {
        const file = join(root, attrs["data-file"]);
        if (!existsSync(file)) {
          report(page, `${where}: data-file ${JSON.stringify(attrs["data-file"])} does not exist`);
        } else {
          const problem = excerptProblem(block.code, await readFile(file, "utf8"), attrs["data-partial"] !== undefined);
          if (problem) report(page, `${where} is not an excerpt of ${attrs["data-file"]}: ${problem}`);
          else tally.excerpts += 1;
        }
        if (attrs["data-check"] === undefined) continue;
      }
      if (attrs["data-check-fails"] !== undefined) {
        const named = String(attrs["data-check-fails"]).match(/^(TS\d{4,5})@(\d+)$/);
        if (!named) {
          report(page, `${where}: data-check-fails must name the code and the line it fails at, such as TS2322@3`);
          continue;
        }
        files.push({ name: `${slug}-${block.number}.${ext}`, code: block.code, expect: { code: named[1], line: Number(named[2]) }, page, origin: where });
        continue;
      }
      const group = attrs["data-check"];
      if (typeof group === "string" && group.length) {
        const key = `${slug}-${group}.${ext}`;
        if (!groups.has(key)) groups.set(key, { name: key, code: "", expect: null, page, origin: `${where} (group ${group})`, parts: [] });
        const entry = groups.get(key);
        if (groups.has(`${slug}-${group}.${ext === "js" ? "ts" : "js"}`)) {
          report(page, `${where}: group ${group} mixes js and ts blocks; a group is one file`);
        }
        entry.parts.push({ start: entry.code.split("\n").length, number: block.number });
        entry.code += block.code + "\n";
      } else {
        files.push({ name: `${slug}-${block.number}.${ext}`, code: block.code, expect: null, page, origin: where });
      }
    }
    files.push(...groups.values());
  }

  if (skipCompile || files.length === 0) {
    return { tally, problems, compiled: false };
  }

  const tsc = join(root, "crates", "candid-core-ts", "ts", "node_modules", ".bin", "tsc");
  if (!existsSync(tsc)) {
    report(
      "snippets",
      `the pinned TypeScript compiler is not installed (${relative(root, tsc)}); run \`npm ci\` in crates/candid-core-ts/ts, ` +
        "or pass --skip-snippets to check everything else while drafting (CI never does)",
    );
    return { tally, problems, compiled: false };
  }

  const scratch = await mkdtemp(join(tmpdir(), "candid-core-snippets-"));
  try {
    const cli = join(root, "crates", "candid-core-wasm", "npm", "lib", "index.d.ts");
    await writeFile(join(scratch, "tsconfig.json"), JSON.stringify(tsconfig(root, cli), null, 2));
    await writeFile(join(scratch, "ambient.d.ts"), AMBIENT);
    for (const [name, golden] of Object.entries(MODULES)) {
      await mkdir(dirname(join(scratch, name)), { recursive: true });
      await writeFile(join(scratch, name), await readFile(join(root, "crates", "candid-core-ts", "tests", "goldens", golden), "utf8"));
    }
    for (const file of files) {
      // A block that is a fragment of a script (`return` at top level, a bare
      // `await`) is wrapped by the page, not here: the gate compiles what is
      // written. Every file is a module so a snippet's own `import` lines work
      // and its bindings do not collide with another snippet's.
      await writeFile(join(scratch, file.name), file.code.includes("import ") || file.code.includes("export ") ? file.code : `export {};\n${file.code}`);
    }

    const run = spawnSync(tsc, ["-p", join(scratch, "tsconfig.json"), "--pretty", "false"], {
      cwd: scratch,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
    if (run.error) {
      report("snippets", `could not run the compiler: ${run.error.message}`);
      return { tally, problems, compiled: false };
    }
    const output = `${run.stdout}${run.stderr}`;
    const diagnostics = [...output.matchAll(/^(.+?)\((\d+),(\d+)\): error (TS\d+): (.*)$/gm)].map((m) => ({
      file: m[1],
      line: Number(m[2]),
      code: m[4],
      message: m[5],
    }));
    const unattributed = output
      .split("\n")
      .filter((line) => /error TS\d+/.test(line) && !/^.+?\(\d+,\d+\): error TS\d+: /.test(line));
    for (const line of unattributed) report("snippets", `compiler: ${line.trim()}`);

    for (const file of files) {
      const mine = diagnostics.filter((d) => d.file === file.name || d.file.endsWith(`/${file.name}`));
      const prefixed = !(file.code.includes("import ") || file.code.includes("export "));
      const lineOf = (d) => (prefixed ? d.line - 1 : d.line);
      if (file.expect) {
        const want = file.expect;
        const seen = mine.map((d) => `${d.code}@${lineOf(d)}`);
        const stray = mine.filter((d) => d.code !== want.code || lineOf(d) !== want.line);
        if (mine.length === 0) {
          report(file.page, `${file.origin} must NOT compile (${want.code}@${want.line}) but compiles cleanly`);
        } else if (stray.length) {
          report(
            file.page,
            `${file.origin} must fail with ${want.code}@${want.line} and nothing else, but the compiler reports ${[...new Set(seen)].join(", ")}`,
          );
        } else tally.fails += 1;
      } else if (mine.length) {
        for (const d of mine.slice(0, 5)) {
          const lines = file.code.split("\n");
          const at = lineOf(d);
          report(file.page, `${file.origin} does not compile: ${d.code} ${d.message} — line ${at}: ${JSON.stringify((lines[at - 1] || "").trim())}`);
        }
      } else tally.compiled += 1;
    }
    const known = new Set(files.map((f) => f.name));
    for (const d of diagnostics) {
      const base = d.file.split("/").pop();
      if (!known.has(base) && !base.endsWith("ambient.d.ts") && !d.file.includes("/crates/")) {
        report("snippets", `compiler: ${d.file}(${d.line}): ${d.code} ${d.message}`);
      } else if (d.file.includes("/crates/")) {
        report("snippets", `compiler, in the tree: ${d.file}(${d.line}): ${d.code} ${d.message}`);
      }
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
  return { tally, problems, compiled: true };
}
