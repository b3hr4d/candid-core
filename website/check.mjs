#!/usr/bin/env node
/* Documentation gates.
 *
 * Zero dependencies. Run after build.mjs:
 *
 *   node website/check.mjs
 *
 * Everything here failed in a draft at least once, which is the bar for a
 * check earning its place. The checks are deliberately mechanical: they catch
 * the class of mistake a writer cannot see by re-reading their own page.
 */

import { readFile, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkSnippets } from "./snippets.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CONTENT = join(HERE, "content");
const DIST = join(HERE, "dist");

const failures = [];
const fail = (page, message) => failures.push({ page, message });

const site = JSON.parse(await readFile(join(CONTENT, "_site.json"), "utf8"));

/* The crate version the pages must agree with, read from the manifest rather
 * than repeated here, so a release makes every stale dependency line in the
 * documentation fail on the next run instead of quietly lying. */
const manifest = await readFile(join(HERE, "..", "Cargo.toml"), "utf8");
const CRATE_VERSION = (manifest.match(/^version\s*=\s*"([^"]+)"/m) || [])[1];
if (!CRATE_VERSION) throw new Error("could not read the crate version from Cargo.toml");
const IS_PRERELEASE = CRATE_VERSION.includes("-");
const REQUIRED_REQ = IS_PRERELEASE ? `=${CRATE_VERSION}` : CRATE_VERSION;
const slugs = new Set(site.sections.flatMap((section) => section.pages.map((page) => page.slug)));

/* Commands and dependency lines that must never appear in a CODE BLOCK.
 *
 * These are checked against <pre> content only, not prose. A page is expected
 * to *discuss* the broken spellings — explaining that a caret requirement
 * selects nothing is exactly the right thing for a page to do. What must never
 * appear is a copyable line that does not work.
 */
/* npm names this repository has prepared but not published.
 *
 * A list rather than a hardcoded pattern, for the same reason CRATE_VERSION is
 * read out of Cargo.toml above: publishing a name should be one edit in one
 * place, not a hunt for regexes. Emptying this list is what a first publish
 * does here; adding to it is what preparing the next package does.
 *
 * Both spellings a reader can copy are refused: the install line and the bare
 * import specifier. The import form matters as much — a `<pre data-lang="js">`
 * block importing an unpublished name is exactly as broken as an `npm i` line
 * for it, and is the form this page had while the earlier pattern only looked
 * for installs.
 */
// Emptied when `@candid-core/cli` was first published. The rules above stay,
// so the next prepared name is one array entry away from being enforced.
const UNPUBLISHED_NPM = [];

const escapeRe = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const unpublishedRules = UNPUBLISHED_NPM.flatMap((name) => [
  {
    pattern: new RegExp(
      `(npm\\s+(i|install|exec)|npx|bunx|pnpm\\s+(add|dlx)|yarn\\s+add)\\s+(-\\S+\\s+)*${escapeRe(name)}\\b`,
    ),
    message: `install/npx line for ${name}, which is not published on npm`,
  },
  {
    /* Both spellings that reach a module specifier: `import x from "n"` and
     * the side-effect form `import "n"`, which carries no `from` at all. The
     * optional parenthesis also catches a dynamic `import("n")`. */
    pattern: new RegExp(
      `(from|import)\\s*\\(?\\s*["']${escapeRe(name)}(/[^"']*)?["']`,
    ),
    message: `import of ${name}, which is not published on npm`,
  },
]);

/* Versions and dist-tags of published names that the registry does not hold
 * yet: a release prepared on `main` before its dispatch. The name resolves, so
 * the rules above cannot see the problem, but `@candid-core/schema@beta`
 * resolves to nothing until a beta is published under that tag, and an exact
 * prepared version is a 404. The same two spellings are refused, in code
 * blocks only, so a page can still say in prose what the line will be. The
 * publish-day change empties this list and turns that prose into blocks. */
// Emptied when @candid-core/schema 0.3.0-beta.1 and @candid-core/cli
// 0.2.0-beta.1 were published under `beta`. The next beta's bump adds its
// versions back until its own publish.
const UNPUBLISHED_NPM_SPECS = [];

const unpublishedSpecRules = UNPUBLISHED_NPM_SPECS.map((spec) => ({
  /* The spec followed by anything that cannot continue a version or a tag, so
   * `@beta` does not also refuse a later `@beta-2` tag by prefix. */
  pattern: new RegExp(`${escapeRe(spec)}(?![\\w.+-])`),
  message: `${spec}, which the registry does not hold yet`,
}));

