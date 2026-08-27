#!/usr/bin/env node
/* The candid-core documentation site generator.
 *
 * Zero dependencies — Node built-ins only. It reads content/_site.json for the
 * navigation and content/<slug>.html for each page body, then writes a flat set
 * of static pages into dist/. Syntax highlighting runs here rather than in the
 * browser, so a published page needs no JavaScript to be complete and readable.
 *
 *   node website/build.mjs            build into website/dist
 *   node website/build.mjs --serve    build, then serve dist on :4173
 *
 * Authoring notes for content/*.html:
 *   - The file is a body fragment. No <html>, <head> or <body>.
 *   - Code goes in <pre data-lang="rust" data-file="src/lib.rs">…</pre>. The
 *     text inside is taken literally: write `<T>` and `&` unescaped, never
 *     write the literal string "</pre>" inside a sample.
 *   - <h2>/<h3> get ids, anchors and a table-of-contents entry automatically.
 */

import { readFile, writeFile, mkdir, readdir, rm, copyFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { highlight } from "./assets/highlight.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const CONTENT = join(HERE, "content");
const ASSETS = join(HERE, "assets");
const OUT = join(HERE, "dist");

// Incremental authoring: skip pages listed in the sitemap whose file is not
// written yet, instead of failing the whole build.
const ALLOW_MISSING = process.argv.includes("--allow-missing");

/* ----------------------------------------------------------------- utils */

const escapeHtml = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const slugify = (text) =>
  text
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/&[a-z]+;/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);

/* --------------------------------------------------------------- icons  */

const ICON = {
  search:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>',
  menu: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 7h16M4 12h16M4 17h16"/></svg>',
  sun: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4"/></svg>',
  github:
    '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 .5A11.5 11.5 0 0 0 .5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.53-1.35-1.29-1.7-1.29-1.7-1.05-.72.08-.71.08-.71 1.16.08 1.77 1.2 1.77 1.2 1.03 1.78 2.7 1.27 3.36.97.1-.75.4-1.27.73-1.56-2.56-.29-5.25-1.28-5.25-5.7 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.18 1.18a11 11 0 0 1 5.8 0c2.2-1.5 3.17-1.18 3.17-1.18.63 1.59.23 2.76.12 3.05.74.81 1.18 1.84 1.18 3.1 0 4.43-2.69 5.4-5.26 5.69.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12 11.5 11.5 0 0 0 12 .5Z"/></svg>',
  copy: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="13" height="13"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>',
  note: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/></svg>',
  tip: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m20 6-11 11-5-5"/></svg>',
  warn: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4M12 17h.01"/></svg>',
  danger:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="m15 9-6 6M9 9l6 6"/></svg>',
};

const MARK = `<svg class="brand-mark" viewBox="0 0 32 32" role="img" aria-hidden="true"><defs><linearGradient id="bm" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7b3fe4"/><stop offset=".55" stop-color="#c94bb0"/><stop offset="1" stop-color="#f7802e"/></linearGradient></defs><rect width="32" height="32" rx="8" fill="url(#bm)"/><path d="M11 10.5h7.5M11 16h5.5M11 21.5h7.5" stroke="#fff" stroke-width="2.1" stroke-linecap="round" opacity=".92"/><circle cx="22.6" cy="10.5" r="1.7" fill="#fff"/><circle cx="20.6" cy="16" r="1.7" fill="#fff"/><circle cx="22.6" cy="21.5" r="1.7" fill="#fff"/></svg>`;

/* ---------------------------------------------------- content transforms */

