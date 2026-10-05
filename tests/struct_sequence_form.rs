//! Issue #235: the bounded loaders refuse serde's sequence form of a struct.
//!
//! Serde's derived `Deserialize` reads a struct written as a JSON array of its
//! field values in declaration order, and an internally tagged enum written
//! as `[tag, field…]`, at any depth. The Contract format is objects only:
//! candid-core writes nothing else, its DTOs deny unknown keys, and
//! `@candid-core/schema`'s `schemaFromContract` refuses the array form. These
//! tests write each struct position of every document a bounded loader reads
//! (Contract, envelope, Compilation and its `SourceInfo` sidecar) in that
//! form, otherwise unchanged, and require a refusal with the error a value of
//! the wrong type gets: `ContractJsonError::MalformedJson`, `invalid type:
//! sequence, expected an object at <path>`. The object form of the same
//! document must load, so each refusal is the array's alone. The two issue
//! repros carry the issue's names, `contract_field_as_array` and
//! `contract_declaration_as_array`.

use candid_core::{
    compile_did, compile_with_resolver, Compilation, CompileOptions, Contract, ContractEnvelope,
    ContractJsonError, Limits, MemoryResolver, RuntimeContext,
};
use serde_json::{json, Value};

/// The issue's environment: `R` is `types[0]`, `nat` is `types[1]`.
const ISSUE_DID: &str = "type R = record { a : nat; b : nat };\n";

/// Every type node kind, a variant arm, a service method and a class actor.
const KINDS_DID: &str = "type R = record { a : nat; b : nat };
type V = variant { x; y : text };
type O = opt nat;
type W = vec text;
service : (nat) -> { m : (R, O) -> (V, W) query }
";

/// A service actor.
const SERVICE_DID: &str = "service : { m : () -> () }\n";

const ROOT_DID: &str = r#"import "types.did";
/// Root documentation.
service : {
  /// Ping documentation.
  ping: (name: text, tag: nat) -> (item: Item) query;
};"#;

const TYPES_DID: &str = r#"/// Item documentation.
type Item = record {
  /// Identifier documentation.
  id: nat;
  label: text;
  1: nat;
};"#;

fn contract_document(source: &str) -> Value {
    let json = compile_did(source)
        .expect("the source compiles")
        .contract()
        .to_json_pretty()
        .expect("the Contract serializes");
    serde_json::from_str(&json).expect("a JSON object")
}

fn envelope_document(source: &str) -> Value {
    let contract = compile_did(source)
        .expect("the source compiles")
        .contract()
        .clone();
    let mut envelope = ContractEnvelope::new(contract);
    envelope
        .insert_extension(
            "org.candid-core.field-names/v1",
            json!([[0, 97, "a"], [0, 98, "b"]]),
            &Limits::default(),
        )
        .expect("the extension is valid");
    let json = envelope
        .to_json_pretty_with_limits(&Limits::default())
        .expect("the envelope serializes");
    serde_json::from_str(&json).expect("a JSON object")
}

/// A two-source bundle whose sidecar fills every provenance collection.
fn compilation_document() -> Value {
    let mut resolver = MemoryResolver::new();
    resolver.insert("root.did", ROOT_DID).unwrap();
    resolver.insert("types.did", TYPES_DID).unwrap();
    let compilation = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions {
            include_source_info: true,
        },
        &RuntimeContext::default(),
    )
    .expect("the bundle compiles");
    let json = compilation
        .to_json_pretty_with_limits(&Limits::default())
        .expect("the Compilation serializes");
    let document: Value = serde_json::from_str(&json).expect("a JSON object");
    let info = &document["source_info"];
    for collection in [
        "sources",
        "imports",
        "declarations",
        "field_labels",
        "methods",
        "function_arguments",
        "actors",
    ] {
        assert!(
            !info[collection].as_array().expect(collection).is_empty(),
            "the bundle must fill `{collection}`"
        );
    }
    document
}

#[derive(Clone, Copy, Debug)]
enum Loader {
    Contract,
    Envelope,
    Compilation,
}

