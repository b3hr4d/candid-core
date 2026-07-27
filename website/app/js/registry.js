// Loading the fleet's release history and compiling every version of it.
//
// The `.did` files under `wallets/` are ordinary source files, fetched at run
// time and compiled in the browser. Nothing about the identities on the page
// is baked in at build time; delete the `.wasm` and the page has nothing to
// show.

import { contractView } from "./contract.js";

/** Reads a path under `wallets/` as text. Overridable so Node can reuse this. */
async function fetchText(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`cannot read ${path}: ${response.status}`);
  return response.text();
}

/**
 * Compiles one version's bundle.
 *
 * The bundle is handed over whole — entry plus every file it imports — and the
 * compiler resolves imports against that map and nothing else. There is no
 * filesystem here and no network fetch inside the compiler; a source the page
 * did not supply is a resolver error, not a surprise download.
 */
export function compileVersion(core, version, sources) {
  const request = {
    entry: `memory:/${version.entry}`,
    sources: Object.fromEntries(
      Object.entries(sources).map(([file, text]) => [`memory:/${file}`, text]),
    ),
  };
  const result = core.compile(request);
  if (!result.ok) return { ...version, ok: false, failure: result };
  return { ...version, ok: true, sources, ...contractView(result) };
}

/**
 * Fetches `registry.json`, every `.did` file it names, and compiles them all.
 *
 * @param {object} core The instantiated bridge.
 * @param {string} base Directory holding `registry.json`.
 * @param {(path: string) => Promise<string>} read Text reader, injected so the
 *   Node verifier can run this exact code path against the filesystem.
 */
export async function loadRegistry(core, base = "wallets", read = fetchText) {
  const registry = JSON.parse(await read(`${base}/registry.json`));
  const versions = [];
  for (const version of registry.versions) {
    const sources = {};
    for (const file of version.files) {
      sources[file] = await read(`${base}/${version.id}/${file}`);
    }
    versions.push(compileVersion(core, version, sources));
  }
  const byId = new Map(versions.map((version) => [version.id, version]));
  const fleet = registry.fleet.map((wallet) => ({ ...wallet, release: byId.get(wallet.version) }));
  return { ...registry, versions, byId, fleet };
}
