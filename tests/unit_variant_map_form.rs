//! Issue #238: the bounded loaders refuse serde's map form of a unit variant.
//!
//! Serde's derived `Deserialize` reads a unit variant of an externally tagged
//! enum written as a map from the variant's name to a unit, as well as the
//! string the Contract format writes: `{"nat": null}` for `"nat"`. Inside the
//! buffered content of an internally tagged enum (a type node's `primitive`
//! and a func's `mode`), serde also reads `{"nat": {}}`. The format writes
//! every unit variant as a string: candid-core writes nothing else, and
//! `@candid-core/schema`'s `schemaFromContract` refuses the map form. These
//! tests write each unit-variant position of every document a bounded loader
//! reads (Contract, envelope, Compilation and its `SourceInfo` sidecar) in
//! each spelling, otherwise unchanged. A spelling serde read before is now
//! refused with the error a value of the wrong type gets:
//! `ContractJsonError::MalformedJson`, `invalid type: map, expected a string
//! at <path>`. A spelling serde refused keeps serde's own error. The string
//! form of the same document must load, so each refusal is the spelling's
//! alone. The issue's two spellings carry its names,
//! `contract_primitive_as_map_of_null` and
//! `contract_primitive_as_map_of_empty_object`.
//!
//! The unit-variant positions, found by reading every DTO the loaders decode:
//! a type node's `primitive` (`PrimitiveType`) and a func's `mode`
//! (`MethodMode`) in the Contract, the envelope's and the Compilation's
//! `contract`; an import's `kind` (`SourceImportKind`) and a function
//! argument's `direction` (`SourceFunctionArgumentDirection`) in the
//! `SourceInfo` sidecar. The internally tagged enums' `kind` tags (type node,
//! actor, `origin`, `label`) are not unit variants: serde reads a tag only as
//! a string, so a map there was refused before and keeps that error.

use candid_core::{
    compile_did, compile_with_resolver, Compilation, CompileOptions, Contract, ContractEnvelope,
    ContractJsonError, Limits, MemoryResolver, RuntimeContext,
};
use serde_json::{json, Value};

/// The issue's environment: `R` is `types[0]`, `nat` is `types[1]`.
const ISSUE_DID: &str = "type R = record { a : nat; b : nat };\n";

/// A primitive node and a func node, whose mode is `query`.
const KINDS_DID: &str = "type R = record { a : nat; b : nat };
type V = variant { x; y : text };
service : (nat) -> { m : (R) -> (V) query }
";

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

/// An envelope whose free-form extension holds the map form at a `mode` key,
/// which stays free-form.
fn envelope_document(source: &str) -> Value {
    let contract = compile_did(source)
        .expect("the source compiles")
        .contract()
        .clone();
    let mut envelope = ContractEnvelope::new(contract);
    envelope
        .insert_extension(
            "org.example.free/v1",
            json!({"mode": {"query": null}, "primitive": {"nat": {}}}),
            &Limits::default(),
        )
        .expect("the extension is valid");
    let json = envelope
        .to_json_pretty_with_limits(&Limits::default())
        .expect("the envelope serializes");
    serde_json::from_str(&json).expect("a JSON object")
}

