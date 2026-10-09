// The library surface of @candid-core/cli: data in, data out.
// Every function hands one JSON-serializable request to the wasm compiler and
// return its parsed JSON response verbatim — no eval and no network, nothing
// thrown for data errors, and no filesystem access from the wasm side (this
// module reads the embedded artifact itself on Node):
// a failure is the same `{ ok: false, diagnostics }` document the native
// `candid-core` CLI prints, passed through byte-for-byte.

import initWasm, {
  checkCompatible as wasmCheckCompatible,
  didToContract as wasmDidToContract,
  didToModule as wasmDidToModule,
  projectDid as wasmProjectDid,
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

function sourcesOf(sources) {
  return typeof sources === "string" ? { source: sources } : sources;
}

function requestOf(sources) {
  return JSON.stringify(sourcesOf(sources));
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
 * Returns `{ ok: true, module, omitted }` with the generated text —
 * byte-identical to what the Rust-native generator emits — and the
 * declarations and methods it left out, or `{ ok: false, diagnostics }`.
 */
export async function didToModule(sources) {
  await init();
  return JSON.parse(wasmDidToModule(requestOf(sources)));
}

/**
 * Project Candid sources onto the methods named: a self-contained `.did`
 * holding exactly those methods of the service and every declaration they
 * reach, deterministic to the byte for the same sources and the same set of
 * names, whatever their order.
 *
 * Returns `{ ok: true, did, methods, input, projection }` — the text, the
 * methods it holds in output order, and the identities of the input and of
 * the projection — or `{ ok: false, diagnostics }`: `unknown_method` (one
 * per name the service lacks, its `notes` listing the service's methods),
 * `empty_method_list`, `no_service`, or the compiler's diagnostics.
 */
export async function projectDid(sources, methods) {
  await init();
  return JSON.parse(wasmProjectDid(JSON.stringify({ ...sourcesOf(sources), methods })));
}

/**
 * Check that a live interface is still compatible with a written one: every
 * method of the written service exists in the live one with the same mode,
 * and the live method's type is a Candid subtype of the written one.
 * Methods only the live service has are ignored.
 *
 * Returns `{ ok: true, compatible, written, live, diagnostics }`, where each
 * diagnostic names its method and, inside a type, the path where the check
 * failed; or `{ ok: false, input, diagnostics }` when a side does not
 * compile (`input` names it).
 */
export async function checkCompatible(written, live) {
  await init();
  return JSON.parse(
    wasmCheckCompatible(JSON.stringify({ written: sourcesOf(written), live: sourcesOf(live) })),
  );
}