/** Replaces <pre data-lang=…>…</pre> with a highlighted, copyable code block. */
function renderCodeBlocks(html) {
  return html.replace(/<pre([^>]*)>([\s\S]*?)<\/pre>/g, (whole, attributes, body) => {
    const lang = (attributes.match(/data-lang="([^"]*)"/) || [, "text"])[1];
    const file = (attributes.match(/data-file="([^"]*)"/) || [, ""])[1];
    const source = body.replace(/^\n/, "").replace(/\s+$/, "");
    const head = [
      `<span class="code-lang">${escapeHtml(lang)}</span>`,
      file ? `<span class="code-file">${escapeHtml(file)}</span>` : "",
      `<button class="copy-button" type="button" aria-label="Copy code">${ICON.copy}<span class="copy-label">copy</span></button>`,
    ].join("");
    return [
      '<div class="code-block">',
      `<div class="code-head">${head}</div>`,
      `<pre><code data-lang="${escapeHtml(lang)}" data-highlighted="true">${highlight(source, lang)}</code></pre>`,
      "</div>",
    ].join("");
  });
}

/** Expands <callout type="tip" title="…">…</callout> into styled markup. */
function renderCallouts(html) {
  return html.replace(/<callout([^>]*)>([\s\S]*?)<\/callout>/g, (whole, attributes, body) => {
    const type = (attributes.match(/type="([^"]*)"/) || [, "note"])[1];
    const title = (attributes.match(/title="([^"]*)"/) || [, ""])[1];
    const icon = ICON[type] || ICON.note;
    const heading = title ? `<span class="callout-title">${escapeHtml(title)}</span>` : "";
    return `<div class="callout ${escapeHtml(type)}">${icon}<div class="callout-body">${heading}${body}</div></div>`;
  });
}

/** Wraps bare tables so they scroll horizontally on narrow screens. */
function wrapTables(html) {
  return html
    .replace(/<table>/g, '<div class="table-wrap"><table>')
    .replace(/<\/table>/g, "</table></div>");
}

/** Adds ids and anchor links to h2/h3/h4, and collects the table of contents. */
function headingsAndToc(html) {
  const toc = [];
  const used = new Set();
  const out = html.replace(
    /<h([234])(\s[^>]*)?>([\s\S]*?)<\/h\1>/g,
    (whole, level, attributes, inner) => {
      const attrs = attributes || "";
      let id = (attrs.match(/id="([^"]*)"/) || [, ""])[1] || slugify(inner);
      if (!id) id = `section-${toc.length + 1}`;
      let unique = id;
      let n = 2;
      while (used.has(unique)) unique = `${id}-${n++}`;
      used.add(unique);
      const text = inner.replace(/<[^>]+>/g, "").trim();
      if (level !== "4") toc.push({ level: Number(level), id: unique, text });
      const rest = attrs.replace(/\sid="[^"]*"/, "");
      return `<h${level} id="${unique}"${rest}>${inner}<a class="heading-anchor" href="#${unique}" aria-label="Link to ${escapeHtml(text)}">#</a></h${level}>`;
    },
  );
  return { html: out, toc };
}

function renderToc(toc) {
  if (toc.length < 2) return "";
  const items = toc
    .map(
      (entry) =>
        `<li class="toc-h${entry.level}"><a href="#${entry.id}">${escapeHtml(entry.text)}</a></li>`,
    )
    .join("");
  return `<nav class="toc" aria-label="On this page"><h2>On this page</h2><ul>${items}</ul></nav>`;
}

/* --------------------------------------------------------------- layout */

function renderNav(site, activeSlug) {
  return site.sections
    .map((section) => {
      const items = section.pages
        .map((page) => {
          const current = page.slug === activeSlug ? ' aria-current="page"' : "";
          const href = page.slug === "index" ? "./" : `${page.slug}.html`;
          return `<li><a href="${href}"${current}>${escapeHtml(page.nav || page.title)}</a></li>`;
        })
        .join("");
      return `<div class="nav-section"><h2>${escapeHtml(section.title)}</h2><ul>${items}</ul></div>`;
    })
    .join("");
}

function flatPages(site) {
  return site.sections.flatMap((section) =>
    section.pages.map((page) => ({ ...page, section: section.title })),
  );
}

function renderPageNav(pages, index) {
  const previous = pages[index - 1];
  const next = pages[index + 1];
  if (!previous && !next) return "";
  const link = (page, direction) => {
    if (!page) return '<span style="flex:1"></span>';
    const href = page.slug === "index" ? "./" : `${page.slug}.html`;
    return `<a class="${direction}" href="${href}"><span class="dir">${direction === "next" ? "Next" : "Previous"}</span><span class="label">${escapeHtml(page.title)}</span></a>`;
  };
  return `<nav class="page-nav" aria-label="Page navigation">${link(previous, "prev")}${link(next, "next")}</nav>`;
}

