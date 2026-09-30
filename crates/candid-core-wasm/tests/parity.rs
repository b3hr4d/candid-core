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
/// for byte through this crate's module path, and report the native
/// generator's omitted list — the `<name>.omitted.json` golden
/// `crates/candid-core-ts/tests/golden.rs` writes from it — or an empty list
/// for a fixture that omits nothing (issue #189).
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
        "options",
        "shadowing",
        "fidelity",
        "docs",
        "omissions",
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
        let omitted = if name == "omissions" {
            serde_json::from_str(&repo(&format!(
                "crates/candid-core-ts/tests/goldens/{name}.omitted.json"
            )))
            .unwrap()
        } else {
            Value::Array(Vec::new())
        };
        assert_eq!(
            response["omitted"], omitted,
            "{name}: the omitted list must equal the native generator's",
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

/// A Windows-saved source — the Node host reads files with `"utf8"`, which
/// keeps a leading BOM as U+FEFF — yields the same envelope and module as
/// its BOM-less twin, through both request shapes.
#[test]
fn a_leading_utf8_bom_changes_neither_envelope_nor_module() {
    let source = repo("tests/fixtures/conformance/basic.did");
    let marked = format!("\u{FEFF}{source}");
    assert_eq!(
        did_to_contract(&single(&marked)),
        repo("tests/fixtures/envelope/basic.envelope.json"),
    );
    assert_eq!(
        did_to_module(&single(&marked)),
        did_to_module(&single(&source))
    );

    let bundle = |entry: &str, types: &str| {
        serde_json::json!({
            "entry": "entry.did",
            "files": { "entry.did": entry, "types.did": types },
        })
        .to_string()
    };
    let entry = "import \"types.did\";\nservice : { get: () -> (Item) query };";
    let types = "type Item = record { id: nat };";
    let plain = bundle(entry, types);
    let marked = bundle(&format!("\u{FEFF}{entry}"), &format!("\u{FEFF}{types}"));
    assert!(did_to_contract(&plain).contains("\"contract\""));
    assert_eq!(did_to_contract(&marked), did_to_contract(&plain));
    assert_eq!(did_to_module(&marked), did_to_module(&plain));
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
            .contains("export { $Item as Item };"));
    }
}

/// Hash-colliding spellings collapse to the exact key the generated module
/// renders — `cemxzwyk` and `amxawvks` share a Candid label hash, so their
/// structurally identical records deduplicate to one semantic node with two
/// provenance spellings, and the table must carry the one the generator
/// keeps (PR #159 review, mirrored here).
#[test]
fn colliding_spellings_collapse_to_the_generated_key() {
    let request =
        single("type A = record { cemxzwyk : nat };\ntype B = record { amxawvks : nat };");
    let module: Value = serde_json::from_str(&did_to_module(&request)).unwrap();
    assert_eq!(module["ok"], Value::Bool(true), "{module}");
    let envelope: Value = serde_json::from_str(&did_to_contract(&request)).unwrap();
    let triples = envelope["extensions"][FIELD_NAMES_EXTENSION]
        .as_array()
        .unwrap();
    assert_eq!(
        triples.len(),
        1,
        "one entry per (container, id): {triples:?}"
    );
    let winner = triples[0][2].as_str().unwrap();
    assert!(
        module["module"]
            .as_str()
            .unwrap()
            .contains(&format!("{winner}: $.c.nat")),
        "the module must render the same spelling the table carries: {winner:?}",
    );
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

    // Since issue #189 no Candid source reaches a generator refusal: a
    // declaration the module cannot represent — here a source name shaped
    // like the `_N_` id rendering (issues #103, #115) — is omitted, with
    // what references it, and the module is a success that says so.
    // `ts_generation_refused` is kept for an invalid Contract graph, which a
    // compiled Contract never is.
    let response: Value = serde_json::from_str(&did_to_module(&single(
        "type R = record { _0_ : nat };\ntype Uses = vec R;\ntype Fine = nat;\n\
         service : { use_it : (R) -> (); fine : (Fine) -> () }",
    )))
    .unwrap();
    assert_eq!(response["ok"], Value::Bool(true), "{response}");
    assert_eq!(
        response["omitted"],
        serde_json::json!([
            { "kind": "declaration", "name": "R", "reason": "reserved_field_name" },
            { "kind": "declaration", "name": "Uses", "reason": "references_omitted", "via": "R" },
            { "kind": "method", "name": "use_it", "reason": "references_omitted", "via": "R" },
        ]),
    );
    let module = response["module"].as_str().unwrap();
    assert!(
        module.contains("// Omitted: type R (reserved_field_name)\n"),
        "{module}"
    );
    assert!(module.contains("export { $Fine as Fine };"), "{module}");
    assert!(
        module.contains("  fine: (arg0: bigint) => Promise<void>;"),
        "{module}"
    );
    // `via` is absent, not null, when the reason carries none.
    assert!(response["omitted"][0].get("via").is_none());

    // A declaration named after one of the module's former bindings
    // generates since issue #188: it binds as a `$`-prefixed local and is
    // exported under its Candid name.
    let response: Value = serde_json::from_str(&did_to_module(&single("type c = nat8;"))).unwrap();
    assert_eq!(response["ok"], Value::Bool(true), "{response}");
    assert!(response["module"]
        .as_str()
        .unwrap()
        .contains("export { $c as c };"));
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
