// The page.
//
// Boot order: instantiate the module, compile every release in `wallets/`,
// then render. Everything after that is a pure function of `state` — there is
// no incremental DOM patching here, just re-render of whichever section the
// interaction touched.

import { loadCandidCore } from "./candid-core.js";
import { loadRegistry, compileVersion } from "./registry.js";
import { compareInterfaces, shortId } from "./contract.js";
import { ALL_LINES, negotiate } from "./dapp.js";

const WASM_URL = "candid_core_web.wasm";

const state = {
  core: null,
  registry: null,
  producer: null,
  lines: [...ALL_LINES],
  selected: null,
  cell: null,
  files: {},
  activeFile: null,
  entry: "wallet.did",
};

/* ------------------------------------------------------------------ DOM */

const byId = (id) => document.getElementById(id);

/** Terse element builder: `el("p", {className: "x"}, "text", child)`. */
function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "className") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key === "html") node.innerHTML = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2).toLowerCase(), value);
    else node.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function replace(container, ...children) {
  container.replaceChildren(...children.flat().filter(Boolean));
}

function chip(status, label = status) {
  return el("span", { className: `chip ${status}` }, label);
}

/** A truncated identity that copies the whole thing when clicked. */
function identityButton(id) {
  if (!id) return el("span", { className: "sig absent" }, "absent");
  const button = el(
    "button",
    {
      className: "copy",
      type: "button",
      title: `${id}\n(click to copy)`,
      onClick: async () => {
        try {
          await navigator.clipboard.writeText(id);
          button.classList.add("copied");
          setTimeout(() => button.classList.remove("copied"), 1400);
        } catch {
          // Clipboard access can be denied; the full value is in the title
          // either way, so there is nothing to recover from.
        }
      },
    },
    shortId(id),
  );
  return button;
}

const IDENTITY_ROWS = [
  ["interface_id", "interfaceId", "the actor's wire interface, under both profiles"],
  ["contract_id", "contractId", "the whole semantic Contract, declaration names included"],
  ["source_bundle_id", "sourceBundleId", "raw source bytes and import edges — comments included"],
  ["artifact_id", "artifactId", "the exact octets of the Contract document, producer included"],
];

function identityGrid(view) {
  return el(
    "div",
    { className: "identity-grid" },
    IDENTITY_ROWS.map(([label, key, covers]) =>
      el(
        "dl",
        { className: "identity" },
        el("dt", {}, label),
        el(
          "dd",
          {},
          identityButton(view[key]),
          el("span", { className: "covers" }, covers),
        ),
      ),
    ),
  );
}

/* ------------------------------------------------------------- sections */

function renderRuntime(elapsed) {
  const { producer, registry } = state;
  replace(
    byId("runtime"),
    el("b", {}, `candid-core ${producer.version}`),
    el("span", {}, `· candid ${producer.candid_version} · candid_parser ${producer.candid_parser_version}`),
    el("span", {}, `· ${registry.versions.length} releases compiled in ${elapsed.toFixed(0)} ms`),
    el("span", {}, "· wasm32-unknown-unknown"),
  );
  byId("crate-version").textContent = producer.version;
}

function renderFleet() {
  const cards = state.registry.fleet.map((wallet, index) => {
    const result = negotiate(wallet.release, state.lines, state.registry.byId);
    const served = result.capabilities.filter((entry) => entry.status === "served").length;
    return el(
      "button",
      {
        type: "button",
        className: "card",
        "aria-pressed": String(index === state.selected),
        onClick: () => {
          state.selected = index;
          renderFleet();
          renderDetail();
        },
      },
      el(
        "div",
        { className: "card-top" },
        el("span", { className: "holder" }, wallet.holder),
        chip(result.status, result.status === "full" ? "all features" : result.status),
      ),
      el("span", { className: "canister" }, wallet.canister),
      el(
        "div",
        { className: "card-version" },
        el("b", {}, wallet.version),
        el("span", {}, `${wallet.release.line} line`),
      ),
      el(
        "div",
        { className: "pips", "aria-hidden": "true" },
        result.capabilities.map((entry) =>
          el("span", {
            className: `pip ${
              entry.status === "served"
                ? "served"
                : entry.capability.need === "required"
                  ? "blocked"
                  : "missing"
            }`,
          }),
        ),
      ),
      el("p", { className: "card-note" }, `${served} of ${result.capabilities.length} capabilities · ${wallet.note}`),
    );
  });
  replace(byId("fleet"), cards);
}