function shell({ site, page, body, toc, nav, pageNav, isLanding }) {
  const title = page.slug === "index" ? site.title : `${page.title} · ${site.title}`;
  const description = page.lead || site.description;
  const editUrl = `${site.repo}/blob/main/website/content/${page.slug}.html`;

  const header = `
<header class="site-header">
  <button class="icon-button" id="menu-toggle" type="button" aria-label="Open navigation" aria-expanded="false">${ICON.menu}</button>
  <a class="brand" href="./">${MARK}<span class="brand-name">candid-core</span><span class="brand-tag">docs</span></a>
  <span class="header-spacer"></span>
  <button class="search-trigger" id="search-trigger" type="button">${ICON.search}<span class="search-label">Search</span><kbd>⌘K</kbd></button>
  <div class="header-links">
    <a class="header-link" href="${site.repo}" rel="noreferrer noopener">${ICON.github}<span>GitHub</span></a>
    <button class="icon-button" id="theme-toggle" type="button" aria-label="Switch theme">${ICON.sun}</button>
  </div>
</header>`;

  const searchDialog = `
<dialog class="search-dialog" id="search-dialog" aria-label="Search the documentation">
  <div class="search-head">${ICON.search}<input id="search-input" type="search" placeholder="Search the documentation…" autocomplete="off" spellcheck="false"></div>
  <ul id="search-results"></ul>
</dialog>`;

  const footer = `
<footer class="site-footer">
  <div class="site-footer-inner">
    <span>${escapeHtml(site.title)} — Apache-2.0</span>
    <span class="sep"></span>
    <a href="${site.repo}">Repository</a>
    <a href="${site.repo}/issues">Issues</a>
    <a href="https://crates.io/crates/candid-core">crates.io</a>
    <a href="https://www.npmjs.com/package/@candid-core/schema">npm</a>
  </div>
</footer>`;

  const main = isLanding
    ? `<main id="main" class="landing">${body}</main>`
    : `<div class="layout${toc ? "" : " no-toc"}">
  <div class="nav-backdrop" hidden></div>
  <aside class="sidebar" aria-label="Documentation navigation">${nav}</aside>
  <main id="main" class="content${page.wide ? " wide" : ""}">
    <div class="content-inner">
      ${body}
      ${pageNav}
      <p class="edit-link"><a href="${editUrl}">Edit this page on GitHub</a></p>
    </div>
  </main>
  ${toc}
</div>`;

  return `<!doctype html>
<html lang="en" data-theme="light" data-base="">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
<meta name="description" content="${escapeHtml(description)}">
<meta property="og:title" content="${escapeHtml(title)}">
<meta property="og:description" content="${escapeHtml(description)}">
<meta property="og:type" content="website">
<link rel="icon" href="assets/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="assets/style.css">
<script>
  (function () {
    try {
      var stored = localStorage.getItem("candid-core-theme");
      var theme = stored || (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
      document.documentElement.setAttribute("data-theme", theme);
    } catch (error) {}
  })();
</script>
</head>
<body>
<a class="skip-link" href="#main">Skip to content</a>
${header}
${main}
${footer}
${searchDialog}
<script src="assets/search-index.js"></script>
<script src="assets/site.js"></script>
</body>
</html>
`;
}

/* ---------------------------------------------------------------- build */

