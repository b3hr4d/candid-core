# The candid-core documentation site

A static documentation site with **no dependencies at all** — no site generator,
no CSS framework, no webfonts, no npm install. `build.mjs` uses Node built-ins
only. That is deliberate: this repository exact-pins every dependency it takes,
and a docs site is not a good reason to take a few hundred more.

```sh
node website/build.mjs --serve
```

Then open <http://localhost:4173>. Omit `--serve` to build only. Add
`--allow-missing` while drafting to skip sitemap entries whose file does not
exist yet.

## Layout

```
website/
  build.mjs         the generator: content + assets -> dist/
  check.mjs         the gates (see below)
  content/
    _site.json      the sitemap; this is the navigation, and the page order
    <slug>.html     one body fragment per page
  assets/
    style.css       the whole design system, tokens first
    site.js         theme, mobile nav, search, copy buttons, scrollspy, tabs
    highlight.js    the syntax highlighter, used at build time
    favicon.svg
  dist/             generated; not committed
```

`dist/` is generated output and is git-ignored. Build it in CI and publish that.

## Writing a page

A content file is a **body fragment**: no `<!doctype>`, no `<html>`, no
`<body>`, and no `<h1>`. The generator supplies the page title and lead from
`_site.json`, adds ids and anchors to every `<h2>`/`<h3>`, builds the
table of contents from them, and adds previous/next navigation.

Add the page to `_site.json` first — a file not listed there is never built, and
`check.mjs` fails on it.

### Code blocks

```html
<pre data-lang="rust" data-file="src/limits.rs">
let limits = Limits::default().with_max_input_bytes(512);
</pre>
```

The text inside a `<pre>` is taken **literally**, so write `<T>`, `&` and `->`
unescaped. The one rule is that a sample must not contain the literal string
`</pre>`. Highlighting happens at build time, so a published page needs no
JavaScript to be complete — `site.js` only adds theme, search, copying and the
scrollspy.

Nothing in the output uses an ES module or `fetch()`, and every asset
reference is relative, so the page has no reason to need a server or an
origin. Keep it that way: a `type="module"` script or a `fetch()` for the
search index would both break a page opened directly from disk.

Languages: `rust`, `ts`, `js`, `json`, `bash`, `candid`, `toml`, `text`.

### Callouts, tables and the rest

```html
<callout type="warn" title="Short title">
  <p>Body.</p>
</callout>
```

`type` is `note`, `tip`, `warn` or `danger`. Tables are plain `<table>` and get
wrapped for horizontal scrolling. The other components — `.card-grid`,
`.steps`, `.tabs`, `.api-entry`, `.pill` — are documented by example in
`assets/style.css` and used across the existing pages.

## Gates

```sh
node website/build.mjs && node website/check.mjs
```

`check.mjs` fails the build on:

- a document shell or an `<h1>` in a content fragment;
- a content file missing from `_site.json`, or a sitemap entry with no file;
- a `<pre>` with no `data-lang`, or an unknown language;
- an unbalanced or unknown-typed `<callout>`;
- an internal link to a slug that is not in the sitemap, or a `#fragment` that
  matches no heading on the target page;
- a root-absolute `href`/`src`, which resolves above the site root on Pages
  while still working on a local server — the one mistake local preview cannot
  catch;
- a performance claim, or marketing filler, anywhere in the prose.

And, **in a code block only** — because a page is expected to discuss the
broken spellings, and a copyable line is the thing that must work:

- an install or `npx` line for `@candid-core/cli`, which is not published;
- a caret dependency line for `candid-core`, which cannot resolve, because
  `0.1.0-beta.3` is a prerelease and a caret requirement never selects one;
- a bare `cargo add candid-core`, for the same reason.

It also warns about softer filler without failing.

Every one of those checks exists because a draft got it wrong. Each was
demonstrated by injecting the corresponding fault into a page, watching
`check.mjs` exit non-zero and name it, and then reverting — a gate shown only
by its green path is not evidence.

## Accuracy

The pages state exact API signatures, default limit values, CLI flags, error
codes and version numbers. All of them are checkable against the source. When
you change a public API, grep this directory for its name before you assume the
docs still hold — nothing here is generated from the code, so nothing here
updates itself.

## Publishing

`.github/workflows/docs.yml` builds the site and deploys `website/dist` to
GitHub Pages at <https://b3hr4d.github.io/candid-core/>.

- A **pull request** touching `website/**` builds and runs `check.mjs`. It
  publishes nothing.
- A **merge to `main`** touching `website/**` republishes.
- **Run workflow** on the Actions tab republishes on demand.

The published URL is a project subpath, not a domain root, which is why every
asset reference and internal link the generator emits is relative. If you ever
add an absolute path such as `/assets/style.css`, the site breaks on Pages and
keeps working locally — the worst kind of bug. To check a change against the
real shape of the URL, serve `dist` under a prefix rather than at the root.