function capabilityTable(release) {
  const result = negotiate(release, state.lines, state.registry.byId);
  const rows = result.capabilities.map((entry) => {
    const { capability } = entry;
    return el(
      "tr",
      {},
      el(
        "td",
        { className: "capability-name" },
        capability.label,
        el("span", {}, capability.detail),
      ),
      el("td", {}, chip(entry.status === "served" ? "served" : entry.status)),
      el("td", { className: "sig nowrap" }, capability.need),
      el(
        "td",
        {},
        entry.status === "served"
          ? [
              el("div", { className: "mono" }, `${entry.accept.method} — accept ${entry.accept.id}`),
              el("div", { className: "sig" }, entry.signature),
            ]
          : el(
              "div",
              { className: "sig absent" },
              entry.status === "mismatch"
                ? `${entry.mismatch.accept.method} exists but its signature is not one this frontend can call — ${capability.degraded}`
                : capability.degraded,
            ),
      ),
    );
  });

  return el(
    "div",
    { className: "table-scroll" },
    el(
      "table",
      {},
      el(
        "thead",
        {},
        el(
          "tr",
          {},
          el("th", {}, "Capability"),
          el("th", {}, "Status"),
          el("th", {}, "Need"),
          el("th", {}, "Negotiated call"),
        ),
      ),
      el("tbody", {}, rows),
    ),
  );
}

function methodList(view) {
  return el(
    "ul",
    { className: "method-list" },
    view.methods.map((method) =>
      el("li", {}, el("b", {}, method.name), el("span", { className: "sig" }, method.signature)),
    ),
  );
}

function sourceBlocks(release) {
  return Object.entries(release.sources).map(([file, text]) =>
    el(
      "details",
      {},
      el("summary", {}, `memory:/${file} · ${new Blob([text]).size} bytes`),
      el("pre", {}, text),
    ),
  );
}

function renderDetail() {
  const wallet = state.registry.fleet[state.selected];
  if (!wallet) return replace(byId("detail"));
  const release = wallet.release;
  const result = negotiate(release, state.lines, state.registry.byId);

  replace(
    byId("detail"),
    el(
      "div",
      { className: "detail-head" },
      el("h3", {}, `${wallet.holder}'s wallet`),
      el("span", { className: "canister" }, wallet.canister),
      chip(result.status, result.status === "full" ? "every capability served" : result.status),
      el("span", { className: "sig absent" }, `${wallet.version} · released ${release.released} · ${release.headline}`),
    ),
    el(
      "div",
      { className: "detail-body" },
      el(
        "section",
        {},
        el("h4", {}, "What this frontend negotiated"),
        capabilityTable(release),
      ),
      el(
        "section",
        {},
        el("h4", {}, "Identities computed for this build"),
        identityGrid(release),
      ),
      el(
        "section",
        {},
        el("h4", {}, `Interface surface · ${release.methods.length} methods`),
        methodList(release),
      ),
      el(
        "section",
        {},
        el("h4", {}, "The source it was compiled from"),
        sourceBlocks(release),
      ),
    ),
  );
}