const FORBIDDEN_IN_CODE = [
  ...unpublishedRules,
  ...unpublishedSpecRules,
  {
    pattern: /^\s*cargo\s+add\s+candid-core\s*$/m,
    message: "bare `cargo add candid-core`, which cannot resolve a prerelease",
  },
];

/* Claims that must never appear in PROSE, wherever they appear. */
const FORBIDDEN_IN_PROSE = [
  {
    // The project forbids performance claims outright (issue #39).
    pattern:
      /\b(blazing|blazingly|lightning[- ]fast|zero[- ]cost|high[- ]performance|ultra[- ]fast)\b/i,
    message: "performance claim; this project does not make them",
  },
  {
    pattern: /\b(seamless(ly)?|effortless(ly)?|magical|supercharge[ds]?|game[- ]chang\w+)\b/i,
    message: "marketing filler",
  },
];

/* Words that are usually filler. Reported as warnings, not failures. */
const SOFT = [
  /\bsimply\b/i,
  /\bjust\s+(run|call|use|add|write)\b/i,
  /\bpowerful\b/i,
  /\bleverage\b/i,
  /\brobust\b/i,
];

/* The surface the site teaches is the one the repository builds. These names
 * belong to the surface `latest` (0.2.0) still has and the 0.3 betas
 * do not, so they may be named only where the change is the subject: the
 * migration page, and the release history on the status page. Anywhere else
 * they would be prose or a snippet that teaches something that no longer
 * exists. The check reads prose and code alike, because a page that shows
 * `createActor` in a block teaches it as surely as one that says so. */
const REMOVED_SURFACE = [
  /\bcreateActor\b/,
  /\bcallFunc\b/,
  /\bhttpTransport\b/,
  /\bHttpTransportOptions\b/,
  /\bActorError\b/,
  /\bPrincipalValue\b/,
  /\bDecodedPrincipal\b/,
  /\bformModel\b/,
  /\bformNodeAt\b/,
  /\bunrepresentable_option\b/,
  /\bUnrepresentableOption\b/,
  /@candid-core\/schema\/(actor|transport-icp|forms|labels)\b/,
  /\.\/transport-icp\b/,
  /\.\/actor\b/,
  /\.\/forms\b/,
  /\.\/labels\b/,
];
const REMOVED_SURFACE_PAGES = new Set(["migrating-from-0-2", "status"]);

/* Pages that describe the schema runtime or the generator carry the one note
 * saying they describe the beta, that `latest` (0.2.0 / 0.1.0) differs, and
 * where to read what changed. */
const NOTE_TITLE = "Published as a beta";
const NOTE_RE = new RegExp(`<callout[^>]*title="${NOTE_TITLE}"`, "g");
const NOTE_LINK_RE = new RegExp(
  `<callout[^>]*title="${NOTE_TITLE}"[\\s\\S]*?href="migrating-from-0-2\\.html"[\\s\\S]*?<\\/callout>`,
);
const NOTE_PAGES = new Set([
  "index",
  "quickstart-typescript",
  "packages",
  "ts-overview",
  "schema-builders",
  "schema-validation",
  "schema-codec",
  "schema-from-contract",
  "generator",
  "cli-npm",
  "reference-mapping",
  "reference-diagnostics",
  "status",
  "migrating-from-0-2",
]);

const warnings = [];
const anchorLinks = [];
const snippetPages = [];