/// A two-source bundle whose sidecar holds an import and function arguments.
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
    for collection in ["imports", "function_arguments"] {
        assert!(
            !document["source_info"][collection]
                .as_array()
                .expect(collection)
                .is_empty(),
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

/// A `$`-rooted path step.
#[derive(Clone)]
enum Step {
    Key(&'static str),
    Index(usize),
}

use Step::{Index, Key};

fn at<'a>(document: &'a mut Value, path: &[Step]) -> &'a mut Value {
    path.iter().fold(document, |value, step| match step {
        Key(key) => value
            .get_mut(*key)
            .unwrap_or_else(|| panic!("no key {key}")),
        Index(index) => value
            .get_mut(*index)
            .unwrap_or_else(|| panic!("no index {index}")),
    })
}

fn render(path: &[Step]) -> String {
    let mut rendered = String::from("$");
    for step in path {
        match step {
            Key(key) => rendered.push_str(&format!(".{key}")),
            Index(index) => rendered.push_str(&format!("[{index}]")),
        }
    }
    rendered
}

/// The arena index of the first type node of `kind` in `types`.
fn node(types: &Value, kind: &str) -> usize {
    types
        .as_array()
        .expect("types")
        .iter()
        .position(|node| node["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} node"))
}

/// `document` serialized with the value at `path` replaced by `raw`, a JSON
/// text spelled exactly as given.
fn with_raw(mut document: Value, path: &[Step], raw: &str) -> String {
    const MARKER: &str = "__unit_variant_map_form__";
    *at(&mut document, path) = json!(MARKER);
    let text = document.to_string();
    let quoted = format!("\"{MARKER}\"");
    assert_eq!(text.matches(&quoted).count(), 1);
    text.replacen(&quoted, raw, 1)
}

fn outcome(result: &Result<(), ContractJsonError>) -> String {
    match result {
        Ok(()) => "accepted".to_string(),
        Err(error) => format!("{error:?}"),
    }
}

/// How serde reads the enum at a position.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    /// Through the buffered content of an internally tagged enum (a type
    /// node's fields), which also reads `{}` as a unit.
    Buffered,
    /// Straight from the JSON text, which reads only `null` as a unit.
    Direct,
}

/// Every unit-variant position, through each loader that reads it.
#[derive(Clone, Copy, Debug)]
enum Position {
    ContractPrimitive,
    ContractMode,
    EnvelopePrimitive,
    EnvelopeMode,
    CompilationPrimitive,
    CompilationMode,
    ImportKind,
    ArgumentDirection,
}

struct Site {
    loader: Loader,
    document: Value,
    path: Vec<Step>,
    read: Read,
    /// The variant the document writes there.
    name: String,
    /// Another variant of the same enum.
    other: &'static str,
}

impl Position {
    fn site(self) -> Site {
        let contract_site = |loader, document: Value, prefix: &[Step], kind, key| {
            let types = prefix
                .iter()
                .fold(&document, |value, step| match step {
                    Key(key) => &value[*key],
                    Index(index) => &value[*index],
                })
                .get("types")
                .expect("types");
            let index = node(types, kind);
            let name = types[index][key].as_str().expect("a string").to_string();
            let mut path = prefix.to_vec();
            path.extend([Key("types"), Index(index), Key(key)]);
            let other = match key {
                "primitive" if name == "int" => "nat",
                "primitive" => "int",
                _ if name == "update" => "oneway",
                _ => "update",
            };
            Site {
                loader,
                document,
                path,
                read: Read::Buffered,
                name,
                other,
            }
        };
        let sidecar_site = |collection, key, other_of: fn(&str) -> &'static str| {
            let document = compilation_document();
            let name = document["source_info"][collection][0][key]
                .as_str()
                .expect("a string")
                .to_string();
            let other = other_of(&name);
            Site {
                loader: Loader::Compilation,
                document,
                path: vec![Key("source_info"), Key(collection), Index(0), Key(key)],
                read: Read::Direct,
                name,
                other,
            }
        };
        match self {
            Self::ContractPrimitive => contract_site(
                Loader::Contract,
                contract_document(KINDS_DID),
                &[],
                "primitive",
                "primitive",
            ),
            Self::ContractMode => contract_site(
                Loader::Contract,
                contract_document(KINDS_DID),
                &[],
                "func",
                "mode",
            ),
            Self::EnvelopePrimitive => contract_site(
                Loader::Envelope,
                envelope_document(KINDS_DID),
                &[Key("contract")],
                "primitive",
                "primitive",
            ),
            Self::EnvelopeMode => contract_site(
                Loader::Envelope,
                envelope_document(KINDS_DID),
                &[Key("contract")],
                "func",
                "mode",
            ),
            Self::CompilationPrimitive => contract_site(
                Loader::Compilation,
                compilation_document(),
                &[Key("contract")],
                "primitive",
                "primitive",
            ),
            Self::CompilationMode => contract_site(
                Loader::Compilation,
                compilation_document(),
                &[Key("contract")],
                "func",
                "mode",
            ),
            Self::ImportKind => sidecar_site("imports", "kind", |name| {
                if name == "type" {
                    "service"
                } else {
                    "type"
                }
            }),
            Self::ArgumentDirection => sidecar_site("function_arguments", "direction", |name| {
                if name == "argument" {
                    "result"
                } else {
                    "argument"
                }
            }),
        }
    }
}

