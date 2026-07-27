// The JavaScript half of the raw ABI exported by `website/wasm`.
//
// There is no generated glue here and no binding-generator runtime in the
// module this talks to. Two calls into linear memory, one JSON document each
// way. See `website/wasm/src/lib.rs` for the other side and for why the demo
// is built this way.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Instantiates the candid-core bridge module.
 *
 * @param {string | ArrayBuffer | Uint8Array} source A URL to fetch, or the
 *   module bytes if the caller already has them (the Node verifier does).
 * @returns {Promise<{compile: Function, producer: Function}>}
 */
export async function loadCandidCore(source) {
  let bytes = source;
  if (typeof source === "string") {
    const response = await fetch(source);
    if (!response.ok) {
      throw new Error(`cannot fetch ${source}: ${response.status}`);
    }
    bytes = await response.arrayBuffer();
  }
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const wasm = instance.exports;

  // Every view has to be taken *after* the last call that could have grown
  // memory: growth detaches the old ArrayBuffer and silently zero-length
  // views are the classic way to lose an afternoon here.
  const view = () => new Uint8Array(wasm.memory.buffer);

  /** Runs one export that writes a `(pointer, length)` pair into `out`. */
  function call(run) {
    const out = wasm.cc_alloc(8);
    try {
      run(out);
      const header = new DataView(wasm.memory.buffer, out, 8);
      const pointer = header.getUint32(0, true);
      const length = header.getUint32(4, true);
      const body = decoder.decode(view().subarray(pointer, pointer + length));
      wasm.cc_free(pointer, length);
      return JSON.parse(body);
    } finally {
      wasm.cc_free(out, 8);
    }
  }

  return {
    /**
     * Compiles one in-memory bundle.
     *
     * @param {{entry: string, sources: Record<string, string>, source_info?: boolean}} request
     */
    compile(request) {
      const body = encoder.encode(JSON.stringify(request));
      const pointer = wasm.cc_alloc(body.length);
      view().set(body, pointer);
      try {
        return call((out) => wasm.cc_compile(pointer, body.length, out));
      } finally {
        wasm.cc_free(pointer, body.length);
      }
    },

    /** The runtime's own version and the Candid crates it delegates to. */
    producer() {
      return call((out) => wasm.cc_producer(out)).producer;
    },
  };
}