function plainText(html) {
  return html
    .replace(/<pre[\s\S]*?<\/pre>/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/&[a-z]+;/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

async function build() {
  const site = JSON.parse(await readFile(join(CONTENT, "_site.json"), "utf8"));
  const pages = flatPages(site);

  if (existsSync(OUT)) await rm(OUT, { recursive: true });
  await mkdir(join(OUT, "assets"), { recursive: true });

  const index = [];
  let built = 0;

  for (let i = 0; i < pages.length; i += 1) {
    const page = pages[i];
    const source = join(CONTENT, `${page.slug}.html`);
    if (!existsSync(source)) {
      if (!ALLOW_MISSING) {
        throw new Error(`content/${page.slug}.html is listed in _site.json but does not exist`);
      }
      console.warn(`  skipping ${page.slug}: content/${page.slug}.html does not exist yet`);
      continue;
    }
    const raw = await readFile(source, "utf8");
    const isLanding = page.layout === "landing";

    let body = raw;
    let toc = "";
    if (!isLanding) {
      const heading = `<h1>${escapeHtml(page.title)}</h1>${page.lead ? `<p class="page-lead">${page.lead}</p>` : ""}`;
      const processed = headingsAndToc(body);
      toc = renderToc(processed.toc);
      body = heading + processed.html;
      for (const entry of processed.toc) {
        // Deep-link headings are searchable in their own right.
        if (entry.level === 2) {
          index.push({
            title: `${page.title} › ${entry.text}`,
            section: page.section,
            url: `${page.slug === "index" ? "index" : page.slug}.html#${entry.id}`,
            headings: entry.text,
            lead: "",
            body: "",
          });
        }
      }
    }

    body = renderCallouts(body);
    body = renderCodeBlocks(body);
    body = wrapTables(body);

    const html = shell({
      site,
      page,
      body,
      toc,
      nav: renderNav(site, page.slug),
      pageNav: renderPageNav(pages, i),
      isLanding,
    });

    await writeFile(join(OUT, `${page.slug}.html`), html, "utf8");
    built += 1;

    index.unshift({
      title: page.title,
      section: page.section,
      url: page.slug === "index" ? "./" : `${page.slug}.html`,
      headings: "",
      lead: page.lead ? plainText(page.lead) : "",
      body: plainText(body).toLowerCase().slice(0, 6000),
    });
  }

  // Headings carry no body text of their own; give each one its page's text so
  // a heading hit still shows a useful snippet.
  const byPage = new Map(
    index.filter((entry) => !entry.title.includes(" › ")).map((entry) => [entry.title, entry]),
  );
  for (const entry of index) {
    if (entry.title.includes(" › ")) {
      const parent = byPage.get(entry.title.split(" › ")[0]);
      if (parent) entry.body = parent.body;
    }
  }

  await writeFile(
    join(OUT, "assets", "search-index.js"),
    `window.CANDID_CORE_SEARCH_INDEX = ${JSON.stringify(index)};\n`,
    "utf8",
  );

  for (const name of await readdir(ASSETS)) {
    const from = join(ASSETS, name);
    if ((await stat(from)).isFile()) await copyFile(from, join(OUT, "assets", name));
  }

  // GitHub Pages runs Jekyll unless told not to, which would drop any path
  // beginning with an underscore.
  await writeFile(join(OUT, ".nojekyll"), "", "utf8");

  console.log(`built ${built}/${pages.length} pages -> ${OUT}`);
  return { site, pages };
}

/* --------------------------------------------------------------- server */

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
};

async function serve(port) {
  const { createServer } = await import("node:http");
  const { extname } = await import("node:path");
  createServer(async (request, response) => {
    try {
      let path = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
      if (path.endsWith("/")) path += "index.html";
      const file = resolve(OUT, `.${path}`);
      if (!file.startsWith(OUT)) {
        response.writeHead(403).end("forbidden");
        return;
      }
      const data = await readFile(file);
      response.writeHead(200, {
        "content-type": MIME[extname(file)] || "application/octet-stream",
        // A preview server exists to show the file you just wrote.
        "cache-control": "no-store",
      });
      response.end(data);
    } catch (error) {
      response.writeHead(404, { "content-type": "text/html; charset=utf-8" });
      response.end("<h1>404</h1>");
    }
  }).listen(port, () => console.log(`serving ${OUT} on http://localhost:${port}`));
}

const argv = process.argv.slice(2);
await build();
if (argv.includes("--serve")) {
  const at = argv.indexOf("--port");
  await serve(at > -1 ? Number(argv[at + 1]) : 4173);
}