/// Each spelling checked at each position.
#[derive(Clone, Copy, Debug)]
enum Spelling {
    /// `{"v": null}`: serde's map form, read at every position.
    MapOfNull,
    /// `{"v": {}}`: read inside a tagged enum's buffered content.
    MapOfEmptyObject,
    /// `{"v": {"x": 1}}`.
    MapOfObject,
    /// `{"v": []}`.
    MapOfEmptyArray,
    /// `{"v": [null]}`.
    MapOfArray,
    /// `{"v": 0}`.
    MapOfNumber,
    /// `{"v": false}`.
    MapOfBoolean,
    /// `{"v": ""}`.
    MapOfString,
    /// `{"\u00XXrest": null}`: the variant's name written with an escape.
    MapWithEscapedName,
    /// `{ "v" : null }`: whitespace inside the map.
    MapWithWhitespace,
    /// `{"w": null}` for another variant `w` of the same enum.
    MapOfOtherVariant,
    /// `{"bogus": null}`.
    MapOfUnknownVariant,
    /// `{"v": null, "v": null}`.
    MapOfTwoEntries,
    /// `{}`.
    EmptyMap,
    /// `["v"]`: the sequence form, refused by serde here.
    SequenceOfName,
}

impl Spelling {
    fn raw(self, name: &str, other: &str) -> String {
        let quoted = serde_json::to_string(name).unwrap();
        match self {
            Self::MapOfNull => format!("{{{quoted}:null}}"),
            Self::MapOfEmptyObject => format!("{{{quoted}:{{}}}}"),
            Self::MapOfObject => format!("{{{quoted}:{{\"x\":1}}}}"),
            Self::MapOfEmptyArray => format!("{{{quoted}:[]}}"),
            Self::MapOfArray => format!("{{{quoted}:[null]}}"),
            Self::MapOfNumber => format!("{{{quoted}:0}}"),
            Self::MapOfBoolean => format!("{{{quoted}:false}}"),
            Self::MapOfString => format!("{{{quoted}:\"\"}}"),
            Self::MapWithEscapedName => {
                format!("{{\"\\u{:04x}{}\":null}}", name.as_bytes()[0], &name[1..])
            }
            Self::MapWithWhitespace => format!("{{ {quoted} : null }}"),
            Self::MapOfOtherVariant => format!("{{\"{other}\":null}}"),
            Self::MapOfUnknownVariant => "{\"bogus\":null}".to_string(),
            Self::MapOfTwoEntries => format!("{{{quoted}:null,{quoted}:null}}"),
            Self::EmptyMap => "{}".to_string(),
            Self::SequenceOfName => format!("[{quoted}]"),
        }
    }

    /// `None` when serde reads this spelling as the variant, which the
    /// loaders now refuse; otherwise the start of the error serde gives it,
    /// which it keeps.
    fn serde_error(self, read: Read) -> Option<&'static str> {
        match (self, read) {
            (Self::MapOfNull | Self::MapWithEscapedName | Self::MapWithWhitespace, _)
            | (Self::MapOfOtherVariant, _)
            | (Self::MapOfEmptyObject, Read::Buffered) => None,
            (Self::MapOfEmptyObject | Self::MapOfObject, _) => {
                Some("invalid type: map, expected unit at ")
            }
            (Self::MapOfEmptyArray | Self::MapOfArray, _) => {
                Some("invalid type: sequence, expected unit at ")
            }
            (Self::MapOfNumber, _) => Some("invalid type: integer `0`, expected unit at "),
            (Self::MapOfBoolean, _) => Some("invalid type: boolean `false`, expected unit at "),
            (Self::MapOfString, _) => Some("invalid type: string \"\", expected unit at "),
            (Self::MapOfUnknownVariant, _) => Some("unknown variant `bogus`, expected "),
            (Self::MapOfTwoEntries | Self::EmptyMap, Read::Buffered) => {
                Some("invalid value: map, expected map with a single key at ")
            }
            (Self::SequenceOfName, Read::Buffered) => {
                Some("invalid type: sequence, expected string or map at ")
            }
            (Self::MapOfTwoEntries | Self::EmptyMap | Self::SequenceOfName, Read::Direct) => {
                Some("expected value at ")
            }
        }
    }
}

