// The library surface of @candid-core/cli (issue #153): data in, data out.
// Both functions hand one JSON-serializable request to the wasm compiler and
// return its parsed JSON response verbatim — no eval, no dynamic import, no
// filesystem access from the wasm side, and nothing thrown for data errors:
// a failure is the same `{ ok: false, diagnostics }` document the native
// `candid-core` CLI prints, passed through byte-for-byte.

import initWasm, {
  didToContract as wasmDidToContract,
  didToModule as wasmDidToModule,
} from "../wasm/candid_core_wasm.js";

let initialized;

/**
 * Initialize the embedded wasm module once. Node needs no argument (the
 * artifact is read from this package); a browser may also pass nothing (the
 * artifact is fetched relative to this module) or supply its own
 * `BufferSource`/`URL`/`Response`.
 */
export function init(input) {
  if (initialized === undefined) {
    initialized = (async () => {
      if (input === undefined && typeof process !== "undefined" && process.versions?.node) {
        const { readFile } = await import("node:fs/promises");
        const bytes = await readFile(
          new URL("../wasm/candid_core_wasm_bg.wasm", import.meta.url),
        );
        await initWasm({ module_or_path: bytes });
      } else {
        await initWasm(input === undefined ? undefined : { module_or_path: input });
      }
    })();
  }
  return initialized;
}

function requestOf(sources) {
  return JSON.stringify(typeof sources === "string" ? { source: sources } : sources);
}

/**
 * Compile Candid sources into a one-document ContractEnvelope carrying the
 * `org.candid-core.field-names/v1` extension — exactly the document
 * `candid-core compile <path> --envelope` emits, ready for
 * `schemaFromContract`. `sources` is Candid text, or
 * `{ entry, files: { name: text } }` for a multi-file bundle.
 *
 * Returns the parsed envelope (`"contract" in result`), or
 * `{ ok: false, diagnostics }` with the compiler's diagnostics verbatim.
 */
export async function didToContract(sources) {
  await init();
  return JSON.parse(wasmDidToContract(requestOf(sources)));
}

/**
 * Generate the `@candid-core/schema` TypeScript module for Candid sources.
 * Returns `{ ok: true, module }` with the generated text — byte-identical to
 * what the Rust-native generator emits — or `{ ok: false, diagnostics }`.
 */
export async function didToModule(sources) {
  await init();
  return JSON.parse(wasmDidToModule(requestOf(sources)));
}