function renderReleases() {
  const head = el(
    "thead",
    {},
    el(
      "tr",
      {},
      el("th", {}, "Release"),
      el("th", { className: "num" }, "Methods"),
      el("th", {}, "interface_id"),
      el("th", {}, "contract_id"),
      el("th", {}, "source_bundle_id"),
      el("th", { className: "num" }, "Doc bytes"),
    ),
  );
  const rows = state.registry.versions.map((version) =>
    el(
      "tr",
      { className: version.id.startsWith("v1.2.") ? "highlight" : null },
      el(
        "td",
        { className: "version-cell" },
        el("b", {}, version.id),
        el("span", {}, `${version.released} · ${version.headline}`),
      ),
      el("td", { className: "num" }, String(version.methods.length)),
      el("td", {}, identityButton(version.interfaceId)),
      el("td", {}, identityButton(version.contractId)),
      el("td", {}, identityButton(version.sourceBundleId)),
      el("td", { className: "num" }, String(version.artifactBytes)),
    ),
  );
  replace(byId("releases"), head, el("tbody", {}, rows));

  const before = state.registry.byId.get("v1.2.0");
  const after = state.registry.byId.get("v1.2.1");
  const sameContract = before.contractId === after.contractId;
  const sameBundle = before.sourceBundleId === after.sourceBundleId;
  replace(
    byId("repackage"),
    el("h4", {}, "The two highlighted rows"),
    el(
      "p",
      {},
      "1.2.1 renamed a file, moved a declaration into another file, reordered the rest, and added " +
        "documentation comments throughout. ",
      el("strong", {}, sameContract ? "Its contract_id and interface_id did not move. " : "Its identities moved. "),
      sameBundle
        ? "Its source_bundle_id did not move either, which would be a bug in this page."
        : "Its source_bundle_id did, because that identity covers raw source bytes and import edges.",
    ),
    el(
      "p",
      {},
      "That is the property a fleet depends on. A team that reformats its ",
      el("code", {}, ".did"),
      " files, splits them differently, or documents them does not strand a single user, and the page " +
        "can prove it rather than assume it — Chi is on 1.2.0 and Dara is on 1.2.1, and no code path " +
        "above distinguishes them.",
    ),
    el(
      "p",
      {},
      "The reverse direction is worth stating too: none of these identities authenticates itself. They " +
        "are content addresses. Something else — a signature, a governance record — is what says a given " +
        "identity is the one you should be running.",
    ),
  );
}

const VERDICT_MARK = { identical: "≡", compatible: "＋", breaking: "✕" };

function renderMatrix() {
  const versions = state.registry.versions;
  const head = el(
    "thead",
    {},
    el(
      "tr",
      {},
      el("th", { className: "corner" }, "client ↓ / wallet →"),
      versions.map((version) => el("th", { scope: "col" }, version.id)),
    ),
  );
  const rows = versions.map((from) =>
    el(
      "tr",
      {},
      el("th", { scope: "row" }, from.id),
      versions.map((to) => {
        if (from.id === to.id) {
          return el("td", {}, el("span", { className: "cell self" }, "—"));
        }
        const comparison = compareInterfaces(from, to);
        const detail =
          comparison.verdict === "breaking"
            ? `${comparison.removed.length + comparison.changed.length}`
            : comparison.verdict === "compatible"
              ? `${comparison.added.length}`
              : "";
        const key = `${from.id}→${to.id}`;
        return el(
          "td",
          {},
          el(
            "button",
            {
              type: "button",
              className: `cell ${comparison.verdict}`,
              "aria-pressed": String(state.cell === key),
              "aria-label": `a client written against ${from.id} meeting a wallet on ${to.id}: ${comparison.verdict}`,
              title: `${from.id} client → ${to.id} wallet: ${comparison.verdict}`,
              onClick: () => {
                state.cell = key;
                renderMatrix();
                renderMatrixDetail(from, to);
              },
            },
            `${VERDICT_MARK[comparison.verdict]}${detail}`,
          ),
        );
      }),
    ),
  );
  replace(byId("matrix"), head, el("tbody", {}, rows));
}

function nameRow(className, label, names, lookup) {
  if (names.length === 0) return null;
  return el(
    "li",
    { className },
    el("b", {}, label),
    el(
      "span",
      { className: "sig" },
      names.map((name) => `${name} : ${lookup(name)}`).join("  ·  "),
    ),
  );
}