impl Loader {
    /// Load `text` through the `_with_context` form of this loader's `&str`
    /// entry point; `loader_entry_points` covers every other form.
    fn load(self, text: &str) -> Result<(), ContractJsonError> {
        let context = RuntimeContext::default();
        match self {
            Self::Contract => Contract::from_json_with_context(text, &context).map(drop),
            Self::Envelope => ContractEnvelope::from_json_with_context(text, &context).map(drop),
            Self::Compilation => Compilation::from_json_with_context(text, &context).map(drop),
        }
    }
}

/// A JSON pointer-like path, as `$`-rooted steps.
#[derive(Clone, Copy)]
enum Step {
    Key(&'static str),
    Index(usize),
}

fn at<'a>(document: &'a mut Value, path: &[Step]) -> &'a mut Value {
    path.iter().fold(document, |value, step| match step {
        Step::Key(key) => value
            .get_mut(*key)
            .unwrap_or_else(|| panic!("no key {key}")),
        Step::Index(index) => value
            .get_mut(*index)
            .unwrap_or_else(|| panic!("no index {index}")),
    })
}

fn render(path: &[Step]) -> String {
    let mut rendered = String::from("$");
    for step in path {
        match step {
            Step::Key(key) => rendered.push_str(&format!(".{key}")),
            Step::Index(index) => rendered.push_str(&format!("[{index}]")),
        }
    }
    rendered
}

/// Serde's sequence form of the object at hand: its values in the struct's
/// declaration order `fields`, preceded by the tag of an internally tagged
/// enum when `tagged`. A field the object omits must be a trailing defaulted
/// one, which the sequence form omits too.
fn sequence_form(object: &Value, tagged: bool, fields: &[&str]) -> Value {
    let map = object.as_object().expect("an object to rewrite");
    let mut items = Vec::new();
    if tagged {
        items.push(map["kind"].clone());
    }
    let mut ended = false;
    for field in fields {
        match map.get(*field) {
            Some(value) => {
                assert!(!ended, "`{field}` follows an omitted field");
                items.push(value.clone());
            }
            None => ended = true,
        }
    }
    let expected = map.len() - usize::from(tagged);
    assert_eq!(
        items.len() - usize::from(tagged),
        expected,
        "the declaration order must name every key of {object}"
    );
    Value::Array(items)
}

fn outcome(result: &Result<(), ContractJsonError>) -> String {
    match result {
        Ok(()) => "accepted".to_string(),
        Err(error) => format!("{error:?}"),
    }
}

/// Write the struct at `path` of `document` in sequence form and require
/// `loader` to refuse it as a value of the wrong type at that path, after
/// requiring the document itself to load.
fn assert_refused(
    loader: Loader,
    mut document: Value,
    path: &[Step],
    tagged: bool,
    fields: &[&str],
) {
    let object_form = document.to_string();
    assert!(
        loader.load(&object_form).is_ok(),
        "{loader:?}: the object form must load: {}",
        outcome(&loader.load(&object_form))
    );
    let target = at(&mut document, path);
    *target = sequence_form(target, tagged, fields);
    assert_sequence_refused(loader, &document.to_string(), &render(path));
}

fn assert_sequence_refused(loader: Loader, text: &str, path: &str) {
    let result = loader.load(text);
    let expected = format!("invalid type: sequence, expected an object at {path}, line ");
    match &result {
        Err(ContractJsonError::MalformedJson(message)) if message.starts_with(&expected) => {}
        _ => panic!(
            "{loader:?}: a struct written as an array at {path} must be refused with \
             `{expected}…`; got {}",
            outcome(&result)
        ),
    }
}

use Step::{Index, Key};

const RAW_CONTRACT: &[&str] = &[
    "format",
    "format_version",
    "semantics_profile",
    "canonicalization_profile",
    "identities",
    "producer",
    "types",
    "declarations",
    "actor",
];

/// The arena index of the first type node of `kind` in `document`.
fn node(document: &Value, kind: &str) -> usize {
    document["types"]
        .as_array()
        .expect("types")
        .iter()
        .position(|node| node["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} node"))
}

