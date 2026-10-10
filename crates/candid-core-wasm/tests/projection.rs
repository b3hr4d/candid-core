//! `projectDid`: a `.did` holding exactly the named methods and every type
//! they reach, deterministic to the byte, through the same entry point the
//! wasm artifact wraps.

use std::path::PathBuf;

use candid_core_wasm::{
    did_to_contract, did_to_module, project_did, project_did_with, ProjectionOptions,
    ProjectionWork, MAX_PROJECTION_DIAGNOSTIC_BYTES,
};
use candid_parser::syntax::{Binding, IDLProg, IDLType};
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

/// A single-source `.did` as the parser reads it: its declarations and its
/// service's docs and methods, every doc comment included (a field's,
/// a tuple element's, an arm's, a method's, a declaration's). Labels compare
/// by id, so a tuple element and the numbered field it stands for agree.
fn parsed(source: &str) -> (Vec<Binding>, Vec<String>, Vec<Binding>) {
    let program: IDLProg = source
        .parse()
        .unwrap_or_else(|error| panic!("{error}\n{source}"));
    let actor = program.actor.expect("a service");
    let mut service = actor.typ;
    if let IDLType::ClassT(_, inner) = service {
        service = *inner;
    }
    let IDLType::ServT(methods) = service else {
        panic!("an inline service: {service:?}");
    };
    (
        IDLProg::typ_decs(program.decs).collect(),
        actor.docs,
        methods,
    )
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
/// nothing; it must then keep the Contract identity, its generated module
/// must be byte-identical to the full one — declaration names, doc
/// comments, argument names, labels and quoting included — and it must
/// parse back to the same declarations and methods, docs included, the
/// ones the generator does not emit (a tuple element's) among them.
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
        assert_eq!(
            parsed(projected),
            parsed(&source),
            "{name}: the parsed source changed\n{projected}"
        );
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
    let actor = &module[module.find("type Actor = {").expect("an Actor type")..];
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
        let actor = &module[module.find("type Actor = {").unwrap()..];
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

/// A tuple element's doc comment survives: in a declared tuple, in a tuple
/// nested in another tuple, a record field, a variant arm, an `opt` or a
/// `vec`, and in tuples a method takes and returns inline. The parser keeps
/// those docs (the generator does not emit them, so the module alone cannot
/// show the loss); the projection must parse back to the same docs, keep the
/// Contract identity and generate the same module. A tuple with no
/// documented element keeps its one-line form.
#[test]
fn documented_tuple_elements_keep_their_docs() {
    let source = "\
/// A documented pair.
type Pair = record {
  /// The amount.
  nat;
  text;
};
type Plain = record { nat; text };
type Nested = record {
  /// The outer element.
  record {
    /// The inner element.
    nat;
    /// A doc line that is //// slashes.
    //// A doc line that starts with a slash.
    principal;
  };
  vec record {
    /// In a vec.
    nat8;
  };
};
type Holder = record {
  pair : opt record {
    bool;
    /// In an opt, in a field, second.
    int;
  };
  choice : variant {
    Both : record {
      /// In an arm.
      text;
      nat;
    };
    Neither;
  };
  plain : record { nat; Plain };
};
service : {
  /// The call.
  call : (Pair, Nested, Holder, record {
    /// In an argument.
    nat;
  }) -> (record {
    /// In a result.
    text;
    bool;
  }) query;
}
";
    let response = project(json!(source), &["call"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    let projected = response["did"].as_str().unwrap();
    assert_eq!(
        parsed(projected),
        parsed(source),
        "the parsed source changed\n{projected}"
    );
    assert_eq!(
        response["projection"], response["input"],
        "identities moved\n{projected}"
    );
    let before: Value = serde_json::from_str(&did_to_module(&single(source))).unwrap();
    let after: Value = serde_json::from_str(&did_to_module(&single(projected))).unwrap();
    assert_eq!(after, before, "the module changed\n{projected}");
    // Every doc of the source is in the text, and an undocumented tuple,
    // alone or holding another, stays on one line.
    for doc in [
        "The amount.",
        "The outer element.",
        "The inner element.",
        "A doc line that is //// slashes.",
        "A doc line that starts with a slash.",
        "In a vec.",
        "In an opt, in a field, second.",
        "In an arm.",
        "In an argument.",
        "In a result.",
    ] {
        assert!(projected.contains(doc), "{doc} is missing\n{projected}");
    }
    assert!(
        projected.contains("type Plain = record { nat; text };\n"),
        "{projected}"
    );
    assert!(
        projected.contains("  plain : record { nat; Plain };\n"),
        "{projected}"
    );
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
            item["message"],
            json!(format!("the service has no method {name:?}"))
        );
    }
    // The service's methods are listed once, sorted, in the first.
    assert_eq!(
        items[0]["notes"], available,
        "the first diagnostic lists the service's methods"
    );
    assert!(items[1].get("notes").is_none(), "{response}");

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

/// A class whose service is reached in more steps than there are
/// declarations: an inline service with no declarations at all, and a
/// named service type with one declaration. Each projects to its service.
#[test]
fn a_class_with_few_declarations_projects_to_its_service() {
    let inline = "service : (nat) -> {\n  get : () -> (nat) query;\n  put : (nat) -> ();\n}\n";
    for source in [
        inline.to_string(),
        inline.replace("(nat) -> {", "(n : nat) -> {"),
        inline.replace("(nat) -> {", "() -> {"),
    ] {
        let response = project(json!(source), &["get"]);
        assert_eq!(response["ok"], json!(true), "{source}: {response}");
        assert_eq!(
            response["did"],
            json!("service : {\n  get : () -> (nat) query;\n}\n"),
            "{source}"
        );
    }
    let named = "type S = service {\n  get : () -> (nat) query;\n};\nservice : (nat) -> S\n";
    let response = project(json!(named), &["get"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(
        response["did"],
        json!("service : {\n  get : () -> (nat) query;\n}\n")
    );
}

/// Every conformance fixture with a method projects onto all of its methods:
/// the same methods with the same modes, and the same interface identity
/// unless the actor is a class (its init arguments are dropped).
#[test]
fn every_conformance_fixture_projects_onto_all_its_methods() {
    let mut projected = 0;
    for name in ["actorless", "basic", "class", "empty_actor", "recursive"] {
        let source = repo(&format!("tests/fixtures/conformance/{name}.did"));
        let envelope = contract(&source);
        if envelope["contract"]["actor"].is_null() {
            continue;
        }
        let methods: Vec<String> = actor_modes(&envelope)
            .into_iter()
            .map(|(method, _)| method)
            .collect();
        if methods.is_empty() {
            continue;
        }
        let names: Vec<&str> = methods.iter().map(String::as_str).collect();
        let response = project(json!(source), &names);
        assert_eq!(response["ok"], json!(true), "{name}: {response}");
        let text = response["did"].as_str().unwrap();
        assert_eq!(
            actor_modes(&contract(text)),
            actor_modes(&envelope),
            "{name}"
        );
        // A class's init arguments are dropped, which moves its interface;
        // any other actor keeps it.
        if envelope["contract"]["actor"]["class"].is_null() {
            assert_eq!(
                response["projection"]["interface_id"], response["input"]["interface_id"],
                "{name}: {response}"
            );
        } else {
            assert!(text.starts_with("service : {"), "{text}");
        }
        projected += 1;
    }
    assert_eq!(projected, 3, "basic, class and recursive have methods");
}

/// Methods come out in name order (code point), whatever order the source
/// declares them in, and so does the `methods` field; record fields come out
/// in label-id order.
#[test]
fn projected_methods_are_in_name_order() {
    let source = "service : {\n  zeta : () -> ();\n  alpha : () -> ();\n  Mid : () -> ();\n  mid : () -> ();\n}\n";
    let response = project(json!(source), &["zeta", "mid", "alpha", "Mid"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["methods"], json!(["Mid", "alpha", "mid", "zeta"]));
    assert_eq!(
        response["did"],
        json!("service : {\n  Mid : () -> ();\n  alpha : () -> ();\n  mid : () -> ();\n  zeta : () -> ();\n}\n")
    );
    let response = project(
        json!("type R = record { zeta : nat; alpha : text; mid : bool };\nservice : { get : () -> (R) }\n"),
        &["get"],
    );
    assert_eq!(
        response["did"],
        json!("type R = record {\n  mid : bool;\n  alpha : text;\n  zeta : nat;\n};\nservice : {\n  get : () -> (R);\n}\n")
    );
}

/// A projection request: `sources` (Candid text or a bundle) and `methods`.
fn request(sources: Value, methods: &[String]) -> String {
    let mut request = match sources {
        Value::String(source) => json!({ "source": source }),
        other => other,
    };
    request["methods"] = json!(methods);
    request.to_string()
}

/// The response, its length in bytes, and what its lookups by name cost.
fn project_with(request: &str, options: ProjectionOptions) -> (Value, usize, ProjectionWork) {
    let (response, work) = project_did_with(request, options);
    (
        serde_json::from_str(&response).unwrap(),
        response.len(),
        work,
    )
}

/// The most names a lookup among `entries` sorted names compares: a binary
/// search, never a scan.
fn per_lookup(entries: usize) -> usize {
    entries.max(1).ilog2() as usize + 2
}

/// A service of `count` methods, `m000000` on, in one line.
fn service_of(count: usize) -> String {
    let mut source = String::from("service:{");
    for index in 0..count {
        source.push_str(&format!("m{index:06}:()->();"));
    }
    source.push_str("}\n");
    source
}

/// Codex round 4, finding 1: an unknown-method failure listed the service's
/// methods in the `message` and the `notes` of every diagnostic, so this
/// request, 3,000 methods and 3,000 unknown names in 75,037 bytes, got a
/// 252,591,041-byte response. The methods are now listed once, in the first
/// diagnostic's `notes`, and each message names its unknown method alone.
#[test]
fn an_unknown_method_failure_lists_the_services_methods_once() {
    let unknown: Vec<String> = (0..3_000).map(|index| format!("x{index:06}")).collect();
    let request = request(json!(service_of(3_000)), &unknown);
    let (response, length, work) = project_with(&request, ProjectionOptions::default());
    assert_eq!(response["ok"], json!(false));
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 3_000);
    for (item, name) in items.iter().zip(&unknown) {
        assert_eq!(item["code"], json!("unknown_method"));
        assert_eq!(
            item["message"],
            json!(format!("the service has no method {name:?}"))
        );
    }
    let methods: Vec<String> = (0..3_000).map(|index| format!("m{index:06}")).collect();
    assert_eq!(items[0]["notes"], json!(methods));
    assert!(items[1..].iter().all(|item| item.get("notes").is_none()));
    // Linear in the request: each name once in a message, each method once
    // in the notes.
    assert_eq!(request.len(), 75_037);
    assert_eq!(length, 522_066);
    // One lookup per distinct requested name, each a binary search.
    assert_eq!(work.lookups, 3_000);
    assert!(
        work.compared <= work.lookups * per_lookup(3_000),
        "{work:?}"
    );
}

/// Codex round 4, finding 2: each requested name was looked for by a scan of
/// the service's methods, so projecting every method of a large service was
/// quadratic in its methods. Each distinct name is now one binary search in
/// the methods sorted once; asked for twice, it is looked up once.
#[test]
fn requested_names_are_found_by_binary_search() {
    let count = 4_000;
    let mut names: Vec<String> = (0..count)
        .rev()
        .map(|index| format!("m{index:06}"))
        .collect();
    names.extend(names[..100].to_vec());
    let (response, _, work) = project_with(
        &request(json!(service_of(count)), &names),
        ProjectionOptions::default(),
    );
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["methods"].as_array().unwrap().len(), count);
    assert_eq!(response["did"].as_str().unwrap().len(), 88_014);
    // The methods' types are inline: no declaration is looked up.
    assert_eq!(work.lookups, count);
    assert!(
        work.compared <= work.lookups * per_lookup(count),
        "{work:?}"
    );

    // An unknown name costs one lookup too.
    let (response, _, work) = project_with(
        &request(json!(service_of(count)), &["nope".to_string()]),
        ProjectionOptions::default(),
    );
    assert_eq!(response["diagnostics"][0]["code"], json!("unknown_method"));
    assert_eq!(work.lookups, 1);
    assert!(work.compared <= per_lookup(count), "{work:?}");
}

/// Codex round 4, finding 3: reachability looked each declaration name up
/// with `IDLMergedProg::lookup`, which scans every declaration, so a record
/// of thousands of fields typed by separate declarations was quadratic. Each
/// name is now looked up once, by binary search in the declarations sorted
/// once.
#[test]
fn declarations_are_found_by_binary_search() {
    let fields = 3_000;
    let mut source = String::new();
    for index in 0..fields {
        source.push_str(&format!("type T{index} = nat;\n"));
    }
    // Declared after its field types, so a scan from the start would pass
    // over all of them to find it.
    source.push_str("type R = record {\n");
    for index in 0..fields {
        source.push_str(&format!("f{index} : T{index};\n"));
    }
    source.push_str(
        "};\ntype Unused = R;\nservice : { get : () -> (R) query; put : (Unused) -> () }\n",
    );
    let (response, _, work) = project_with(
        &request(json!(source), &["get".to_string()]),
        ProjectionOptions::default(),
    );
    assert_eq!(response["ok"], json!(true), "{response}");
    let text = response["did"].as_str().unwrap();
    assert!(!text.contains("Unused"), "only what `get` reaches");
    assert_eq!(text.matches("type ").count(), fields + 1);
    assert_eq!(text.len(), 101_730);
    // `get`, then `R` and each `T`: one lookup each.
    assert_eq!(work.lookups, 1 + 1 + fields);
    let declarations = fields + 2;
    assert!(
        work.compared <= per_lookup(2) + (fields + 1) * per_lookup(declarations),
        "{work:?}"
    );
}

/// The actor's service is resolved through the same index: each step of a
/// chain of service names, the entry's and a service import's, is one
/// binary search, where `IDLMergedProg::resolve_actor` and the walk to the
/// service scanned the declarations at every step.
#[test]
fn the_actor_is_resolved_through_the_index() {
    let chain = 300;
    let declare = |prefix: &str, method: &str| {
        let mut source = String::new();
        for index in 0..chain {
            if index + 1 < chain {
                source.push_str(&format!("type {prefix}{index} = {prefix}{};\n", index + 1));
            } else {
                source.push_str(&format!(
                    "type {prefix}{index} = service {{ {method} : () -> (nat) query }};\n"
                ));
            }
        }
        source
    };
    let bundle = json!({
        "entry": "main.did",
        "files": {
            "main.did": format!("import service \"other.did\";\n{}service : E0\n", declare("E", "get")),
            "other.did": format!("{}service : I0\n", declare("I", "put")),
        },
    });
    let (response, _, work) = project_with(
        &request(bundle, &["get".to_string(), "put".to_string()]),
        ProjectionOptions::default(),
    );
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(
        response["did"],
        json!("service : {\n  get : () -> (nat) query;\n  put : () -> (nat) query;\n}\n")
    );
    // Two requested names, then each chain's steps.
    assert_eq!(work.lookups, 2 + 2 * chain);
    assert!(
        work.compared <= 2 * per_lookup(2) + 2 * chain * per_lookup(2 * chain),
        "{work:?}"
    );
}

/// A service import's methods come out among the entry's in name order, as
/// the docs say, not after them. Before this round the projection kept the
/// order `IDLMergedProg::resolve_actor` gives: the entry's methods, then the
/// import's.
#[test]
fn a_service_imports_methods_come_out_in_name_order() {
    let bundle = json!({
        "entry": "main.did",
        "files": {
            "main.did": "import service \"other.did\";\nservice : { zeta : () -> (); mid : () -> () }\n",
            "other.did": "service : { alpha : () -> (); omega : () -> () }\n",
        },
    });
    let response = project(bundle, &["omega", "zeta", "alpha", "mid"]);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(
        response["methods"],
        json!(["alpha", "mid", "omega", "zeta"])
    );
    assert_eq!(
        response["did"],
        json!("service : {\n  alpha : () -> ();\n  mid : () -> ();\n  omega : () -> ();\n  zeta : () -> ();\n}\n")
    );
}

/// A source of `depth` nested records whose innermost holds `fields` fields
/// of `docs` doc lines each: each doc line is printed behind two spaces a
/// level, so the text printed is far longer than the source.
fn deep_docs(depth: usize, fields: usize, docs: usize) -> String {
    let mut source = String::from("type R = ");
    for _ in 0..depth {
        source.push_str("record { a : ");
    }
    source.push_str("record {\n");
    for index in 0..fields {
        for _ in 0..docs {
            source.push_str("//\n");
        }
        source.push_str(&format!("x{index} : nat;\n"));
    }
    source.push('}');
    for _ in 0..depth {
        source.push_str(" }");
    }
    source.push_str(";\nservice : { get : () -> (R) query }\n");
    source
}

/// A projection whose text would pass the compiler's bound on one source
/// cannot be compiled again, so printing stops at the first piece that
/// would pass it, and the projection fails with `projection_bytes`. Before
/// this round the printer wrote all of it (here 2,129,300 bytes from an
/// 18,597-byte source; up to about 18 MB within the compiler's limits), and
/// the second compile refused it as `projection_failed`.
#[test]
fn a_projection_past_the_compilers_source_bound_stops_printing() {
    let source = deep_docs(200, 50, 100);
    assert_eq!(source.len(), 18_597);
    let (response, length, _) = project_with(
        &request(json!(source), &["get".to_string()]),
        ProjectionOptions::default(),
    );
    assert_eq!(response["ok"], json!(false));
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{response}");
    assert_eq!(items[0]["code"], json!("resource_limit_exceeded"));
    assert_eq!(items[0]["phase"], json!("project"));
    let limit = 1_048_576;
    assert_eq!(
        items[0]["resource_limit"]["resource"],
        json!("projection_bytes")
    );
    assert_eq!(items[0]["resource_limit"]["limit"], json!(limit));
    // Stopped at the piece that passed the bound: at most one line's
    // indentation (two spaces a level) past it.
    let observed = items[0]["resource_limit"]["observed"].as_u64().unwrap() as usize;
    assert!(
        observed > limit && observed <= limit + 2 * 201,
        "{observed}"
    );
    assert!(length < 1_000, "{length}");

    // At the bound exactly the projection is written; one byte under, the
    // last piece (the final newline) does not fit.
    let ledger = request(json!(fixture("ledger")), &["fee".to_string()]);
    let full = project(json!(fixture("ledger")), &["fee"])["did"]
        .as_str()
        .unwrap()
        .len();
    let bounded = |text_byte_limit| {
        project_with(
            &ledger,
            ProjectionOptions {
                text_byte_limit,
                ..ProjectionOptions::default()
            },
        )
        .0
    };
    assert_eq!(bounded(full)["ok"], json!(true));
    let refused = bounded(full - 1);
    assert_eq!(
        refused["diagnostics"][0]["resource_limit"],
        json!({ "resource": "projection_bytes", "limit": full - 1, "observed": full })
    );
}

/// The text of the unknown-method messages is bounded in all
/// ([`MAX_PROJECTION_DIAGNOSTIC_BYTES`]), as the check's reported text is:
/// the name whose message would pass the bound, and every unknown name after
/// it, are only counted, and one `resource_limit_exceeded` diagnostic
/// (`projection_diagnostic_bytes`) ends the list. A message is measured as
/// the JSON writes it, and only while its raw length fits in what is left.
#[test]
fn the_unknown_method_text_is_bounded() {
    let source = fixture("ledger");
    let names = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    let three = request(json!(source), &names(&["x", "y", "z", "y"]));
    let bounded = |request: &str, diagnostic_byte_limit| {
        project_with(
            request,
            ProjectionOptions {
                diagnostic_byte_limit,
                ..ProjectionOptions::default()
            },
        )
        .0
    };
    let codes = |response: &Value| {
        response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["code"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    // `the service has no method \"x\"`: 31 bytes as the JSON writes it,
    // so three take 93.
    let all = bounded(&three, 93);
    assert_eq!(codes(&all), ["unknown_method"; 3]);
    let two = bounded(&three, 92);
    assert_eq!(
        codes(&two),
        [
            "unknown_method",
            "unknown_method",
            "resource_limit_exceeded"
        ]
    );
    assert_eq!(
        two["diagnostics"][2]["resource_limit"],
        json!({ "resource": "projection_diagnostic_bytes", "limit": 92, "observed": 93 })
    );
    assert_eq!(
        two["diagnostics"][2]["message"],
        json!("the unknown-method diagnostics reached their projection_diagnostic_bytes bound of 92, so 1 unknown method name(s) are not reported")
    );
    assert!(two["diagnostics"][0]["notes"].is_array());
    // Not even the first fits: the bound's diagnostic is the first, and it
    // carries the service's methods.
    let none = bounded(&three, 30);
    assert_eq!(codes(&none), ["resource_limit_exceeded"]);
    assert_eq!(none["diagnostics"][0]["notes"].as_array().unwrap().len(), 8);
    assert!(none["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .ends_with(", so 3 unknown method name(s) are not reported"));
    // A name longer than what is left is counted at its raw length, not
    // quoted and measured: 1,000 quotes would be 4,030 bytes written.
    let quotes = request(json!(source), &["\"".repeat(1_000)]);
    let long = bounded(&quotes, 100);
    assert_eq!(
        long["diagnostics"][0]["resource_limit"]["observed"],
        json!(26 + 1_000 + 2)
    );
    let measured = bounded(&quotes, 4_030);
    assert_eq!(codes(&measured), ["unknown_method"]);
    assert_eq!(
        serde_json::to_string(&measured["diagnostics"][0]["message"])
            .unwrap()
            .len()
            - 2,
        4_030
    );

    // At the default bound: 5,000 unknown names of 1,000 bytes each, 5 MB of
    // messages (1,030 bytes each as the JSON writes them); the first 4,072
    // are reported.
    let long_names: Vec<String> = (0..5_000).map(|index| format!("{index:01000}")).collect();
    let (response, length, _) = project_with(
        &request(json!(source), &long_names),
        ProjectionOptions::default(),
    );
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 4_073);
    assert_eq!(
        items[4_072]["resource_limit"],
        json!({
            "resource": "projection_diagnostic_bytes",
            "limit": MAX_PROJECTION_DIAGNOSTIC_BYTES,
            // The 4,073rd is longer than what is left, so it is counted
            // at its raw length.
            "observed": 4_072 * 1_030 + 1_028,
        })
    );
    assert!(items[4_072]["message"]
        .as_str()
        .unwrap()
        .ends_with(", so 928 unknown method name(s) are not reported"));
    // 4 MiB of messages, the per-diagnostic keys, and the ledger's eight
    // methods once.
    assert_eq!(length, 4_675_289);
}