for (const name of (await readdir(CONTENT)).filter((f) => f.endsWith(".html"))) {
  const page = `content/${name}`;
  const source = await readFile(join(CONTENT, name), "utf8");
  const isLanding = name === "index.html";

  /* --- document shape --- */
  if (/<!doctype|<html[\s>]|<body[\s>]|<head[\s>]/i.test(source)) {
    fail(page, "content files are body fragments; they must not contain a document shell");
  }
  if (!isLanding && /<h1[\s>]/i.test(source)) {
    fail(page, "the generator supplies <h1> from _site.json; remove it");
  }
  if (!slugs.has(name.replace(/\.html$/, ""))) {
    fail(page, "file is not listed in _site.json, so it is never built");
  }

  /* --- code blocks --- */
  for (const match of source.matchAll(/<pre([^>]*)>/g)) {
    if (!/data-lang="/.test(match[1])) fail(page, "a <pre> is missing data-lang");
  }
  const LANGS = new Set([
    "rust",
    "rs",
    "ts",
    "tsx",
    "js",
    "javascript",
    "typescript",
    "json",
    "bash",
    "sh",
    "shell",
    "console",
    "candid",
    "did",
    "toml",
    "text",
    "txt",
  ]);
  for (const match of source.matchAll(/<pre[^>]*data-lang="([^"]*)"/g)) {
    if (!LANGS.has(match[1])) fail(page, `unknown data-lang "${match[1]}"`);
  }

  /* --- callouts --- */
  for (const match of source.matchAll(/<callout([^>]*)>/g)) {
    const type = (match[1].match(/type="([^"]*)"/) || [, ""])[1];
    if (!["note", "tip", "warn", "danger"].includes(type)) {
      fail(page, `callout has unknown type "${type}"`);
    }
  }
  const opens = (source.match(/<callout[\s>]/g) || []).length;
  const closes = (source.match(/<\/callout>/g) || []).length;
  if (opens !== closes) fail(page, `unbalanced callout tags (${opens} open, ${closes} close)`);

  /* --- internal links --- */
  for (const match of source.matchAll(/href="(?!https?:|mailto:|#|\.\/)([^"#]+)(#[^"]*)?"/g)) {
    const target = match[1];
    if (!target.endsWith(".html")) {
      fail(page, `internal link "${target}" does not point at a .html page`);
      continue;
    }
    const slug = target.replace(/\.html$/, "");
    if (!slugs.has(slug)) {
      fail(page, `link to "${target}", which is not a page in _site.json`);
      continue;
    }
    if (match[2]) anchorLinks.push({ page, target: slug, anchor: match[2].slice(1) });
  }
  for (const match of source.matchAll(/href="#([^"]+)"/g)) {
    anchorLinks.push({ page, target: name.replace(/\.html$/, ""), anchor: match[1] });
  }

  /* --- root-absolute paths ---
   *
   * The site publishes to a project subpath (…github.io/candid-core/), so a
   * root-absolute reference resolves above the site root in production while
   * continuing to work on a local server rooted at "/". That asymmetry makes
   * it the one mistake local preview cannot catch, so it is a hard failure. */
  for (const match of source.matchAll(/\b(href|src)="(\/[^\/][^"]*)"/g)) {
    fail(
      page,
      `root-absolute ${match[1]}="${match[2]}"; the site is served from a subpath, so use a relative path`,
    );
  }

  /* --- prose --- */
  const prose = source.replace(/<pre[\s\S]*?<\/pre>/g, " ");
  const code = [...source.matchAll(/<pre[^>]*>([\s\S]*?)<\/pre>/g)].map((m) => m[1]).join("\n");

  for (const rule of FORBIDDEN_IN_CODE) {
    const hit = code.match(rule.pattern);
    if (hit) fail(page, `${rule.message} — found ${JSON.stringify(hit[0].trim())}`);
  }

  /* --- candid-core dependency requirements ---
   *
   * An allowlist, not a blocklist, for the same reason Cargo.toml's `include`
   * is one: it decides which way a mistake falls. A blocklist of the caret
   * spellings someone thought of missed `candid-core = "0.1.0-beta.3"` — which
   * IS a caret requirement, because a bare string in a manifest means `^` —
   * along with the inline-table form and every other version entirely.
   *
   * While the crate version is a prerelease the `=` is mandatory, because a
   * caret requirement never selects a prerelease at all. After 1.0 the plain
   * requirement becomes the correct one and this rule follows automatically.
   *
   * Scoped to manifest dependency lines on purpose. `cargo install --version
   * 0.1.0-beta.3` is NOT the same defect: cargo's manual states that a
   * `--version` without a requirement operator installs exactly that version
   * and is not treated as a caret requirement, unlike a dependency. */
  for (const match of code.matchAll(/^[ \t]*candid-core\s*=\s*(.+)$/gm)) {
    const value = match[1].trim();
    const version = (value.match(/^"([^"]*)"/) ||
      value.match(/\bversion\s*=\s*"([^"]*)"/) ||
      [])[1];
    if (version === undefined) {
      fail(page, `candid-core dependency line with no readable version: ${JSON.stringify(value)}`);
    } else if (version !== REQUIRED_REQ) {
      fail(
        page,
        `candid-core dependency requirement ${JSON.stringify(version)}; it must be ${JSON.stringify(REQUIRED_REQ)}` +
          (IS_PRERELEASE ? " — a caret requirement never selects a prerelease" : ""),
      );
    }
  }
  for (const match of code.matchAll(/\bcargo\s+add\s+candid-core@(\S+)/g)) {
    if (match[1] !== REQUIRED_REQ) {
      fail(
        page,
        `\`cargo add candid-core@${match[1]}\`; the requirement must be ${JSON.stringify(REQUIRED_REQ)}`,
      );
    }
  }
  /* --- the removed surface, and the release note --- */
  const slug = name.replace(/\.html$/, "");
  snippetPages.push({ slug, source });
  if (!REMOVED_SURFACE_PAGES.has(slug)) {
    for (const pattern of REMOVED_SURFACE) {
      const hit = source.match(pattern);
      if (hit) {
        fail(
          page,
          `names ${JSON.stringify(hit[0])}, which latest (0.2.0) has and the 0.3 betas remove; ` +
            "that belongs on the migration page, not here",
        );
      }
    }
  }
  if (NOTE_PAGES.has(slug)) {
    const notes = (source.match(NOTE_RE) || []).length;
    if (slug === "migrating-from-0-2") {
      if (notes !== 0) fail(page, "the migration page is the note; it must not carry another");
    } else if (notes !== 1) {
      fail(
        page,
        `must carry exactly one <callout title="${NOTE_TITLE}"> saying the published packages differ (found ${notes})`,
      );
    } else if (!NOTE_LINK_RE.test(source)) {
      fail(page, `the ${NOTE_TITLE} note must link migrating-from-0-2.html`);
    }
  }
  for (const rule of FORBIDDEN_IN_PROSE) {
    const hit = prose.match(rule.pattern);
    if (hit) fail(page, `${rule.message} — found ${JSON.stringify(hit[0])}`);
  }
  for (const rule of SOFT) {
    const hit = prose.match(rule);
    if (hit) warnings.push(`${page}: soft filler ${JSON.stringify(hit[0])}`);
  }
}