// The issue's two repros, exactly as #235 states them.

#[test]
fn contract_field_as_array() {
    let mut document = contract_document(ISSUE_DID);
    assert_eq!(
        document["types"][0]["fields"][0],
        json!({"id": 97, "type": 1})
    );
    *at(
        &mut document,
        &[Key("types"), Index(0), Key("fields"), Index(0)],
    ) = json!([97, 1]);
    assert_sequence_refused(
        Loader::Contract,
        &document.to_string(),
        "$.types[0].fields[0]",
    );
}

#[test]
fn contract_declaration_as_array() {
    let mut document = contract_document(ISSUE_DID);
    assert_eq!(document["declarations"][0], json!({"name": "R", "type": 0}));
    *at(&mut document, &[Key("declarations"), Index(0)]) = json!(["R", 0]);
    assert_sequence_refused(Loader::Contract, &document.to_string(), "$.declarations[0]");
}

// Every other struct position of a Contract document.

#[test]
fn contract_document_as_array() {
    assert_refused(
        Loader::Contract,
        contract_document(SERVICE_DID),
        &[],
        false,
        RAW_CONTRACT,
    );
}

#[test]
fn contract_identities_as_array() {
    assert_refused(
        Loader::Contract,
        contract_document(SERVICE_DID),
        &[Key("identities")],
        false,
        &["contract", "interface"],
    );
}

#[test]
fn contract_producer_as_array() {
    assert_refused(
        Loader::Contract,
        contract_document(SERVICE_DID),
        &[Key("producer")],
        false,
        &["name", "version", "candid_version", "candid_parser_version"],
    );
}

#[test]
fn contract_service_actor_as_array() {
    assert_refused(
        Loader::Contract,
        contract_document(SERVICE_DID),
        &[Key("actor")],
        true,
        &["service"],
    );
}

#[test]
fn contract_class_actor_as_array() {
    assert_refused(
        Loader::Contract,
        contract_document(KINDS_DID),
        &[Key("actor")],
        true,
        &["class"],
    );
}

#[test]
fn contract_variant_arm_as_array() {
    let document = contract_document(KINDS_DID);
    let variant = node(&document, "variant");
    assert_refused(
        Loader::Contract,
        document,
        &[Key("types"), Index(variant), Key("fields"), Index(1)],
        false,
        &["id", "type"],
    );
}

#[test]
fn contract_service_method_as_array() {
    let document = contract_document(KINDS_DID);
    let service = node(&document, "service");
    assert_refused(
        Loader::Contract,
        document,
        &[Key("types"), Index(service), Key("methods"), Index(0)],
        false,
        &["name", "id", "function"],
    );
}

/// Each type node kind as `[kind, field…]`, the internally tagged enum's
/// sequence form.
fn assert_type_node_refused(kind: &str, fields: &[&str]) {
    let document = contract_document(KINDS_DID);
    let index = node(&document, kind);
    assert_refused(
        Loader::Contract,
        document,
        &[Key("types"), Index(index)],
        true,
        fields,
    );
}

#[test]
fn contract_primitive_node_as_array() {
    assert_type_node_refused("primitive", &["primitive"]);
}

#[test]
fn contract_opt_node_as_array() {
    assert_type_node_refused("opt", &["inner"]);
}

#[test]
fn contract_vec_node_as_array() {
    assert_type_node_refused("vec", &["inner"]);
}

#[test]
fn contract_record_node_as_array() {
    assert_type_node_refused("record", &["fields"]);
}

#[test]
fn contract_variant_node_as_array() {
    assert_type_node_refused("variant", &["fields"]);
}

#[test]
fn contract_func_node_as_array() {
    assert_type_node_refused("func", &["args", "results", "mode"]);
}

#[test]
fn contract_service_node_as_array() {
    assert_type_node_refused("service", &["methods"]);
}

#[test]
fn contract_class_node_as_array() {
    assert_type_node_refused("class", &["init", "service"]);
}

// The envelope.

#[test]
fn envelope_as_array() {
    assert_refused(
        Loader::Envelope,
        envelope_document(ISSUE_DID),
        &[],
        false,
        &["contract", "extensions"],
    );
}

