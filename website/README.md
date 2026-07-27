# The multi-version wallet demo

A live page that runs one decentralized-application frontend against a fleet of
canisters sitting on six different interface versions, and shows — rather than
asserts — why none of them breaks.

The premise is the ordinary one for a dapp where users own their own canisters:
every holder upgrades their wallet on their own schedule, or never, so the fleet
is permanently spread across releases. The page fetches each release's `.did`
files, compiles them **in the browser** with `candid-core` built for
`wasm32-unknown-unknown`, and uses the canonical identities and type graph that
come back to decide what the frontend can do with each wallet.

```sh
./website/build.sh                                   # build the module
node website/verify.mjs                              # check the page's claims
python3 -m http.server --directory website/app 8080  # then open localhost:8080
```

`file://` will not work — ES modules and `fetch` need an origin. Any static
server does.

## What is actually computed here

Nothing on the page is precomputed at build time. Delete
`app/candid_core_web.wasm` and there is nothing left to render.

| The page shows | Where it comes from |
| --- | --- |
| Per-release `interface_id`, `contract_id`, `source_bundle_id` | The compiled Contract and its provenance sidecar |
| `artifact_id` and the document byte count | `artifact_id_with_limits(ArtifactKind::ContractJsonV1, …)` over the exact octets the page received |
| Method signatures with field and argument names | The canonical type graph, with `SourceInfo.field_labels` putting names back on labels the graph reduced to Candid hashes |
| Which upgrades break a client | Structural fingerprints of each method's function type, compared across separately compiled contracts |
| Compile failures | The compiler's own `diagnostics`, in the same shape the `candid-core` CLI prints |

Two claims in particular are worth watching, because they are the reason a fleet
like this holds together:

- **1.2.1 is 1.2.0 with the sources rewritten.** A renamed file, a declaration
  moved between files, reordered declarations, and documentation comments
  throughout. `contract_id` and `interface_id` do not move; `source_bundle_id`
  does. Two holders in the fleet are on those two builds and no code path in the
  page distinguishes them.
- **2.0.0 is the one breaking step.** `owner` is gone and `transfer` changed
  shape, so a client written against 1.x cannot drive a 2.x wallet blind. The
  frontend carries an accept for each shape and negotiates; untick "the 1.x
  line" on the page to see exactly which users a support drop would strand.

## Layout

```
website/
├── build.sh          one cargo invocation and one copy
├── verify.mjs        the page's claims, checked in Node against the same modules
├── wasm/             the bridge crate: a cdylib exporting a raw C ABI
└── app/              the page itself — no bundler, no npm, no framework
    ├── index.html
    ├── styles.css
    ├── js/
    │   ├── candid-core.js   hand-written glue for the raw ABI
    │   ├── contract.js      walking the canonical graph: fingerprints, rendering
    │   ├── registry.js      fetch the releases, compile each bundle
    │   ├── dapp.js          the frontend's declared capabilities and negotiation
    │   └── main.js          rendering
    └── wallets/      registry.json plus one directory of .did files per release
```

## Design constraints

- **No `wasm-bindgen`.** `candid-core` deliberately takes no `wasm-bindgen`,
  `js-sys`, or `web-time` production dependency; a demo that reached for one to
  reach the browser would weaken that claim. The bridge is four `extern "C"`
  functions over linear memory and one JSON document each way, and
  `app/js/candid-core.js` is the glue.
- **No filesystem surface.** The bridge builds `candid-core` with
  `default-features = false` and only the `compiler` feature — no `cap-std`,
  nothing that could open a file. An import the page did not supply is a
  resolver error, never a fetch.
- **No build step for the page.** No bundler, no npm, no framework, no CDN. The
  files served are the files in `app/`.
- **Every bound still applies.** `Limits::default()` — the versioned
  `interactive_v1` profile — governs the browser exactly as it governs the CLI.
  Paste a megabyte of padding into the editor and the result is a structured
  `resource_limit_exceeded`, not a hung tab.

## Editing the releases

`app/wallets/registry.json` lists the releases and who in the fleet runs them;
each release is a directory of ordinary `.did` files. Add or change one and
rerun `node website/verify.mjs` — it asserts the relationships the page's prose
claims (which step is breaking, which repackage is identity-stable, that one
frontend serves the whole fleet) and fails if an edit falsifies any of them.

## Deployment

`.github/workflows/website.yml` builds the module, runs the verifier, and
publishes `website/app` to GitHub Pages on every push to `main`. Pull requests
run the build and the verifier without deploying.

The workflow needs Pages enabled for the repository with **Source: GitHub
Actions** (Settings → Pages). Until that is set, the deploy job fails while the
build and verify jobs keep passing.
