//! `projectDid`: a `.did` holding exactly the named methods and every type
//! they reach, deterministic to the byte, through the same entry point the
//! wasm artifact wraps.

use std::path::PathBuf;

use candid_core_wasm::{did_to_contract, did_to_module, project_did};
use serde_json::{json, Value};

fn repo(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn fixture(name: &str) -> String {
    repo(&format!("crates/candid-core-ts/tests/fixtures/{name}.did"))
}

fn project(sources: Value, methods: &[&str]) -> Value {
    let mut request = match sources {
        Value::String(source) => json!({ "source": source }),
        other => other,
    };
    request["methods"] = json!(methods);
    serde_json::from_str(&project_did(&request.to_string())).unwrap()
}

fn single(source: &str) -> String {
    json!({ "source": source }).to_string()
}

fn contract(source: &str) -> Value {
    serde_json::from_str(&did_to_contract(&single(source))).unwrap()
}

/// `(name, mode)` of every actor method, sorted by name.
fn actor_modes(envelope: &Value) -> Vec<(String, String)> {
    let contract = &envelope["contract"];
    let types = contract["types"].as_array().unwrap();
    let mut service = &types[contract["actor"]["service"]
        .as_u64()
        .or_else(|| {
            let class = &types[contract["actor"]["class"].as_u64().unwrap() as usize];
            class["service"].as_u64()
        })
        .unwrap() as usize];
    if service["kind"] == "class" {
        service = &types[service["service"].as_u64().unwrap() as usize];
    }
    let mut modes: Vec<(String, String)> = service["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|method| {
            let function = &types[method["function"].as_u64().unwrap() as usize];
            (
                method["name"].as_str().unwrap().to_string(),
                function["mode"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    modes.sort();
    modes
}

const FIXTURES_WITH_A_SERVICE: &[&str] = &[
    "deferred",
    "fidelity",
    "docs",
    "ledger",
    "options",
    "shadowing",
    "omissions",
];

/// Every generator fixture, service or not.
const FIXTURES: &[&str] = &[
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
];

/// The printer is faithful. Each generator fixture gets one extra method
/// whose arguments name every declaration (and a service holding just that
/// method when it has none), so that a projection of all methods drops
/// nothing; it must then keep the Contract identity, and its generated
/// module must be byte-identical to the full one — declaration names, doc
/// comments, argument names, labels and quoting included.
#[test]
fn projecting_every_method_reprints_the_source_faithfully() {
    for name in FIXTURES {
        let mut fixture_source = fixture(name);
        if !fixture_source.contains("service : {") {
            fixture_source.push_str("\nservice : {\n}\n");
        }
        let declarations: Vec<String> = contract(&fixture_source)["contract"]["declarations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|declaration| declaration["name"].as_str().unwrap().to_string())
            .collect();
        let at = fixture_source.find("service : {").expect("a service block") + "service : {".len();
        let source = format!(
            "{}\n  reach_all : ({}) -> ();{}",
            &fixture_source[..at],
            declarations.join(", "),
            &fixture_source[at..]
        );
        let methods: Vec<String> = actor_modes(&contract(&source))
            .into_iter()
            .map(|(method, _)| method)
            .collect();
        let names: Vec<&str> = methods.iter().map(String::as_str).collect();
        let response = project(json!(source), &names);
        assert_eq!(response["ok"], json!(true), "{name}: {response}");
        assert_eq!(
            response["projection"], response["input"],
            "{name}: identities moved"
        );
        let projected = response["did"].as_str().unwrap();
        let before: Value = serde_json::from_str(&did_to_module(&single(&source))).unwrap();
        let after: Value = serde_json::from_str(&did_to_module(&single(projected))).unwrap();
        assert_eq!(after, before, "{name}: the module changed\n{projected}");
    }
}

/// Without the extra method, projecting every method still keeps the
/// interface identity; only declarations no method reaches are dropped.
#[test]
fn projecting_every_method_keeps_the_interface() {
    for name in FIXTURES_WITH_A_SERVICE {
        let source = fixture(name);
        let methods: Vec<String> = actor_modes(&contract(&source))
            .into_iter()
            .map(|(method, _)| method)
            .collect();
        let names: Vec<&str> = methods.iter().map(String::as_str).collect();
        let response = project(json!(source), &names);
        assert_eq!(
            response["projection"]["interface_id"], response["input"]["interface_id"],
            "{name}: projecting every method must keep the interface"
        );
    }
}

/// The ledger cut to two methods, pinned as a reviewed golden: exactly the
/// two methods and the declarations they reach.
#[test]
fn a_projection_holds_exactly_the_named_methods_and_what_they_reach() {
    let source = fixture("ledger");
    let response = project(json!(source), &["transfer", "balance_of"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["methods"], json!(["balance_of", "transfer"]));
    let text = response["did"].as_str().unwrap();
    let golden_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/ledger.projection.did");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, text).unwrap();
    }
    assert_eq!(
        text,
        std::fs::read_to_string(&golden_path).unwrap(),
        "regenerate with UPDATE_GOLDENS=1 and review the diff"
    );
    for id in ["input", "projection"] {
        for key in ["contract_id", "interface_id"] {
            assert!(response[id][key]
                .as_str()
                .is_some_and(|value| value.starts_with("candid-core:")));
        }
    }
    assert_ne!(
        response["input"]["interface_id"],
        response["projection"]["interface_id"]
    );
    assert_eq!(
        response["input"]["interface_id"],
        contract(&source)["contract"]["identities"]["interface"]
    );
    assert_eq!(
        response["projection"]["interface_id"],
        contract(text)["contract"]["identities"]["interface"]
    );
}

/// `gen` on a projection: the module's `Actor` lists only the projected
/// methods, and each keeps its mode.
#[test]
fn a_projected_module_lists_only_the_projected_methods_with_their_modes() {
    let source = fixture("ledger");
    let all = actor_modes(&contract(&source));
    let response = project(json!(source), &["balance_of", "transfer"]);
    let text = response["did"].as_str().unwrap();
    let kept = actor_modes(&contract(text));
    assert_eq!(
        kept,
        all.iter()
            .filter(|(name, _)| name == "balance_of" || name == "transfer")
            .cloned()
            .collect::<Vec<_>>()
    );
    assert!(kept.iter().any(|(_, mode)| mode == "query"));
    assert!(kept.iter().any(|(_, mode)| mode == "update"));
    let module: Value = serde_json::from_str(&did_to_module(&single(text))).unwrap();
    let module = module["module"].as_str().unwrap();
    let actor = &module[module.find("type $Actor = {").expect("an Actor type")..];
    let actor = &actor[..actor.find("};").unwrap()];
    let listed: Vec<&str> = actor
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().split(':').next())
        .filter(|name| !name.is_empty() && !name.starts_with('/') && !name.starts_with('*'))
        .collect();
    assert_eq!(listed, ["balance_of", "transfer"], "{actor}");
    assert!(module.contains("\"query\""), "{module}");
    for (name, _) in &all {
        if name != "balance_of" && name != "transfer" {
            assert!(
                !actor.contains(&format!("{name}:")),
                "{name} leaked into {actor}"
            );
        }
    }
}

#[test]
fn projections_are_deterministic_whatever_the_order_of_the_names() {
    let source = fixture("ledger");
    let first = project_did(
        &json!({ "source": source, "methods": ["transfer", "balance_of"] }).to_string(),
    );
    let again = project_did(
        &json!({ "source": source, "methods": ["transfer", "balance_of"] }).to_string(),
    );
    let reordered = project_did(
        &json!({ "source": source, "methods": ["balance_of", "transfer", "balance_of"] })
            .to_string(),
    );
    assert_eq!(first, again);
    assert_eq!(first, reordered);
}

/// Doc comments and argument names survive into the projection, so the
/// projected module documents the methods it keeps exactly as the full one.
#[test]
fn docs_and_argument_names_survive() {
    let source = fixture("docs");
    let methods: Vec<String> = actor_modes(&contract(&source))
        .into_iter()
        .map(|(method, _)| method)
        .collect();
    let keep = &methods[..1];
    let names: Vec<&str> = keep.iter().map(String::as_str).collect();
    let response = project(json!(source), &names);
    let text = response["did"].as_str().unwrap();
    let full: Value = serde_json::from_str(&did_to_module(&single(&source))).unwrap();
    let cut: Value = serde_json::from_str(&did_to_module(&single(text))).unwrap();
    let doc_block = |module: &str, method: &str| -> String {
        let actor = &module[module.find("type $Actor = {").unwrap()..];
        let at = actor.find(&format!("  {method}:")).unwrap();
        let before = &actor[..at];
        let start = before.rfind("  /**").map_or(at, |start| start);
        actor[start..at + actor[at..].find('\n').unwrap()].to_string()
    };
    for method in keep {
        let expected = doc_block(full["module"].as_str().unwrap(), method);
        assert!(
            expected.contains("/**"),
            "the fixture documents {method}: {expected}"
        );
        assert_eq!(doc_block(cut["module"].as_str().unwrap(), method), expected);
    }
}

#[test]
fn unknown_and_empty_method_lists_fail_with_diagnostics() {
    let source = fixture("ledger");
    let response = project(json!(source), &["balance_of", "nope", "nope", "also_nope"]);
    assert_eq!(response["ok"], json!(false), "{response}");
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(
        items.len(),
        2,
        "one diagnostic per distinct unknown name: {response}"
    );
    let available = json!([
        "balance_of",
        "decimals",
        "fee",
        "get_transactions",
        "name",
        "symbol",
        "total_supply",
        "transfer"
    ]);
    for (item, name) in items.iter().zip(["nope", "also_nope"]) {
        assert_eq!(item["code"], json!("unknown_method"));
        assert_eq!(item["phase"], json!("project"));
        assert_eq!(item["severity"], json!("error"));
        assert_eq!(
            item["notes"], available,
            "the diagnostic lists the service's methods"
        );
        let message = item["message"].as_str().unwrap();
        assert!(message.contains(&format!("{name:?}")), "{message}");
        assert!(message.contains("balance_of, decimals, fee"), "{message}");
    }

    let response = project(json!(source), &[]);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("empty_method_list")
    );

    let response = project(json!("type A = nat;"), &["a"]);
    assert_eq!(response["diagnostics"][0]["code"], json!("no_service"));

    // A compile failure passes the compiler's diagnostics through.
    let response = project(json!("service : { a : () -> (Missing) }"), &["a"]);
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("did_type_check_error")
    );

    for request in [
        json!({ "source": source }),
        json!({ "source": source, "methods": "balance_of" }),
        json!({ "source": source, "methods": [1] }),
        json!({ "source": source, "methods": [], "extra": true }),
    ] {
        let response: Value = serde_json::from_str(&project_did(&request.to_string())).unwrap();
        assert_eq!(
            response["diagnostics"][0]["code"],
            json!("invalid_request"),
            "{request}"
        );
    }
    // `methods` belongs to a projection request only.
    let response: Value = serde_json::from_str(&did_to_module(
        &json!({ "source": source, "methods": ["fee"] }).to_string(),
    ))
    .unwrap();
    assert_eq!(response["diagnostics"][0]["code"], json!("invalid_request"));
}

/// A bundle projects to one self-contained file: imported declarations and
/// methods of an imported service are inlined, and the result compiles on
/// its own.
#[test]
fn bundles_project_to_one_self_contained_file() {
    let bundle = json!({
        "entry": "app.did",
        "files": {
            "app.did": "import \"types.did\";\nimport service \"admin.did\";\n/// The app.\nservice : {\n  get : (Key) -> (opt Item) query;\n  put : (Key, Item) -> ();\n}\n",
            "types.did": "type Key = text;\n/// An item.\ntype Item = record { id : nat; tags : vec Tag };\ntype Tag = variant { a; b };\ntype Unused = nat;\n",
            "admin.did": "import \"types.did\";\nservice : { reset : (Tag) -> () }\n",
        }
    });
    let response = project(bundle, &["get", "reset"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["methods"], json!(["get", "reset"]));
    let text = response["did"].as_str().unwrap();
    assert!(!text.contains("import"), "{text}");
    assert!(!text.contains("Unused"), "{text}");
    assert!(!text.contains("put"), "{text}");
    // `///` docs are printed as `///` again.
    assert!(text.contains("/// An item.\n"), "{text}");
    assert!(text.contains("/// The app.\n"), "{text}");
    assert_eq!(
        actor_modes(&contract(text)),
        [
            ("get".to_string(), "query".to_string()),
            ("reset".to_string(), "update".to_string())
        ]
    );
}

/// A class projects to the service a client calls: its init arguments, and
/// the declarations only they reach, are dropped.
#[test]
fn a_class_projects_to_its_service() {
    let source = "type Init = record { owner : principal };\ntype Item = record { id : nat };\nservice : (Init) -> {\n  get : () -> (Item) query;\n  admin : (Init) -> ();\n}\n";
    let response = project(json!(source), &["get"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(
        response["did"],
        json!(
            "type Item = record {\n  id : nat;\n};\nservice : {\n  get : () -> (Item) query;\n}\n"
        )
    );
}

/// A leading byte order mark is skipped on the way in, as the compiler skips
/// it; the projection is the same as for the unmarked text.
#[test]
fn a_leading_bom_is_skipped() {
    let source = fixture("ledger");
    let marked = format!("\u{FEFF}{source}");
    assert_eq!(
        project(json!(marked), &["fee"])["did"],
        project(json!(source), &["fee"])["did"]
    );
}

/// A method name that needs quoting is quoted in the projection and found by
/// its unquoted name.
#[test]
fn quoted_method_names_round_trip() {
    let source = "service : {\n  \"two words\" : () -> (nat) query;\n  \"\u{e9}t\u{e9}\" : (text) -> ();\n  plain : () -> ();\n}\n";
    let response = project(json!(source), &["two words", "\u{e9}t\u{e9}"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    let text = response["did"].as_str().unwrap();
    assert!(text.contains("\"two words\""), "{text}");
    assert_eq!(
        actor_modes(&contract(text)),
        [
            ("two words".to_string(), "query".to_string()),
            ("\u{e9}t\u{e9}".to_string(), "update".to_string())
        ]
    );
}

/// A service written through a declaration, and a method typed by a func
/// declaration: both are followed, and the func declaration is kept because
/// the method reaches it.
#[test]
fn service_and_method_aliases_are_followed() {
    let source = "type Get = func () -> (nat) query;\ntype Unused = text;\ntype S = service {\n  get : Get;\n  put : (nat) -> ();\n};\nservice : S\n";
    let response = project(json!(source), &["get"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(
        response["did"],
        json!("type Get = func () -> (nat) query;\nservice : {\n  get : Get;\n}\n")
    );
    assert_eq!(
        actor_modes(&contract(response["did"].as_str().unwrap())),
        [("get".to_string(), "query".to_string())]
    );
}