#[test]
fn envelope_contract_as_array() {
    assert_refused(
        Loader::Envelope,
        envelope_document(ISSUE_DID),
        &[Key("contract")],
        false,
        RAW_CONTRACT,
    );
}

#[test]
fn envelope_contract_field_as_array() {
    assert_refused(
        Loader::Envelope,
        envelope_document(ISSUE_DID),
        &[
            Key("contract"),
            Key("types"),
            Index(0),
            Key("fields"),
            Index(0),
        ],
        false,
        &["id", "type"],
    );
}

/// Extension values are free-form JSON outside the closed format: arrays in
/// arrays, and keys that name a struct position elsewhere, load as before.
#[test]
fn envelope_extensions_stay_free_form() {
    let mut document = envelope_document(ISSUE_DID);
    document["extensions"]["org.example.free/v1"] = json!({
        "identities": [["a"], []],
        "types": [[0, [1, {"fields": [[2]]}]]],
        "nested": {"declarations": [["R", 0]], "s": "[{\"]}"},
    });
    document["extensions"]["org.example.list/v1"] = json!([[["deep"]], {"actor": ["service", 0]}]);
    let text = document.to_string();
    let envelope = ContractEnvelope::from_json_with_context(&text, &RuntimeContext::default())
        .expect("free-form extension values load");
    assert_eq!(envelope.extensions().len(), 3);
}

// The Compilation and its SourceInfo sidecar.

#[test]
fn compilation_as_array() {
    assert_refused(
        Loader::Compilation,
        compilation_document(),
        &[],
        false,
        &["contract", "source_info"],
    );
}

#[test]
fn compilation_contract_declaration_as_array() {
    let document = compilation_document();
    assert_refused(
        Loader::Compilation,
        document,
        &[Key("contract"), Key("declarations"), Index(0)],
        false,
        &["name", "type"],
    );
}

#[test]
fn source_info_as_array() {
    assert_refused(
        Loader::Compilation,
        compilation_document(),
        &[Key("source_info")],
        false,
        &[
            "source_info_version",
            "contract_id",
            "source_bundle_id",
            "sources",
            "imports",
            "declarations",
            "field_labels",
            "methods",
            "function_arguments",
            "actors",
        ],
    );
}

fn assert_sidecar_refused(path: &[Step], tagged: bool, fields: &[&str]) {
    let mut full = vec![Key("source_info")];
    full.extend_from_slice(path);
    assert_refused(
        Loader::Compilation,
        compilation_document(),
        &full,
        tagged,
        fields,
    );
}

#[test]
fn source_file_as_array() {
    assert_sidecar_refused(&[Key("sources"), Index(0)], false, &["name", "source"]);
}

#[test]
fn source_import_as_array() {
    assert_sidecar_refused(
        &[Key("imports"), Index(0)],
        false,
        &["from", "import", "to", "kind"],
    );
}

#[test]
fn source_declaration_as_array() {
    assert_sidecar_refused(
        &[Key("declarations"), Index(0)],
        false,
        &["source", "name", "type", "docs"],
    );
}

#[test]
fn field_label_as_array() {
    assert_sidecar_refused(
        &[Key("field_labels"), Index(0)],
        false,
        &["origin", "path", "container", "id", "label", "docs"],
    );
}

#[test]
fn declaration_origin_as_array() {
    assert_sidecar_refused(
        &[Key("field_labels"), Index(0), Key("origin")],
        true,
        &["source", "name"],
    );
}

