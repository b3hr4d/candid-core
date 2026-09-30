# The candid-core documentation site

A static documentation site with **no dependencies at all** — no site generator,
no CSS framework, no webfonts, no npm install. `build.mjs` and `check.mjs` use
Node built-ins only. That is deliberate: this repository exact-pins every
dependency it takes, and a docs site is not a good reason to take a few hundred
more. The one thing the check borrows is the TypeScript compiler already pinned
for the runtime package, to compile the snippets (see Snippets below).

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
  snippets.mjs      the TypeScript snippet gates, run by check.mjs
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
- a performance claim, or marketing filler, anywhere in the prose;
- a name the published `@candid-core/schema` 0.2.0 has and the next release
  removes (`createActor`, `httpTransport`, `PrincipalValue`, the four removed
  subpaths, `unrepresentable_option`, …) on any page but the migration page and
  the release history on the status page — in prose or in a code block, because
  a block that shows one teaches it as surely as a sentence does;
- a page that describes the TypeScript packages without carrying exactly one
  `Not yet released` callout that links the migration page. The site describes
  the surface the repository builds, which is the next release; the published
  packages differ, and each page says so once, in that one place;
- any TypeScript or JavaScript snippet that is not verified (next section).

And, **in a code block only** — because a page is expected to discuss the
broken spellings, and a copyable line is the thing that must work:

- an install, `npx` or bare `import` line for any name in `UNPUBLISHED_NPM`
  in `check.mjs` — the list this repository keeps of names it has prepared but
  not published. The list is empty today; adding a name is what preparing the
  next package does, and emptying it is what publishing one does;
- a `candid-core` dependency requirement that is anything other than the
  exact pin. The required spelling is derived from the `version` in
  `Cargo.toml` rather than written down here, so a release makes every stale
  line in the documentation fail on the next run. While that version is a
  prerelease the `=` is mandatory, because a caret requirement never selects
  a prerelease — and note that a bare `candid-core = "0.1.0-beta.3"` _is_ a
  caret requirement. After 1.0 the plain requirement becomes the correct one
  and this rule follows on its own;
- a bare `cargo add candid-core`, or `cargo add candid-core@<wrong-req>`.

`cargo install candid-core --version 0.1.0-beta.3` is deliberately **not**
flagged. Cargo's manual states that a `--version` without a requirement
operator installs exactly that version and is not treated as a caret
requirement — unlike a dependency in a manifest, where a bare string is one.

It also warns about softer filler without failing.

### Snippets

Every `<pre>` of language `ts` or `js` is in exactly one of these classes, and
a block in none of them fails the build:

| Attribute | Gate |
| --- | --- |
| `data-file="path"` | An excerpt of a repository file. Every run of lines must appear in that file verbatim (whitespace trimmed; a trailing `{` or `;` is ignored, so a signature can be quoted without its body). Mark an intended gap with a line `// …`, or add `data-partial` to require only that the lines appear in the file in order. |
| `data-check` | Compiled on its own by the pinned compiler, and must have no diagnostics. It may sit beside `data-file`. |
| `data-check="name"` | Blocks of one page with the same name are concatenated, in page order, into one file: an example built up across several blocks. |
| `data-check-fails="TS2322@3"` | Must **not** compile: the compiler must report that code at that line of the block (1-based) and nothing else anywhere in it, so a block whose showcased line was fixed cannot stay "proven stale" because another line keeps the same error. This is how the migration page proves a "before" snippet is stale rather than asserting it. |
| `data-unchecked="reason"` | The visible opt-out, with a reason. Every exemption is listed in the summary line. |

A `js` block is written to a `.js` file and compiled with `allowJs` and
`checkJs`, so TypeScript-only syntax in it is refused as the syntax error a
reader copying it would get.

Compilation uses the TypeScript pinned in `crates/candid-core-ts/ts`
(`npm ci` there installs it; the site itself still takes no dependency) with
`strict`, resolving `@candid-core/schema` and its three subpaths to the source
in that directory and `@candid-core/cli` to its hand-written declarations.
Exactly the four exported subpaths resolve, so an import of a removed one is a
compile error, which is what the migration page's "before" blocks rely on.
A snippet may import `./ledger.ts`, `./shadowing.ts` or `./generated/interface`,
which the gate supplies from the checked-in goldens, so the import resolves to
real generated output. If the compiler is not installed the check fails and
says how to install it; `--skip-snippets` checks everything else while
drafting and is never passed in CI.

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

- A **pull request** touching `website/**`, or anything a snippet is compiled
  against or excerpted from (the runtime package's sources and tests, the
  generator's goldens and library, the CLI package's library and tests),
  installs the pinned compiler, builds and runs `check.mjs`. It publishes
  nothing.
- A **merge to `main`** touching the same paths republishes.
- **Run workflow** on the Actions tab republishes on demand — but only with
  `main` selected. There is one Pages site and no preview channel, so a
  dispatch from any other ref builds and checks, prints a warning saying it
  did not publish, and skips the deploy.

The published URL is a project subpath, not a domain root, which is why every
asset reference and internal link the generator emits is relative. If you ever
add an absolute path such as `/assets/style.css`, the site breaks on Pages and
keeps working locally — the worst kind of bug. To check a change against the
real shape of the URL, serve `dist` under a prefix rather than at the root.