function renderMatrixDetail(from, to) {
  const comparison = compareInterfaces(from, to);
  const signature = (view) => (name) =>
    view.methods.find((method) => method.name === name)?.signature ?? "—";

  const explanation = {
    identical:
      "Identical interface_id. The two builds are the same wire interface; a client cannot tell them apart, and does not need to.",
    compatible:
      "Every method the client knows about is still there with the same signature. The new ones are simply invisible to it.",
    breaking:
      "A method the client depends on is gone or has changed shape. Calling it blind is the failure this page exists to prevent.",
  }[comparison.verdict];

  replace(
    byId("matrix-detail"),
    el("h4", {}, `A client built for ${from.id}, talking to a wallet on ${to.id}`),
    el("p", {}, chip(comparison.verdict), " ", explanation),
    el(
      "ul",
      { className: "method-list" },
      nameRow("added", "added", comparison.added, signature(to)),
      nameRow("removed", "removed", comparison.removed, signature(from)),
      nameRow("removed", "changed", comparison.changed, signature(to)),
      comparison.added.length + comparison.removed.length + comparison.changed.length === 0
        ? el("li", {}, el("b", {}, "no change"), el("span", { className: "sig" }, "method for method, the same service"))
        : null,
    ),
  );
}

/* ----------------------------------------------------------- playground */

let compileTimer = null;

function renderEditorTabs() {
  replace(
    byId("editor-tabs"),
    Object.keys(state.files).map((file) =>
      el(
        "button",
        {
          type: "button",
          role: "tab",
          className: "tab",
          "aria-selected": String(file === state.activeFile),
          onClick: () => {
            state.activeFile = file;
            byId("editor").value = state.files[file];
            renderEditorTabs();
          },
        },
        `memory:/${file}${file === state.entry ? " ·" : ""}`,
      ),
    ),
  );
}

function loadCandidate(release) {
  state.files = { ...release.sources };
  state.entry = release.entry;
  state.activeFile = release.entry;
  byId("editor").value = state.files[state.activeFile];
  renderEditorTabs();
  renderCandidate();
}

function renderCandidate() {
  const started = performance.now();
  const candidate = compileVersion(
    state.core,
    { id: "candidate", entry: state.entry, files: Object.keys(state.files) },
    state.files,
  );
  const elapsed = performance.now() - started;
  byId("editor-status").textContent = `compiled in ${elapsed.toFixed(1)} ms`;

  if (!candidate.ok) {
    const items = candidate.failure.diagnostics ?? candidate.failure.violations ?? [];
    replace(
      byId("candidate"),
      el(
        "div",
        { className: "panel" },
        el("h4", {}, `Rejected · ${items.length} diagnostic${items.length === 1 ? "" : "s"}`),
        items.map((item) =>
          el(
            "div",
            { className: "diagnostic" },
            el("code", {}, item.code),
            el("p", {}, item.message ?? "(no message)"),
            item.span || item.path || item.resource_limit
              ? el(
                  "span",
                  { className: "where" },
                  [
                    item.span?.source,
                    item.path,
                    item.resource_limit &&
                      `${item.resource_limit.resource}: observed ${item.resource_limit.observed} > limit ${item.resource_limit.limit}`,
                  ]
                    .filter(Boolean)
                    .join(" · "),
                )
              : null,
          ),
        ),
        el(
          "p",
          { className: "sig absent" },
          "Codes and paths are the stable machine surface; message text is not. This is the same " +
            "shape the candid-core CLI prints.",
        ),
      ),
    );
    return;
  }

  const baseline = state.registry.byId.get("v2.1.0");
  const drift = compareInterfaces(baseline, candidate);
  const support = negotiate(candidate, state.lines, state.registry.byId);

  const impact = state.registry.versions.map((version) => {
    const comparison = compareInterfaces(version, candidate);
    const holders = state.registry.fleet.filter((wallet) => wallet.version === version.id);
    return el(
      "div",
      { className: "impact-row" },
      el("span", { className: "who" }, `${version.id} → candidate`),
      el(
        "span",
        { className: "why" },
        holders.map((wallet) => wallet.holder).join(", ") || "nobody on this build",
      ),
      chip(comparison.verdict),
    );
  });

  replace(
    byId("candidate"),
    el(
      "div",
      { className: "panel" },
      el("h4", {}, "Identities for the candidate"),
      identityGrid(candidate),
    ),
    el(
      "div",
      { className: "panel" },
      el("h4", {}, "Would this frontend still drive it?"),
      el(
        "p",
        { className: "sig" },
        chip(support.status, support.status === "full" ? "every capability served" : support.status),
        " ",
        support.failed.length === 0
          ? "Every capability the page declares is negotiable against this candidate."
          : `Unserved: ${support.failed.map((entry) => entry.capability.label.toLowerCase()).join(", ")}.`,
      ),
      capabilityTable(candidate),
    ),
    el(
      "div",
      { className: "panel" },
      el("h4", {}, "What it does to wallets already in the fleet"),
      el("div", { className: "impact" }, impact),
      el(
        "p",
        { className: "sig absent" },
        "Each row is a wallet on that release being upgraded to the candidate. A breaking row is a " +
          "user whose page stops working if you ship this without a client that speaks both.",
      ),
    ),
    el(
      "div",
      { className: "panel" },
      el("h4", {}, `Against 2.1.0 · ${drift.verdict}`),
      el(
        "ul",
        { className: "method-list" },
        nameRow("added", "added", drift.added, (name) => candidate.methods.find((m) => m.name === name).signature),
        nameRow("removed", "removed", drift.removed, (name) => baseline.methods.find((m) => m.name === name).signature),
        nameRow("removed", "changed", drift.changed, (name) => candidate.methods.find((m) => m.name === name).signature),
        drift.verdict === "identical"
          ? el("li", {}, el("b", {}, "identical"), el("span", { className: "sig" }, "same interface_id as 2.1.0"))
          : null,
      ),
    ),
  );
}

