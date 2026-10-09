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
//! - [`did_to_module`] success: `{"ok": true, "module": "<TypeScript>",
//!   "omitted": […]}` — the text `candid-core-ts` generates, byte-identical
//!   to the reviewed goldens, pinned by test, and what it left out (issue
//!   #189). Each `omitted` entry is `{"kind", "name", "reason", "via"?}`:
//!   `kind` is `declaration` or `method` (a method of the actor's service);
//!   `reason` is one of the generator's closed set of codes —
//!   `reserved_field_name`, `ambiguous_variant_arm`, `reserved_export_name`,
//!   `invalid_declaration_name`, `references_omitted`; and `via`, present
//!   only for `references_omitted`, names the omitted declaration the entry
//!   references. Declarations come first, then methods, each sorted by name.
//!   The array is empty, never absent, when nothing was omitted. A module
//!   with omissions is still a success: everything it emits is exactly what
//!   it would be without the omitted declarations.
//! - failure, either function: `{"ok": false, "diagnostics": […]}` — the
//!   one and only failure shape. Compiler diagnostics pass through verbatim;
//!   envelope-validation refusals surface as their path-addressed violation
//!   items under the same key (the native binary's channel, aligned in
//!   review); and the two codes this crate itself originates are
//!   `invalid_request` (`"phase": "load"` — the request document is not one
//!   of the two shapes) and `ts_generation_refused`
//!   (`"phase": "generate"` — the generator's refusal of an invalid
//!   Contract graph, message text verbatim; a Contract compiled from
//!   Candid source is always valid, so since issue #189 this is a
//!   fail-closed guard that no `.did` input reaches).
//!
//! # Determinism
//!
//! Identical requests produce byte-identical responses — the generator and
//! canonical serialization guarantee it, a test pins it, and the CLI on top
//! additionally double-runs every generation and refuses on any mismatch.

use std::collections::BTreeSet;

use candid_core::{
    compile_did_with_options, compile_with_resolver, Compilation, CompileError, CompileOptions,
    ContractEnvelope, Limits, MemoryResolver, RuntimeContext, SourceInfo, SourceLabel,
};
use candid_core_ts::{generate_module, Omission, TsNames, TsOptions};
use serde_json::{json, Map, Value};

mod compat;
mod project;

pub use compat::{MAX_CHECK_DEPTH, MAX_CHECK_STEPS, MAX_CHECK_WARNINGS};

#[doc(hidden)]
pub use compat::CheckOptions;

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