fn assert_map_refused(loader: Loader, text: &str, path: &str) {
    let result = loader.load(text);
    let expected = format!("invalid type: map, expected a string at {path}, line ");
    match &result {
        Err(ContractJsonError::MalformedJson(message)) if message.starts_with(&expected) => {}
        _ => panic!(
            "{loader:?}: a unit variant written as a map at {path} must be refused with \
             `{expected}…`; got {}",
            outcome(&result)
        ),
    }
}

/// Write `spelling` at `position` and require the outcome its `serde_error`
/// names, after requiring the string form to load.
fn check(position: Position, spelling: Spelling) {
    let Site {
        loader,
        document,
        path,
        read,
        name,
        other,
    } = position.site();
    let string_form = document.to_string();
    assert!(
        loader.load(&string_form).is_ok(),
        "{position:?}: the string form must load: {}",
        outcome(&loader.load(&string_form))
    );
    let text = with_raw(document, &path, &spelling.raw(&name, other));
    match spelling.serde_error(read) {
        None => assert_map_refused(loader, &text, &render(&path)),
        Some(prefix) => {
            let result = loader.load(&text);
            match &result {
                Err(ContractJsonError::MalformedJson(message))
                    if message.starts_with(prefix) && !message.contains("expected a string") => {}
                _ => panic!(
                    "{position:?} {spelling:?}: serde's own error `{prefix}…` must stand; got {}",
                    outcome(&result)
                ),
            }
        }
    }
}

macro_rules! positions {
    ($($module:ident: $position:ident;)*) => {$(
        mod $module {
            use super::{check, Position, Spelling};

            #[test]
            fn map_of_null() {
                check(Position::$position, Spelling::MapOfNull);
            }

            #[test]
            fn map_of_empty_object() {
                check(Position::$position, Spelling::MapOfEmptyObject);
            }

            #[test]
            fn map_of_object() {
                check(Position::$position, Spelling::MapOfObject);
            }

            #[test]
            fn map_of_empty_array() {
                check(Position::$position, Spelling::MapOfEmptyArray);
            }

            #[test]
            fn map_of_array() {
                check(Position::$position, Spelling::MapOfArray);
            }

            #[test]
            fn map_of_number() {
                check(Position::$position, Spelling::MapOfNumber);
            }

            #[test]
            fn map_of_boolean() {
                check(Position::$position, Spelling::MapOfBoolean);
            }

            #[test]
            fn map_of_string() {
                check(Position::$position, Spelling::MapOfString);
            }

            #[test]
            fn map_with_escaped_name() {
                check(Position::$position, Spelling::MapWithEscapedName);
            }

            #[test]
            fn map_with_whitespace() {
                check(Position::$position, Spelling::MapWithWhitespace);
            }

            #[test]
            fn map_of_other_variant() {
                check(Position::$position, Spelling::MapOfOtherVariant);
            }

            #[test]
            fn map_of_unknown_variant() {
                check(Position::$position, Spelling::MapOfUnknownVariant);
            }

            #[test]
            fn map_of_two_entries() {
                check(Position::$position, Spelling::MapOfTwoEntries);
            }

            #[test]
            fn empty_map() {
                check(Position::$position, Spelling::EmptyMap);
            }

            #[test]
            fn sequence_of_name() {
                check(Position::$position, Spelling::SequenceOfName);
            }
        }
    )*};
}

positions! {
    contract_primitive: ContractPrimitive;
    contract_mode: ContractMode;
    envelope_primitive: EnvelopePrimitive;
    envelope_mode: EnvelopeMode;
    compilation_primitive: CompilationPrimitive;
    compilation_mode: CompilationMode;
    sidecar_import_kind: ImportKind;
    sidecar_argument_direction: ArgumentDirection;
}