/* ----------------------------------------------------------------- boot */

function bindControls() {
  for (const [id, line] of [
    ["line-1", "1.x"],
    ["line-2", "2.x"],
  ]) {
    byId(id).addEventListener("change", (event) => {
      state.lines = event.target.checked
        ? [...new Set([...state.lines, line])]
        : state.lines.filter((entry) => entry !== line);
      renderFleet();
      renderDetail();
      renderCandidate();
    });
  }

  byId("editor").addEventListener("input", (event) => {
    state.files[state.activeFile] = event.target.value;
    clearTimeout(compileTimer);
    compileTimer = setTimeout(renderCandidate, 220);
  });

  byId("reset-editor").addEventListener("click", () => {
    loadCandidate(state.registry.byId.get("v2.1.0"));
  });
}

async function boot() {
  const started = performance.now();
  try {
    state.core = await loadCandidCore(WASM_URL);
    state.producer = state.core.producer();
    state.registry = await loadRegistry(state.core, "wallets");
  } catch (error) {
    const runtime = byId("runtime");
    runtime.classList.add("failed");
    replace(runtime, `could not start: ${error.message}`);
    throw error;
  }

  const failed = state.registry.versions.filter((version) => !version.ok);
  if (failed.length > 0) {
    const runtime = byId("runtime");
    runtime.classList.add("failed");
    replace(runtime, `${failed.map((version) => version.id).join(", ")} did not compile`);
    return;
  }

  state.selected = 0;
  // The breaking step is the one worth landing on, so the matrix opens on it.
  state.cell = "v1.2.1→v2.0.0";

  renderRuntime(performance.now() - started);
  renderFleet();
  renderDetail();
  renderReleases();
  renderMatrix();
  renderMatrixDetail(state.registry.byId.get("v1.2.1"), state.registry.byId.get("v2.0.0"));
  bindControls();
  loadCandidate(state.registry.byId.get("v2.1.0"));

  byId("fineprint").textContent =
    "bounds in force: interactive_v1 · max_source_bytes 1 MiB · max_bundle_bytes 8 MiB · " +
    "max_input_bytes 4 MiB — the same profile the command-line tool uses";
}

boot();