#[test]
fn named_label_as_array() {
    let document = compilation_document();
    let named = document["source_info"]["field_labels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|label| label["label"]["kind"] == "named")
        .expect("a named label");
    assert_sidecar_refused(
        &[Key("field_labels"), Index(named), Key("label")],
        true,
        &["name"],
    );
}

#[test]
fn numeric_label_as_array() {
    let document = compilation_document();
    let numeric = document["source_info"]["field_labels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|label| label["label"]["kind"] == "numeric")
        .expect("a numeric label");
    assert_sidecar_refused(
        &[Key("field_labels"), Index(numeric), Key("label")],
        true,
        &[],
    );
}

#[test]
fn source_method_as_array() {
    assert_sidecar_refused(
        &[Key("methods"), Index(0)],
        false,
        &["origin", "path", "service", "name", "docs"],
    );
}

#[test]
fn actor_origin_as_array() {
    assert_sidecar_refused(
        &[Key("methods"), Index(0), Key("origin")],
        true,
        &["source"],
    );
}

#[test]
fn source_function_argument_as_array() {
    assert_sidecar_refused(
        &[Key("function_arguments"), Index(0)],
        false,
        &[
            "origin",
            "path",
            "function",
            "direction",
            "position",
            "name",
        ],
    );
}

#[test]
fn source_actor_as_array() {
    assert_sidecar_refused(&[Key("actors"), Index(0)], false, &["source", "docs"]);
}

// What the refusal does not touch.

/// A key spelled with an escape is the key serde reads: an escaped array key
/// still holds its array, and an escaped struct key still refuses one.
#[test]
fn escaped_keys_are_read_as_serde_reads_them() {
    let text =
        contract_document(ISSUE_DID)
            .to_string()
            .replacen("\"types\":", "\"typ\\u0065s\":", 1);
    assert!(text.contains("typ\\u0065s"));
    Contract::from_json(&text).expect("an escaped `types` key holds the arena");

    let mut document = contract_document(SERVICE_DID);
    let identities = document["identities"].clone();
    document["identities"] = sequence_form(&identities, false, &["contract", "interface"]);
    let text = document
        .to_string()
        .replacen("\"identities\":", "\"identiti\\u0065s\":", 1);
    assert!(text.contains("identiti\\u0065s"));
    assert_sequence_refused(Loader::Contract, &text, "$.identities");
}

/// Brackets, braces, commas and escaped quotes inside strings are text.
#[test]
fn string_contents_are_not_structure() {
    let mut document = contract_document(ISSUE_DID);
    document["producer"]["name"] = json!("[\"{],\\\"[[]\\");
    document["producer"]["version"] = json!("}]}]");
    // Read as ending at its escaped quote, this would open an array at
    // `candid_version`.
    document["producer"]["candid_version"] = json!("x\"[0]");
    Contract::from_json(&document.to_string()).expect("producer text is text");
}

/// The error names the array's line and column as serde would.
#[test]
fn the_error_names_line_and_column() {
    let mut document = contract_document(ISSUE_DID);
    *at(&mut document, &[Key("declarations"), Index(0)]) = json!(["R", 0]);
    let text = serde_json::to_string_pretty(&document).unwrap();
    let offset = text.find("[\n      \"R\"").expect("the array");
    let line = text[..offset].matches('\n').count() + 1;
    let column = offset - text[..offset].rfind('\n').map_or(0, |at| at + 1) + 1;
    assert_eq!(
        Contract::from_json(&text),
        Err(ContractJsonError::MalformedJson(format!(
            "invalid type: sequence, expected an object at $.declarations[0], \
             line {line} column {column}"
        )))
    );
}

/// A document the typed decode refuses keeps the decode's own error, even
/// when it also holds a struct in sequence form.
#[test]
fn a_decode_error_takes_precedence() {
    let mut document = contract_document(ISSUE_DID);
    *at(
        &mut document,
        &[Key("types"), Index(0), Key("fields"), Index(0)],
    ) = json!([97, 1]);
    document["format_version"] = json!("one");
    match Contract::from_json(&document.to_string()) {
        Err(ContractJsonError::MalformedJson(message)) => assert!(
            message.starts_with("invalid type: string \"one\", expected u32"),
            "{message}"
        ),
        other => panic!("expected the decode's error, got {other:?}"),
    }
}

// Every public loader entry point, end to end, on the issue's field repro.

fn issue_field_repro() -> Value {
    let mut document = contract_document(ISSUE_DID);
    *at(
        &mut document,
        &[Key("types"), Index(0), Key("fields"), Index(0)],
    ) = json!([97, 1]);
    document
}

fn assert_issue_refusal(entry: &str, result: Result<(), ContractJsonError>, path: &str) {
    let expected = format!("invalid type: sequence, expected an object at {path}, line ");
    match &result {
        Err(ContractJsonError::MalformedJson(message)) if message.starts_with(&expected) => {}
        _ => panic!("{entry} must refuse the repro; got {}", outcome(&result)),
    }
}

#[test]
fn loader_entry_points() {
    let limits = Limits::default();
    let context = RuntimeContext::default();

    let contract = issue_field_repro().to_string();
    let path = "$.types[0].fields[0]";
    for (entry, result) in [
        (
            "Contract::from_json",
            Contract::from_json(&contract).map(drop),
        ),
        (
            "Contract::from_json_with_limits",
            Contract::from_json_with_limits(&contract, &limits).map(drop),
        ),
        (
            "Contract::from_json_with_context",
            Contract::from_json_with_context(&contract, &context).map(drop),
        ),
        (
            "Contract::from_slice_with_limits",
            Contract::from_slice_with_limits(contract.as_bytes(), &limits).map(drop),
        ),
        (
            "Contract::from_slice_with_context",
            Contract::from_slice_with_context(contract.as_bytes(), &context).map(drop),
        ),
    ] {
        assert_issue_refusal(entry, result, path);
    }

    let mut envelope = envelope_document(ISSUE_DID);
    envelope["contract"] = issue_field_repro();
    let envelope = envelope.to_string();
    let path = "$.contract.types[0].fields[0]";
    for (entry, result) in [
        (
            "ContractEnvelope::from_json_with_limits",
            ContractEnvelope::from_json_with_limits(&envelope, &limits).map(drop),
        ),
        (
            "ContractEnvelope::from_json_with_context",
            ContractEnvelope::from_json_with_context(&envelope, &context).map(drop),
        ),
        (
            "ContractEnvelope::from_slice_with_limits",
            ContractEnvelope::from_slice_with_limits(envelope.as_bytes(), &limits).map(drop),
        ),
        (
            "ContractEnvelope::from_slice_with_context",
            ContractEnvelope::from_slice_with_context(envelope.as_bytes(), &context).map(drop),
        ),
    ] {
        assert_issue_refusal(entry, result, path);
    }

    let compilation = json!({ "contract": issue_field_repro() }).to_string();
    for (entry, result) in [
        (
            "Compilation::from_json_with_limits",
            Compilation::from_json_with_limits(&compilation, &limits).map(drop),
        ),
        (
            "Compilation::from_json_with_context",
            Compilation::from_json_with_context(&compilation, &context).map(drop),
        ),
        (
            "Compilation::from_slice_with_limits",
            Compilation::from_slice_with_limits(compilation.as_bytes(), &limits).map(drop),
        ),
        (
            "Compilation::from_slice_with_context",
            Compilation::from_slice_with_context(compilation.as_bytes(), &context).map(drop),
        ),
    ] {
        assert_issue_refusal(entry, result, path);
    }
}

/// The `candid-core validate` command loads through `Contract::from_json` and
/// reports the refusal as `malformed_contract_json`, the code a value of the
/// wrong type gets.
#[cfg(feature = "filesystem-compiler")]
#[test]
fn validate_command_refuses_the_repro() {
    let directory = std::env::temp_dir().join(format!(
        "candid-core-struct-sequence-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("repro.contract.json");
    std::fs::write(&path, issue_field_repro().to_string()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_candid-core"))
        .arg("validate")
        .arg(&path)
        .output()
        .expect("the binary runs");
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let response: Value = serde_json::from_slice(&output.stdout).expect("a JSON response");
    let diagnostic = &response["diagnostics"][0];
    assert_eq!(response["ok"], false);
    assert_eq!(diagnostic["code"], "malformed_contract_json");
    assert!(
        diagnostic["message"]
            .as_str()
            .expect("a message")
            .starts_with("invalid type: sequence, expected an object at $.types[0].fields[0], "),
        "{diagnostic}"
    );
}