/// Generate the TypeScript module for a request: `{"ok": true, "module": …,
/// "omitted": […]}` or `{"ok": false, "diagnostics": […]}`.
pub fn did_to_module(request: &str) -> String {
    match compile_request(request) {
        Ok(compilation) => {
            let source_info = compilation
                .source_info()
                .expect("source info was requested");
            let names = TsNames::from_source_info(source_info);
            match generate_module(compilation.contract(), &names, &TsOptions::default()) {
                Ok(generated) => pretty(&json!({
                    "ok": true,
                    "module": generated.module,
                    "omitted": omitted_document(&generated.omitted),
                })),
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
    compile(&entry, &resolver).map_err(|error| diagnostics_document(&error))
}

fn compile(entry: &str, resolver: &MemoryResolver) -> Result<Compilation, CompileError> {
    compile_with_resolver(
        entry,
        resolver,
        CompileOptions {
            include_source_info: true,
        },
        &RuntimeContext::default(),
    )
}

/// Project a request onto the methods it names: `{"ok": true, "did": …,
/// "methods": […], "input": {…}, "projection": {…}}` or `{"ok": false,
/// "diagnostics": […]}`. The request is a sources request plus `"methods":
/// ["name", …]`.
pub fn project_did(request: &str) -> String {
    match project_request(request) {
        Ok(response) | Err(response) => response,
    }
}

fn project_request(request: &str) -> Result<String, String> {
    let document = request_object(request)?;
    let requested: Vec<&str> = match document.get("methods") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| invalid_request("methods must be an array of method names"))
            })
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(invalid_request(
                "the request must carry \"methods\": an array of method names",
            ))
        }
    };
    let (entry, resolver) = parse_sources(&document, &["methods"])?;
    let compilation = compile(&entry, &resolver).map_err(|error| diagnostics_document(&error))?;
    let contract = compilation.contract();
    let Some(service) = compat::actor_methods(contract) else {
        return Err(failure(vec![phase_diagnostic(
            "no_service",
            "project",
            "the source declares no service, so there are no methods to project".to_string(),
        )]));
    };
    if requested.is_empty() {
        return Err(failure(vec![phase_diagnostic(
            "empty_method_list",
            "project",
            "name at least one method to project".to_string(),
        )]));
    }
    let mut available: Vec<&str> = service.iter().map(|(name, _)| name.as_str()).collect();
    available.sort_unstable();
    let mut unknown = Vec::new();
    let mut seen = BTreeSet::new();
    for name in &requested {
        if !seen.insert(*name) {
            continue;
        }
        if !available.contains(name) {
            let mut item = phase_diagnostic(
                "unknown_method",
                "project",
                format!(
                    "the service has no method {name:?}; its methods are: {}",
                    available.join(", ")
                ),
            );
            item["notes"] = json!(available);
            unknown.push(item);
        }
    }
    if !unknown.is_empty() {
        return Err(failure(unknown));
    }
    let wanted: BTreeSet<String> = seen.into_iter().map(str::to_string).collect();
    let source_info = compilation
        .source_info()
        .expect("source info was requested");
    let (text, methods) = project::project(source_info, &entry, &wanted)
        .map_err(|project::Internal(message)| projection_failed(message, Vec::new()))?;
    // The output is only as good as a second compile says it is: it must be
    // a valid self-contained `.did` whose service has exactly these methods.
    let projected = compile_did_with_options(
        &text,
        CompileOptions {
            include_source_info: false,
        },
    )
    .map_err(|error| {
        projection_failed(
            "the projection does not compile".to_string(),
            error
                .diagnostics
                .iter()
                .map(|item| format!("{}: {}", item.code, item.message))
                .collect(),
        )
    })?;
    let mut kept: Vec<String> = compat::actor_methods(projected.contract())
        .unwrap_or_default()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    kept.sort();
    if kept != wanted.iter().cloned().collect::<Vec<_>>() {
        return Err(projection_failed(
            format!("the projection holds the methods {kept:?}, not the ones requested"),
            Vec::new(),
        ));
    }
    Ok(pretty(&json!({
        "ok": true,
        "did": text,
        "methods": methods,
        "input": identities(contract),
        "projection": identities(projected.contract()),
    })))
}

/// Check that a live service is a Candid subtype of a written one:
/// `{"ok": true, "compatible": …, "written": {…}, "live": {…},
/// "diagnostics": […]}`, or `{"ok": false, "input"?: "written" | "live",
/// "diagnostics": […]}` when the request or a side's sources fail. The
/// request is `{"written": <sources request>, "live": <sources request>}`.
pub fn check_compatible(request: &str) -> String {
    check_compatible_with(request, CheckOptions::default())
}

/// [`check_compatible`] under other options: this crate's tests lower the
/// work bound, or turn off the re-reporting of proven pairs to get the
/// reference set of warnings.
#[doc(hidden)]
pub fn check_compatible_with(request: &str, options: CheckOptions) -> String {
    match check_request(request, options) {
        Ok(response) | Err(response) => response,
    }
}

fn check_request(request: &str, options: CheckOptions) -> Result<String, String> {
    let document = request_object(request)?;
    let unknown: Vec<&str> = document
        .keys()
        .map(String::as_str)
        .filter(|key| !matches!(*key, "written" | "live"))
        .collect();
    if !unknown.is_empty() {
        return Err(invalid_request(&format!(
            "unknown request keys: {}",
            unknown.join(", ")
        )));
    }
    let side = |name: &str| -> Result<Compilation, String> {
        let Some(Value::Object(object)) = document.get(name) else {
            return Err(invalid_request(&format!(
                "the request must carry \"{name}\": a sources request object"
            )));
        };
        let (entry, resolver) = parse_sources(object, &[])?;
        let compilation = compile(&entry, &resolver)
            .map_err(|error| side_failure(name, json!(error.diagnostics)))?;
        if compat::actor_methods(compilation.contract()).is_none() {
            return Err(side_failure(
                name,
                json!([phase_diagnostic(
                    "no_service",
                    "check",
                    format!("the {name} source declares no service to compare"),
                )]),
            ));
        }
        Ok(compilation)
    };
    let written = side("written")?;
    let live = side("live")?;
    let diagnostics = compat::check(
        &compat::Input {
            contract: written.contract(),
            source_info: written.source_info(),
        },
        &compat::Input {
            contract: live.contract(),
            source_info: live.source_info(),
        },
        options,
    );
    let compatible = diagnostics
        .iter()
        .all(|item| item["severity"] != json!("error"));
    Ok(pretty(&json!({
        "ok": true,
        "compatible": compatible,
        "written": identities(written.contract()),
        "live": identities(live.contract()),
        "diagnostics": diagnostics,
    })))
}

