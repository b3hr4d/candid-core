//! The issue #153 parity and determinism gates, at the function level: the
//! same code the wasm artifact wraps runs on the host here, and its outputs
//! are compared byte-for-byte against the repository's reviewed artifacts —
//! the generator's golden `.ts` modules and the committed envelope fixture
//! the native `candid-core compile --envelope` binary produced. CI runs the
//! same comparisons again through the actual wasm build under Node, so the
//! "same code compiled twice" claim is asserted, not assumed.

use std::path::PathBuf;

use candid_core_wasm::{did_to_contract, did_to_module, FIELD_NAMES_EXTENSION};
use serde_json::Value;

fn repo(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn single(source: &str) -> String {
    serde_json::to_string(&serde_json::json!({ "source": source })).unwrap()
}

/// Every generator golden fixture must reproduce its reviewed `.ts` byte
/// for byte through this crate's module path.
#[test]
fn modules_match_the_generator_goldens() {
    for name in [
        "primitives",
        "collections",
        "variants",
        "recursion",
        "quoting",
        "deferred",
        "proto",
        "ledger",
        "empties",
        "arms",
    ] {
        let source = repo(&format!("crates/candid-core-ts/tests/fixtures/{name}.did"));
        let golden = repo(&format!("crates/candid-core-ts/tests/goldens/{name}.ts"));
        let response: Value = serde_json::from_str(&did_to_module(&single(&source))).unwrap();
        assert_eq!(response["ok"], Value::Bool(true), "{name}: {response}");
        assert_eq!(
            response["module"].as_str().unwrap(),
            golden,
            "{name}: the module must be byte-identical to the reviewed golden",
        );
    }
}

/// The envelope path must be byte-identical to the native binary's: the
/// committed fixture is real `candid-core compile <path> --envelope` output,
/// and this compares the entire document — producer included.
#[test]
fn envelope_matches_the_native_cli_fixture_byte_for_byte() {
    let source = repo("tests/fixtures/conformance/basic.did");
    let fixture = repo("tests/fixtures/envelope/basic.envelope.json");
    assert_eq!(
        did_to_contract(&single(&source)),
        fixture,
        "the wasm path and the native CLI path must emit identical bytes",
    );
}

/// The envelope triples equal the reviewed `*.names.json` goldens — the
/// same cross-surface equality the native CLI pins.
#[test]
fn envelope_field_names_match_the_generator_goldens() {
    for name in ["collections", "variants", "quoting", "proto", "ledger"] {
        let source = repo(&format!("crates/candid-core-ts/tests/fixtures/{name}.did"));
        let golden: Value = serde_json::from_str(&repo(&format!(
            "crates/candid-core-ts/tests/goldens/{name}.names.json"
        )))
        .unwrap();
        let envelope: Value = serde_json::from_str(&did_to_contract(&single(&source))).unwrap();
        assert_eq!(
            envelope["extensions"][FIELD_NAMES_EXTENSION], golden,
            "{name}: the envelope triples must equal the names.json golden",
        );
    }
}

/// Multi-file bundles resolve through the caller-supplied map — both request
/// spellings of the entry, bare and scheme-qualified.
#[test]
fn bundles_resolve_through_the_files_map() {
    for entry in ["entry.did", "memory:/entry.did"] {
        let request = serde_json::json!({
            "entry": entry,
            "files": {
                "entry.did": "import \"types.did\";\nservice : { get: () -> (Item) query };",
                "types.did": "type Item = record { id: nat };",
            },
        });
        let envelope: Value = serde_json::from_str(&did_to_contract(&request.to_string())).unwrap();
        assert!(
            envelope.get("contract").is_some(),
            "{entry}: the bundle must compile: {envelope}",
        );
        let module: Value = serde_json::from_str(&did_to_module(&request.to_string())).unwrap();
        assert_eq!(module["ok"], Value::Bool(true), "{entry}: {module}");
        assert!(module["module"]
            .as_str()
            .unwrap()
            .contains("export type Item"));
    }
}

/// Determinism: identical requests, byte-identical responses.
#[test]
fn responses_are_deterministic() {
    let source = repo("crates/candid-core-ts/tests/fixtures/ledger.did");
    let request = single(&source);
    assert_eq!(did_to_contract(&request), did_to_contract(&request));
    assert_eq!(did_to_module(&request), did_to_module(&request));
}

/// Compiler diagnostics pass through verbatim, in the native CLI's failure
/// shape, and fail closed.
#[test]
fn diagnostics_pass_through_verbatim() {
    // A parse error, from the compiler itself.
    let response: Value = serde_json::from_str(&did_to_contract(&single("service : {"))).unwrap();
    assert_eq!(response["ok"], Value::Bool(false));
    assert_eq!(response["diagnostics"][0]["code"], "did_parse_error");
    assert_eq!(response["diagnostics"][0]["severity"], "error");

    // A missing import in a bundle: the resolver's structured failure.
    let request = serde_json::json!({
        "entry": "entry.did",
        "files": { "entry.did": "import \"missing.did\";\nservice : {};" },
    });
    let response: Value = serde_json::from_str(&did_to_contract(&request.to_string())).unwrap();
    assert_eq!(response["ok"], Value::Bool(false), "{response}");

    // The generator's fail-closed refusals surface under a stable code with
    // the refusal text verbatim.
    let response: Value = serde_json::from_str(&did_to_module(&single("type c = nat8;"))).unwrap();
    assert_eq!(response["ok"], Value::Bool(false));
    assert_eq!(response["diagnostics"][0]["code"], "ts_generation_refused");
    assert_eq!(response["diagnostics"][0]["phase"], "generate");
}

/// Malformed requests fail closed with this crate's own stable code — never
/// a panic, never a half-answer.
#[test]
fn malformed_requests_fail_closed() {
    for request in [
        "not json",
        "[]",
        "{}",
        r#"{"source": 5}"#,
        r#"{"entry": "a.did"}"#,
        r#"{"files": {}}"#,
        r#"{"source": "service : {};", "entry": "a.did", "files": {}}"#,
        r#"{"typo": true}"#,
        r#"{"entry": "a.did", "files": {"a.did": 5}}"#,
    ] {
        let response: Value = serde_json::from_str(&did_to_contract(request)).unwrap();
        assert_eq!(response["ok"], Value::Bool(false), "{request}");
        assert_eq!(
            response["diagnostics"][0]["code"], "invalid_request",
            "{request}: {response}",
        );
    }
    // A wrong-scheme entry reaches the resolver, whose own structured
    // refusal passes through verbatim — the passthrough rule, not a request
    // error.
    let request = r#"{"entry": "https:/a.did", "files": {"a.did": "service : {};"}}"#;
    let response: Value = serde_json::from_str(&did_to_contract(request)).unwrap();
    assert_eq!(response["ok"], Value::Bool(false));
    assert_eq!(
        response["diagnostics"][0]["code"],
        "did_source_scheme_mismatch"
    );
}
