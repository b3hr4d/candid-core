//! The library half of `@candid-core/cli` (issue #153): `.did` sources in,
//! data out — an envelope JSON document or generated TypeScript text — with
//! no eval, no dynamic import, and no filesystem. The JS host does all I/O
//! and hands sources to [`did_to_contract`] / [`did_to_module`]; compilation
//! runs through `compile_with_resolver` + `MemoryResolver`, the documented
//! browser-WASM surface, so the same artifact serves the Node CLI and a
//! headless page.
//!
//! # The request/response convention
//!
//! Both functions take one JSON document and return one JSON document, so
//! the wasm ABI stays two strings wide and every richer shape lives in
//! reviewable JSON:
//!
//! - request: `{"source": "<did text>"}` for a self-contained source, or
//!   `{"entry": "<name>", "files": {"<name>": "<did text>", …}}` for a
//!   bundle resolved through `MemoryResolver` (names are `memory:/` source
//!   IDs; a bare name is prefixed automatically).
//! - [`did_to_contract`] success: the `ContractEnvelope` document itself —
//!   `{"contract": …, "extensions": {"org.candid-core.field-names/v1":
//!   [[container, id, name], …]}}` — byte-identical to what the native
//!   `candid-core compile <path> --envelope` binary prints for the same
//!   sources, pinned by test against the committed envelope fixture. The
//!   `contract` key is the same discriminator `schemaFromContract` detects.
//! - [`did_to_module`] success: `{"ok": true, "module": "<TypeScript>"}` —
//!   the text `candid-core-ts` generates, byte-identical to the reviewed
//!   goldens, pinned by test.
//! - failure, either function: `{"ok": false, "diagnostics": […]}` with
//!   candid-core's diagnostics passed through verbatim. Failures this crate
//!   itself originates use the same item shape with its own stable codes:
//!   `invalid_request` (the request document is not one of the two shapes)
//!   and `ts_generation_refused` (the generator's fail-closed refusals,
//!   message text verbatim), both under `"phase": "generate"` only for the
//!   latter and `"load"` for the former.
//!
//! # Determinism
//!
//! Identical requests produce byte-identical responses — the generator and
//! canonical serialization guarantee it, a test pins it, and the CLI on top
//! additionally double-runs every generation and refuses on any mismatch.

use candid_core::{
    compile_with_resolver, Compilation, CompileError, CompileOptions, ContractEnvelope, Limits,
    MemoryResolver, RuntimeContext, SourceInfo, SourceLabel,
};
use candid_core_ts::{generate_module, TsNames, TsOptions};
use serde_json::{json, Value};

/// The envelope extension carrying field names, per the issue #152 decision.
pub const FIELD_NAMES_EXTENSION: &str = "org.candid-core.field-names/v1";

/// Compile a request into a `ContractEnvelope` JSON document carrying the
/// field-names extension, or `{"ok": false, "diagnostics": […]}`.
pub fn did_to_contract(request: &str) -> String {
    match compile_request(request) {
        Ok(compilation) => envelope_document(compilation),
        Err(failure) => failure,
    }
}

/// Generate the TypeScript module for a request: `{"ok": true, "module": …}`
/// or `{"ok": false, "diagnostics": […]}`.
pub fn did_to_module(request: &str) -> String {
    match compile_request(request) {
        Ok(compilation) => {
            let source_info = compilation
                .source_info()
                .expect("source info was requested");
            let names = TsNames::from_source_info(source_info);
            match generate_module(compilation.contract(), &names, &TsOptions::default()) {
                Ok(module) => pretty(&json!({ "ok": true, "module": module })),
                Err(refusal) => pretty(&json!({
                    "ok": false,
                    "diagnostics": [{
                        "code": "ts_generation_refused",
                        "phase": "generate",
                        "severity": "error",
                        "message": refusal.to_string(),
                    }],
                })),
            }
        }
        Err(failure) => failure,
    }
}

/// Parse the request and compile it; failures come back pre-rendered in the
/// response convention so callers return them as-is.
fn compile_request(request: &str) -> Result<Compilation, String> {
    let (entry, resolver) = parse_request(request)?;
    compile_with_resolver(
        &entry,
        &resolver,
        CompileOptions {
            include_source_info: true,
        },
        &RuntimeContext::default(),
    )
    .map_err(|error| diagnostics_document(&error))
}