fn identities(contract: &candid_core::Contract) -> Value {
    json!({
        "contract_id": contract.contract_id(),
        "interface_id": contract.interface_id(),
    })
}

fn phase_diagnostic(code: &str, phase: &str, message: String) -> Value {
    json!({
        "code": code,
        "phase": phase,
        "severity": "error",
        "message": message,
    })
}

fn failure(diagnostics: Vec<Value>) -> String {
    pretty(&json!({ "ok": false, "diagnostics": diagnostics }))
}

fn side_failure(side: &str, diagnostics: Value) -> String {
    pretty(&json!({ "ok": false, "input": side, "diagnostics": diagnostics }))
}

/// A projection the compiler refused or that lost a method: never expected,
/// and reported rather than written.
fn projection_failed(message: String, notes: Vec<String>) -> String {
    let mut item = phase_diagnostic("projection_failed", "project", message);
    if !notes.is_empty() {
        item["notes"] = json!(notes);
    }
    failure(vec![item])
}

fn parse_request(request: &str) -> Result<(String, MemoryResolver), String> {
    let document = request_object(request)?;
    parse_sources(&document, &[])
}

/// The request document as a JSON object, or the `invalid_request` failure.
fn request_object(request: &str) -> Result<Map<String, Value>, String> {
    let document: Value = serde_json::from_str(request)
        .map_err(|error| invalid_request(&format!("the request is not JSON: {error}")))?;
    match document {
        Value::Object(object) => Ok(object),
        _ => Err(invalid_request("the request must be a JSON object")),
    }
}

/// The sources half of a request: `{"source": …}` or `{"entry": …,
/// "files": {…}}`, beside only the `extra` keys the caller reads itself.
fn parse_sources(
    object: &Map<String, Value>,
    extra: &[&str],
) -> Result<(String, MemoryResolver), String> {
    let unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !matches!(*key, "source" | "entry" | "files") && !extra.contains(key))
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
            "the request must be {\"source\": …} or {\"entry\": …, \"files\": {…}}",
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
        // One failure channel, same as the native binary since its review:
        // `ContractViolation` is an alias of `Diagnostic`, so the validation
        // items land under `diagnostics` unchanged rather than introducing a
        // second failure shape.
        Err(error) => pretty(&json!({ "ok": false, "diagnostics": error.violations })),
    }
}

/// Named field labels as `[container, id, name]` triples — the
/// `org.candid-core.field-names/v1` value, derived exactly as the golden
/// pipeline and the native binary derive it: one name per `(container, id)`,
/// retained with `TsNames::from_source_info`'s own overwrite order (later
/// provenance wins), so a hash-colliding pair of spellings can never leave
/// the loader a different key than the generated module renders.
fn field_name_triples(source_info: &SourceInfo) -> Vec<Value> {
    let mut names: std::collections::BTreeMap<(u32, u32), &str> = std::collections::BTreeMap::new();
    for provenance in source_info.field_labels() {
        if let SourceLabel::Named { name } = &provenance.label {
            names.insert((provenance.container, provenance.id), name.as_str());
        }
    }
    names
        .iter()
        .map(|((container, id), label)| json!([container, id, label]))
        .collect()
}

/// The generator's omissions as `ModuleSuccess.omitted` serializes them:
/// `{"kind", "name", "reason", "via"?}` with the generator's stable codes,
/// in the generator's order. `via` is present only for `references_omitted`.
fn omitted_document(omitted: &[Omission]) -> Value {
    Value::Array(
        omitted
            .iter()
            .map(|omission| {
                let mut entry = json!({
                    "kind": omission.kind.code(),
                    "name": omission.name,
                    "reason": omission.reason.code(),
                });
                if let Some(via) = &omission.via {
                    entry["via"] = json!(via);
                }
                entry
            })
            .collect(),
    )
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

    /// See [`crate::project_did`].
    #[wasm_bindgen(js_name = projectDid)]
    pub fn project_did(request: &str) -> String {
        crate::project_did(request)
    }

    /// See [`crate::check_compatible`].
    #[wasm_bindgen(js_name = checkCompatible)]
    pub fn check_compatible(request: &str) -> String {
        crate::check_compatible(request)
    }
}
