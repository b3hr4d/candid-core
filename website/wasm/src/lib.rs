//! The browser bridge behind the multi-version wallet demo.
//!
//! Every identity the page displays is computed here, in WebAssembly, from DID
//! source the page holds in memory. Nothing is precomputed at build time and no
//! server is involved: the page fetches `.did` files, hands them to
//! [`candid_core::compile_with_resolver`] through a [`MemoryResolver`], and
//! renders whatever comes back.
//!
//! # Why there is no `wasm-bindgen` here
//!
//! `candid-core` deliberately takes no `wasm-bindgen`, `js-sys`, or `web-time`
//! production dependency, and a demo that reached for one to talk to the
//! browser would quietly weaken that claim. So the ABI is four `extern "C"`
//! functions over linear memory and one JSON document in each direction, and
//! the JavaScript side is 40 lines of hand-written glue. The consequence worth
//! stating: the `.wasm` this crate produces is the library plus this file, with
//! no binding-generator runtime linked in.
//!
//! # ABI
//!
//! | Export | Meaning |
//! | --- | --- |
//! | `cc_alloc(len) -> ptr` | Reserve `len` bytes the caller may write into |
//! | `cc_free(ptr, len)` | Release a region from `cc_alloc` or an out-parameter |
//! | `cc_compile(ptr, len, out)` | Compile the UTF-8 JSON request at `ptr[..len]` |
//! | `cc_producer(out)` | Emit [`ProducerInfo::current`] as JSON |
//!
//! `out` points at 8 writable bytes that receive two little-endian `u32`s: the
//! response pointer and its length. The response is always UTF-8 JSON, and the
//! caller owns it — pass the same pair back to `cc_free`.
//!
//! A request is `{"entry": "memory:/…", "sources": {"memory:/…": "…"}, …}`.
//! A response is `{"ok": true, …}` or `{"ok": false, …}`; the failure shapes
//! mirror the `candid-core` CLI, so the same `diagnostics` codes a terminal
//! user sees are the ones the page renders.

use candid_core::{
    artifact_id_with_limits, compile_with_resolver, ArtifactKind, CompileOptions, Limits,
    MemoryResolver, ProducerInfo, RuntimeContext,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// One compilation request from the page.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    /// The logical source ID to start from. Must be one of `sources`' keys.
    entry: String,
    /// The whole bundle, keyed by logical source ID. Imports are resolved
    /// against this map and nowhere else.
    sources: BTreeMap<String, String>,
    /// Whether to ask for the provenance sidecar. The page turns this off to
    /// show what a Contract-only consumer receives.
    #[serde(default = "enabled")]
    source_info: bool,
}

fn enabled() -> bool {
    true
}

/// Reserves `len` bytes of linear memory for the caller to write into.
///
/// Returns a dangling-but-aligned pointer for `len == 0`, which the JavaScript
/// side never dereferences.
#[no_mangle]
pub extern "C" fn cc_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Releases a region previously handed out by [`cc_alloc`] or written into an
/// out-parameter by [`cc_compile`] or [`cc_producer`].
///
/// # Safety
///
/// `ptr`/`len` must be a pair this module produced and must not be freed twice.
#[no_mangle]
pub unsafe extern "C" fn cc_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    drop(Vec::from_raw_parts(ptr, len, len));
}

/// Compiles one in-memory DID bundle and writes a JSON response.
///
/// # Safety
///
/// `ptr[..len]` must be readable and `out[..8]` writable, both inside this
/// module's linear memory.
#[no_mangle]
pub unsafe extern "C" fn cc_compile(ptr: *const u8, len: usize, out: *mut u8) {
    let request = std::slice::from_raw_parts(ptr, len);
    emit(compile(request), out);
}

/// Writes the runtime's own provenance — the `candid-core` version and the
/// exact Candid crates it delegates to — as JSON.
///
/// # Safety
///
/// `out[..8]` must be writable inside this module's linear memory.
#[no_mangle]
pub unsafe extern "C" fn cc_producer(out: *mut u8) {
    emit(
        json!({ "ok": true, "producer": ProducerInfo::current() }),
        out,
    );
}

/// Hands `value` to the caller as an owned UTF-8 JSON region, recorded in `out`
/// as two little-endian `u32`s.
///
/// # Safety
///
/// `out[..8]` must be writable inside this module's linear memory.
unsafe fn emit(value: Value, out: *mut u8) {
    // Serializing a `Value` cannot fail: it holds no map with non-string keys
    // and no non-finite float. The fallback keeps the ABI total anyway, since
    // an aborting panic here would take the page's module down with it.
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| {
        br#"{"ok":false,"error":{"code":"response_encoding_failed"}}"#.to_vec()
    });
    let len = body.len();
    let pointer = Box::into_raw(body.into_boxed_slice()) as *mut u8;
    let header = std::slice::from_raw_parts_mut(out, 8);
    header[..4].copy_from_slice(&(pointer as u32).to_le_bytes());
    header[4..].copy_from_slice(&(len as u32).to_le_bytes());
}

/// The whole demo, minus rendering: parse the request, build the hermetic
/// bundle, compile it, and describe the result.
fn compile(request: &[u8]) -> Value {
    let request: Request = match serde_json::from_slice(request) {
        Ok(request) => request,
        Err(error) => return malformed(&error.to_string()),
    };
    if !request.sources.contains_key(&request.entry) {
        return malformed(&format!(
            "entry {} is not one of the supplied sources",
            request.entry
        ));
    }

    let mut resolver = MemoryResolver::new();
    for (id, source) in &request.sources {
        if let Err(error) = resolver.insert(id, source.clone()) {
            return json!({
                "ok": false,
                "diagnostics": [{
                    "code": error.code,
                    "phase": "load",
                    "severity": "error",
                    "message": error.message,
                    "resource_limit": error.resource_limit,
                }],
            });
        }
    }

    let compilation = match compile_with_resolver(
        &request.entry,
        &resolver,
        CompileOptions {
            include_source_info: request.source_info,
        },
        &RuntimeContext::default(),
    ) {
        Ok(compilation) => compilation,
        // The same shape the CLI prints, so the page can render compiler
        // failures without a second code path.
        Err(error) => return json!({ "ok": false, "diagnostics": error.diagnostics }),
    };

    let limits = Limits::default();
    let (contract, source_info) = compilation.into_parts();
    // The document, not a re-encoding of it: `artifact_id` covers exactly the
    // octets below, so the page has to receive those octets to be able to
    // claim anything about them. Everything the UI renders is parsed back out
    // of this one string.
    let document = match contract.to_json_pretty_with_limits(&limits) {
        Ok(document) => document,
        Err(error) => return json!({ "ok": false, "violations": error.violations }),
    };
    let artifact_id =
        match artifact_id_with_limits(ArtifactKind::ContractJsonV1, document.as_bytes(), &limits) {
            Ok(id) => id,
            Err(error) => return json!({ "ok": false, "violations": error.violations }),
        };

    json!({
        "ok": true,
        "contract_document": document,
        "artifact_id": artifact_id,
        "artifact_kind": "ContractJsonV1",
        "artifact_bytes": document.len(),
        "source_info": source_info,
    })
}

/// A request this bridge could not even read as a request. Reported in the
/// `diagnostics` shape so the page has one failure renderer, not two.
fn malformed(message: &str) -> Value {
    json!({
        "ok": false,
        "diagnostics": [{
            "code": "malformed_request",
            "phase": "load",
            "severity": "error",
            "message": message,
        }],
    })
}