fn parse_request(request: &str) -> Result<(String, MemoryResolver), String> {
    let document: Value = serde_json::from_str(request)
        .map_err(|error| invalid_request(&format!("the request is not JSON: {error}")))?;
    let object = document
        .as_object()
        .ok_or_else(|| invalid_request("the request is a JSON object"))?;
    let unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !matches!(*key, "source" | "entry" | "files"))
        .collect();
    if !unknown.is_empty() {
        return Err(invalid_request(&format!(
            "unknown request keys: {}",
            unknown.join(", ")
        )));
    }
    match (
        object.get("source"),
        object.get("entry"),
        object.get("files"),
    ) {
        (Some(source), None, None) => {
            let source = source
                .as_str()
                .ok_or_else(|| invalid_request("source must be a string of Candid text"))?;
            let resolver = MemoryResolver::new()
                .with_source("memory:/service.did", source)
                .map_err(|error| invalid_request(&error.to_string()))?;
            Ok(("memory:/service.did".to_string(), resolver))
        }
        (None, Some(entry), Some(files)) => {
            let entry = entry
                .as_str()
                .ok_or_else(|| invalid_request("entry must be a string file name"))?;
            let files = files
                .as_object()
                .ok_or_else(|| invalid_request("files must map file names to Candid text"))?;
            let mut resolver = MemoryResolver::new();
            for (name, text) in files {
                let text = text.as_str().ok_or_else(|| {
                    invalid_request(&format!("files[{name:?}] must be a string of Candid text"))
                })?;
                resolver
                    .insert(scheme_qualified(name), text)
                    .map_err(|error| invalid_request(&format!("files[{name:?}]: {error}")))?;
            }
            Ok((scheme_qualified(entry), resolver))
        }
        _ => Err(invalid_request(
            "the request is {\"source\": …} or {\"entry\": …, \"files\": {…}}",
        )),
    }
}

/// A bare file name becomes a `memory:/` source ID; an already-qualified ID
/// passes through for `MemoryResolver` to validate.
fn scheme_qualified(name: &str) -> String {
    if name.contains(":/") {
        name.to_string()
    } else {
        format!("memory:/{name}")
    }
}

/// The envelope document, built exactly as `candid-core compile --envelope`
/// builds it: the same triples derivation as the generator's `*.names.json`
/// goldens (named labels only, sorted, deduplicated), inserted through the
/// real `ContractEnvelope` so the extension passes envelope validation.
fn envelope_document(compilation: Compilation) -> String {
    let (contract, source_info) = compilation.into_parts();
    let source_info = source_info.expect("source info was requested");
    let mut envelope = ContractEnvelope::new(contract);
    match envelope.insert_extension(
        FIELD_NAMES_EXTENSION,
        Value::Array(field_name_triples(&source_info)),
        &Limits::default(),
    ) {
        Ok(()) => pretty(&serde_json::to_value(&envelope).expect("JSON values serialize")),
        Err(error) => pretty(&json!({ "ok": false, "violations": error.violations })),
    }
}

/// Named field labels as `[container, id, name]` triples — the
/// `org.candid-core.field-names/v1` value, derived exactly as the golden
/// pipeline and the native binary derive it.
fn field_name_triples(source_info: &SourceInfo) -> Vec<Value> {
    let mut triples: Vec<(u32, u32, &str)> = source_info
        .field_labels()
        .iter()
        .filter_map(|provenance| match &provenance.label {
            SourceLabel::Named { name } => {
                Some((provenance.container, provenance.id, name.as_str()))
            }
            _ => None,
        })
        .collect();
    triples.sort();
    triples.dedup();
    triples
        .iter()
        .map(|(container, id, label)| json!([container, id, label]))
        .collect()
}

fn diagnostics_document(error: &CompileError) -> String {
    pretty(&json!({ "ok": false, "diagnostics": error.diagnostics }))
}

fn invalid_request(message: &str) -> String {
    pretty(&json!({
        "ok": false,
        "diagnostics": [{
            "code": "invalid_request",
            "phase": "load",
            "severity": "error",
            "message": message,
        }],
    }))
}

/// One pretty-printed JSON document with a trailing newline — the native
/// CLI's exact output convention, which is what makes byte-identity with it
/// possible.
fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("JSON values serialize");
    text.push('\n');
    text
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use wasm_bindgen::prelude::wasm_bindgen;

    /// See [`crate::did_to_contract`].
    #[wasm_bindgen(js_name = didToContract)]
    pub fn did_to_contract(request: &str) -> String {
        crate::did_to_contract(request)
    }

    /// See [`crate::did_to_module`].
    #[wasm_bindgen(js_name = didToModule)]
    pub fn did_to_module(request: &str) -> String {
        crate::did_to_module(request)
    }
}