// The issue's two spellings, exactly as #238 states them.

fn issue_repro(value: Value) -> String {
    let mut document = contract_document(ISSUE_DID);
    assert_eq!(
        document["types"][1],
        json!({"kind": "primitive", "primitive": "nat"})
    );
    document["types"][1]["primitive"] = value;
    document.to_string()
}

#[test]
fn contract_primitive_as_map_of_null() {
    assert_map_refused(
        Loader::Contract,
        &issue_repro(json!({"nat": null})),
        "$.types[1].primitive",
    );
}

#[test]
fn contract_primitive_as_map_of_empty_object() {
    assert_map_refused(
        Loader::Contract,
        &issue_repro(json!({"nat": {}})),
        "$.types[1].primitive",
    );
}

/// Every variant of each Contract enum: its map form is refused, never
/// decoded. The identities would not match a changed variant, but the map
/// form is refused before validation runs.
#[test]
fn every_primitive_and_mode_in_map_form() {
    for primitive in [
        "null",
        "bool",
        "nat",
        "int",
        "nat8",
        "nat16",
        "nat32",
        "nat64",
        "int8",
        "int16",
        "int32",
        "int64",
        "float32",
        "float64",
        "text",
        "reserved",
        "empty",
        "principal",
    ] {
        for unit in [json!(null), json!({})] {
            assert_map_refused(
                Loader::Contract,
                &issue_repro(json!({ primitive: unit })),
                "$.types[1].primitive",
            );
        }
    }
    let document = contract_document(KINDS_DID);
    let func = node(&document["types"], "func");
    for mode in ["update", "query", "composite_query", "oneway"] {
        for unit in [json!(null), json!({})] {
            let mut edited = document.clone();
            edited["types"][func]["mode"] = json!({ mode: unit });
            assert_map_refused(
                Loader::Contract,
                &edited.to_string(),
                &format!("$.types[{func}].mode"),
            );
        }
    }
}

// What the refusal does not touch.