/* --- the snippets: compiled against the tree, or excerpted verbatim --- */
const skipSnippets = process.argv.includes("--skip-snippets");
const snippets = await checkSnippets({
  pages: snippetPages,
  root: join(HERE, ".."),
  skipCompile: skipSnippets,
});
for (const problem of snippets.problems) fail(problem.page, problem.message);
if (skipSnippets) {
  warnings.push("snippets were not compiled (--skip-snippets); CI does not pass that flag");
}

/* --- every sitemap entry has a file --- */
for (const slug of slugs) {
  if (!existsSync(join(CONTENT, `${slug}.html`)))
    fail("_site.json", `lists "${slug}" but content/${slug}.html does not exist`);
}

/* --- the built output actually contains every page --- */
if (existsSync(DIST)) {
  for (const slug of slugs) {
    if (!existsSync(join(DIST, `${slug}.html`))) fail("dist", `${slug}.html was not built`);
  }

  /* --- anchors resolve ---
   *
   * Heading ids are generated by build.mjs, so this can only be checked
   * against dist/. A link to a heading that was renamed lands the reader at
   * the top of the page with no error, which is exactly the kind of rot
   * nobody notices. */
  const idsByPage = new Map();
  for (const slug of slugs) {
    const file = join(DIST, `${slug}.html`);
    if (!existsSync(file)) continue;
    const html = await readFile(file, "utf8");
    idsByPage.set(slug, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
  }
  for (const link of anchorLinks) {
    const ids = idsByPage.get(link.target);
    if (!ids) continue;
    if (!ids.has(decodeURIComponent(link.anchor))) {
      fail(
        link.page,
        `link to "${link.target}.html#${link.anchor}", but that page has no such heading`,
      );
    }
  }
}

if (warnings.length) {
  console.log(`${warnings.length} warning(s):`);
  for (const warning of warnings) console.log(`  ${warning}`);
  console.log("");
}

if (failures.length === 0) {
  const t = snippets.tally;
  console.log(
    `checks passed: ${slugs.size} pages; snippets: ${t.compiled} compiled, ${t.fails} proven stale ` +
      `(must not compile), ${t.excerpts} excerpts matched verbatim, ${t.unchecked.length} unchecked with a reason` +
      (t.unchecked.length ? ` (${t.unchecked.join(", ")})` : ""),
  );
  process.exit(0);
}

console.error(`${failures.length} failure(s):`);
for (const failure of failures) console.error(`  ${failure.page}: ${failure.message}`);
process.exit(1);