/// The internally tagged enums' `kind` tags are read only as a string, so a
/// map there was refused before this change, with serde's own error, and
/// still is.
#[test]
fn a_tag_written_as_a_map_keeps_the_decode_error() {
    let expected = "invalid type: map, expected variant identifier at ";
    let contract = contract_document(KINDS_DID);
    let primitive = node(&contract["types"], "primitive");
    let compilation = compilation_document();
    let numeric = compilation["source_info"]["field_labels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|label| label["label"]["kind"] == "numeric")
        .expect("a numeric label");
    for (loader, document, path, tag) in [
        (
            Loader::Contract,
            contract.clone(),
            vec![Key("types"), Index(primitive), Key("kind")],
            "primitive",
        ),
        (
            Loader::Contract,
            contract,
            vec![Key("actor"), Key("kind")],
            "class",
        ),
        (
            Loader::Compilation,
            compilation.clone(),
            vec![
                Key("source_info"),
                Key("field_labels"),
                Index(0),
                Key("origin"),
                Key("kind"),
            ],
            "declaration",
        ),
        (
            Loader::Compilation,
            compilation,
            vec![
                Key("source_info"),
                Key("field_labels"),
                Index(numeric),
                Key("label"),
                Key("kind"),
            ],
            "numeric",
        ),
    ] {
        let text = with_raw(document, &path, &format!("{{\"{tag}\":null}}"));
        match loader.load(&text) {
            Err(ContractJsonError::MalformedJson(message)) if message.starts_with(expected) => {}
            other => panic!(
                "{}: expected the decode's error, got {other:?}",
                render(&path)
            ),
        }
    }
}

/// Object-valued keys, envelope extension values and escaped keys are read
/// as serde reads them: an escaped object key still holds its object, an
/// escaped unit key still refuses a map, and extension values stay
/// free-form.
#[test]
fn object_keys_and_extensions_are_untouched() {
    let text = contract_document(ISSUE_DID).to_string().replacen(
        "\"producer\":",
        "\"pr\\u006fducer\":",
        1,
    );
    assert!(text.contains("pr\\u006fducer"));
    Contract::from_json(&text).expect("an escaped `producer` key holds its object");

    let document = contract_document(KINDS_DID);
    let func = node(&document["types"], "func");
    let text = with_raw(
        document,
        &[Key("types"), Index(func), Key("mode")],
        "{\"query\":null}",
    )
    .replacen("\"mode\":", "\"m\\u006fde\":", 1);
    assert!(text.contains("m\\u006fde"));
    assert_map_refused(Loader::Contract, &text, &format!("$.types[{func}].mode"));

    let envelope = envelope_document(KINDS_DID);
    assert_eq!(
        envelope["extensions"]["org.example.free/v1"]["mode"],
        json!({"query": null})
    );
    ContractEnvelope::from_json_with_context(&envelope.to_string(), &RuntimeContext::default())
        .expect("free-form extension values load");
}

/// The error names the map's line and column as serde would.
#[test]
fn the_error_names_line_and_column() {
    let mut document = contract_document(ISSUE_DID);
    document["types"][1]["primitive"] = json!({"nat": null});
    let text = serde_json::to_string_pretty(&document).unwrap();
    let offset = text.find("{\n        \"nat\"").expect("the map");
    let line = text[..offset].matches('\n').count() + 1;
    let column = offset - text[..offset].rfind('\n').map_or(0, |at| at + 1) + 1;
    assert_eq!(
        Contract::from_json(&text),
        Err(ContractJsonError::MalformedJson(format!(
            "invalid type: map, expected a string at $.types[1].primitive, \
             line {line} column {column}"
        )))
    );
}

// Every public loader entry point, end to end.

fn assert_entry_refusal(entry: &str, result: Result<(), ContractJsonError>, path: &str) {
    let expected = format!("invalid type: map, expected a string at {path}, line ");
    match &result {
        Err(ContractJsonError::MalformedJson(message)) if message.starts_with(&expected) => {}
        _ => panic!("{entry} must refuse the repro; got {}", outcome(&result)),
    }
}

#[test]
fn loader_entry_points() {
    let limits = Limits::default();
    let context = RuntimeContext::default();
    let repro: Value = serde_json::from_str(&issue_repro(json!({"nat": null}))).unwrap();

    let contract = repro.to_string();
    let path = "$.types[1].primitive";
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
        assert_entry_refusal(entry, result, path);
    }

    let mut envelope = envelope_document(ISSUE_DID);
    envelope["contract"] = repro.clone();
    let envelope = envelope.to_string();
    let path = "$.contract.types[1].primitive";
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
        assert_entry_refusal(entry, result, path);
    }

    let compilation = json!({ "contract": repro }).to_string();
    let mut sidecar = compilation_document();
    sidecar["source_info"]["imports"][0]["kind"] = json!({"type": null});
    let sidecar = sidecar.to_string();
    for (input, path) in [
        (&compilation, "$.contract.types[1].primitive"),
        (&sidecar, "$.source_info.imports[0].kind"),
    ] {
        for (entry, result) in [
            (
                "Compilation::from_json_with_limits",
                Compilation::from_json_with_limits(input, &limits).map(drop),
            ),
            (
                "Compilation::from_json_with_context",
                Compilation::from_json_with_context(input, &context).map(drop),
            ),
            (
                "Compilation::from_slice_with_limits",
                Compilation::from_slice_with_limits(input.as_bytes(), &limits).map(drop),
            ),
            (
                "Compilation::from_slice_with_context",
                Compilation::from_slice_with_context(input.as_bytes(), &context).map(drop),
            ),
        ] {
            assert_entry_refusal(entry, result, path);
        }
    }
}

/// The `candid-core validate` command loads through `Contract::from_json` and
/// reports the refusal as `malformed_contract_json`, the code a value of the
/// wrong type gets.
#[cfg(feature = "filesystem-compiler")]
#[test]
fn validate_command_refuses_the_repro() {
    let directory = std::env::temp_dir().join(format!(
        "candid-core-unit-variant-map-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("repro.contract.json");
    std::fs::write(&path, issue_repro(json!({"nat": null}))).unwrap();
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
            .starts_with("invalid type: map, expected a string at $.types[1].primitive, "),
        "{diagnostic}"
    );
}
