use candid_core::{
    compile_did, compile_did_file, compile_did_with_options, compile_with_resolver, Actor,
    CompileOptions, Contract, Declaration, Field, Limits, MemoryResolver, MethodMode,
    PrimitiveType, RawContract, RuntimeContext, ServiceMethod, SourceLabel, SourceOrigin, TypeNode,
};
use std::collections::{BTreeMap, BTreeSet};

fn compile(source: &str) -> candid_core::Compilation {
    compile_did(source).unwrap_or_else(|error| panic!("compilation failed: {error:#?}"))
}

fn declaration(contract: &Contract, name: &str) -> u32 {
    contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == name)
        .unwrap_or_else(|| panic!("missing declaration {name}"))
        .ty
}

fn raw_declaration(contract: &RawContract, name: &str) -> u32 {
    contract
        .declarations
        .iter()
        .find(|declaration| declaration.name == name)
        .unwrap_or_else(|| panic!("missing raw declaration {name}"))
        .ty
}

fn service_methods(contract: &Contract) -> &Vec<candid_core::ServiceMethod> {
    let Actor::Service { service } = contract.actor().as_ref().expect("expected actor") else {
        panic!("expected service actor")
    };
    let TypeNode::Service { methods } = &contract.types()[*service as usize] else {
        panic!("actor reference is not a service")
    };
    methods
}

fn primitive_set(contract: &Contract) -> BTreeSet<PrimitiveType> {
    contract
        .types()
        .iter()
        .filter_map(|node| match node {
            TypeNode::Primitive { primitive } => Some(*primitive),
            _ => None,
        })
        .collect()
}

#[test]
fn lowers_every_candid_primitive_without_host_special_cases() {
    let compilation = compile(
        r#"
        type All = record {
          a: null; b: bool; c: nat; d: int;
          e: nat8; f: nat16; g: nat32; h: nat64;
          i: int8; j: int16; k: int32; l: int64;
          m: float32; n: float64; o: text; p: reserved; q: empty; r: principal;
        };
        service : { inspect: (All) -> () };
        "#,
    );
    let expected = BTreeSet::from([
        PrimitiveType::Null,
        PrimitiveType::Bool,
        PrimitiveType::Nat,
        PrimitiveType::Int,
        PrimitiveType::Nat8,
        PrimitiveType::Nat16,
        PrimitiveType::Nat32,
        PrimitiveType::Nat64,
        PrimitiveType::Int8,
        PrimitiveType::Int16,
        PrimitiveType::Int32,
        PrimitiveType::Int64,
        PrimitiveType::Float32,
        PrimitiveType::Float64,
        PrimitiveType::Text,
        PrimitiveType::Reserved,
        PrimitiveType::Empty,
        PrimitiveType::Principal,
    ]);
    assert_eq!(primitive_set(compilation.contract()), expected);
    assert!(compilation.contract().validate().is_ok());
}

#[test]
fn aliases_are_provenance_and_resolve_to_direct_semantic_edges() {
    let compilation = compile(
        r#"
        // A wire user.
        type User = record { id: nat; };
        // Same semantic type, distinct source alias.
        type UserAlias = User;
        service : { echo: (input: UserAlias) -> (output: User) query };
        "#,
    );
    let contract = compilation.contract();
    assert_eq!(
        declaration(contract, "User"),
        declaration(contract, "UserAlias")
    );
    let method = &service_methods(contract)[0];
    let TypeNode::Func { args, results, .. } = &contract.types()[method.function as usize] else {
        panic!("method must be a function")
    };
    assert_eq!(args, results);
    assert!(!contract.to_json_pretty().unwrap().contains("input"));
    assert!(!contract.to_json_pretty().unwrap().contains("output"));
    assert!(!contract.to_json_pretty().unwrap().contains("wire user"));

    let source_info = compilation.source_info().expect("source sidecar");
    assert!(source_info
        .declarations()
        .iter()
        .any(|entry| entry.name == "User"));
    assert!(source_info
        .sources()
        .iter()
        .any(|source| source.source.contains("Same semantic type")));
}

#[test]
fn labels_keep_authoritative_ids_and_source_spelling_is_sidecar_only() {
    let compilation = compile(
        r#"
        type Labels = record { foo: text; 42: nat; "hyphen-name": bool };
        service : { accept: (Labels) -> () };
        "#,
    );
    let labels = declaration(compilation.contract(), "Labels");
    let TypeNode::Record { fields } = &compilation.contract().types()[labels as usize] else {
        panic!("Labels must be a record")
    };
    let ids: BTreeSet<_> = fields.iter().map(|field| field.id).collect();
    assert!(ids.contains(&5_097_222));
    assert!(ids.contains(&42));
    assert!(ids.contains(&candid_parser::candid::idl_hash("hyphen-name")));
    assert_eq!(fields.iter().map(|field| field.id).collect::<Vec<_>>(), {
        let mut sorted = fields.iter().map(|field| field.id).collect::<Vec<_>>();
        sorted.sort_unstable();
        sorted
    });

    let source_info = compilation.source_info().unwrap();
    assert!(source_info.field_labels().iter().any(|label| {
        label.container == labels
            && label.id == 5_097_222
            && label.label
                == SourceLabel::Named {
                    name: "foo".to_string(),
                }
    }));
    assert!(source_info.field_labels().iter().any(|label| {
        label.container == labels && label.id == 42 && label.label == SourceLabel::Numeric
    }));
}

#[test]
fn source_occurrences_survive_semantic_interning() {
    let compilation = compile(
        r#"
        type Named = record { foo: nat };
        type Numeric = record { 5097222: nat };
        type Tuple = record { nat };
        type Explicit = record { 0: nat };
        type Both = record { a: record { 0: nat }; b: record { nat } };
        service : { echo: (input: Named) -> (output: Numeric) };
        "#,
    );
    assert_eq!(
        declaration(compilation.contract(), "Named"),
        declaration(compilation.contract(), "Numeric")
    );
    assert_eq!(
        declaration(compilation.contract(), "Tuple"),
        declaration(compilation.contract(), "Explicit")
    );
    let source_info = compilation.source_info().unwrap();
    let named = source_info.field_labels().iter().find(|field| {
        field.origin
            == SourceOrigin::Declaration {
                source: "memory:/inline.did".to_string(),
                name: "Named".to_string(),
            }
    });
    assert!(matches!(
        named.map(|field| &field.label),
        Some(SourceLabel::Named { name }) if name == "foo"
    ));
    let numeric = source_info.field_labels().iter().find(|field| {
        field.origin
            == SourceOrigin::Declaration {
                source: "memory:/inline.did".to_string(),
                name: "Numeric".to_string(),
            }
    });
    assert!(matches!(
        numeric.map(|field| &field.label),
        Some(SourceLabel::Numeric)
    ));
    let tuple = source_info.field_labels().iter().find(|field| {
        field.origin
            == SourceOrigin::Declaration {
                source: "memory:/inline.did".to_string(),
                name: "Tuple".to_string(),
            }
    });
    assert!(matches!(
        tuple.map(|field| &field.label),
        Some(SourceLabel::Positional)
    ));
    let explicit = source_info.field_labels().iter().find(|field| {
        field.origin
            == SourceOrigin::Declaration {
                source: "memory:/inline.did".to_string(),
                name: "Explicit".to_string(),
            }
    });
    assert!(matches!(
        explicit.map(|field| &field.label),
        Some(SourceLabel::Numeric)
    ));
    assert!(source_info
        .function_arguments()
        .iter()
        .any(|argument| argument.name == "input"));
    assert!(source_info
        .function_arguments()
        .iter()
        .any(|argument| argument.name == "output"));
    let nested_labels: Vec<_> = source_info
        .field_labels()
        .iter()
        .filter(|field| {
            matches!(
                &field.origin,
                SourceOrigin::Declaration { name, .. } if name == "Both"
            ) && field.id == 0
        })
        .collect();
    assert_eq!(nested_labels.len(), 2);
    assert_ne!(nested_labels[0].path, nested_labels[1].path);
    assert!(nested_labels
        .iter()
        .any(|field| field.label == SourceLabel::Numeric));
    assert!(nested_labels
        .iter()
        .any(|field| field.label == SourceLabel::Positional));
}

#[test]
fn tuple_and_explicit_numbered_record_have_the_same_wire_contract() {
    let tuple = compile(
        r#"
        type Shape = record { text; nat };
        service : { proof: (Shape) -> (Shape) };
        "#,
    );
    let explicit = compile(
        r#"
        type Renamed = record { 0: text; 1: nat };
        service : { proof: (Renamed) -> (Renamed) };
        "#,
    );
    assert_eq!(
        tuple.contract().interface_id(),
        explicit.contract().interface_id()
    );
    let shape = declaration(tuple.contract(), "Shape");
    let TypeNode::Record { fields } = &tuple.contract().types()[shape as usize] else {
        panic!("Shape must be a record")
    };
    assert_eq!(
        fields.iter().map(|field| field.id).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(tuple
        .contract()
        .types()
        .iter()
        .all(|node| !format!("{node:?}").contains("Tuple")));

    let tuple_labels = tuple.source_info().unwrap().field_labels();
    assert!(tuple_labels
        .iter()
        .any(|label| label.container == shape && label.label == SourceLabel::Positional));
}

#[test]
fn blob_is_only_vec_nat8_in_the_contract() {
    let blob = compile(
        r#"
        type Bytes = blob;
        service : { get: () -> (Bytes) };
        "#,
    );
    let vec = compile(
        r#"
        type Bytes = vec nat8;
        service : { get: () -> (Bytes) };
        "#,
    );
    assert_eq!(blob.contract().contract_id(), vec.contract().contract_id());
    assert!(!blob.contract().to_json_pretty().unwrap().contains("blob"));
    let bytes = declaration(blob.contract(), "Bytes");
    let TypeNode::Vec { inner } = &blob.contract().types()[bytes as usize] else {
        panic!("blob must lower to vec")
    };
    assert!(matches!(
        blob.contract().types()[*inner as usize],
        TypeNode::Primitive {
            primitive: PrimitiveType::Nat8
        }
    ));
}

#[test]
fn conventional_result_variants_remain_plain_variants() {
    let compilation = compile(
        r#"
        type Outcome = variant { ok: nat; err: text };
        service : { run: () -> (Outcome) };
        "#,
    );
    let outcome = declaration(compilation.contract(), "Outcome");
    assert!(matches!(
        compilation.contract().types()[outcome as usize],
        TypeNode::Variant { .. }
    ));
    assert!(!compilation
        .contract()
        .to_json_pretty()
        .unwrap()
        .contains("\"kind\": \"result\""));
}

#[test]
fn self_and_mutual_recursion_are_direct_graph_cycles() {
    let self_recursive = compile(
        r#"
        type List = opt record { head: nat; tail: List };
        service : { get: () -> (List) };
        "#,
    );
    let list = declaration(self_recursive.contract(), "List");
    let TypeNode::Opt { inner } = &self_recursive.contract().types()[list as usize] else {
        panic!("List must start with opt")
    };
    let TypeNode::Record { fields } = &self_recursive.contract().types()[*inner as usize] else {
        panic!("List opt must contain a record")
    };
    let tail = fields
        .iter()
        .find(|field| field.id == candid_parser::candid::idl_hash("tail"))
        .unwrap();
    assert_eq!(tail.ty, list);
    assert!(self_recursive.contract().validate().is_ok());

    let mutual = compile(
        r#"
        type A = record { b: opt B };
        type B = variant { stop; more: A };
        service : { get: () -> (A) };
        "#,
    );
    let a = declaration(mutual.contract(), "A");
    let b = declaration(mutual.contract(), "B");
    let TypeNode::Record { fields } = &mutual.contract().types()[a as usize] else {
        panic!("A must be a record")
    };
    let TypeNode::Opt { inner } = &mutual.contract().types()[fields[0].ty as usize] else {
        panic!("A.b must be opt")
    };
    assert_eq!(*inner, b);
    let TypeNode::Variant { fields } = &mutual.contract().types()[b as usize] else {
        panic!("B must be a variant")
    };
    let more = fields
        .iter()
        .find(|field| field.id == candid_parser::candid::idl_hash("more"))
        .unwrap();
    assert_eq!(more.ty, a);
}

#[test]
fn function_and_service_references_are_first_class_type_nodes() {
    let compilation = compile(
        r#"
        type Callback = func (text) -> (nat) query;
        type Directory = service { lookup: (text) -> (Callback) query };
        type Envelope = record { callback: Callback; directory: Directory };
        service : { accept: (Envelope) -> (Directory) };
        "#,
    );
    let callback = declaration(compilation.contract(), "Callback");
    let directory = declaration(compilation.contract(), "Directory");
    let envelope = declaration(compilation.contract(), "Envelope");
    assert!(matches!(
        compilation.contract().types()[callback as usize],
        TypeNode::Func { .. }
    ));
    assert!(matches!(
        compilation.contract().types()[directory as usize],
        TypeNode::Service { .. }
    ));
    let TypeNode::Record { fields } = &compilation.contract().types()[envelope as usize] else {
        panic!("Envelope must be a record")
    };
    assert!(fields.iter().any(|field| field.ty == callback));
    assert!(fields.iter().any(|field| field.ty == directory));
}

#[test]
fn all_valid_method_modes_are_explicit() {
    let compilation = compile(
        r#"
        service : {
          update_method: () -> ();
          q: () -> () query;
          cq: () -> () composite_query;
          ow: () -> () oneway;
        };
        "#,
    );
    let modes: BTreeMap<_, _> = service_methods(compilation.contract())
        .iter()
        .map(|method| {
            let TypeNode::Func { mode, results, .. } =
                &compilation.contract().types()[method.function as usize]
            else {
                panic!("service method must be a function")
            };
            if method.name == "ow" {
                assert!(results.is_empty());
            }
            (method.name.as_str(), *mode)
        })
        .collect();
    assert_eq!(modes["update_method"], MethodMode::Update);
    assert_eq!(modes["q"], MethodMode::Query);
    assert_eq!(modes["cq"], MethodMode::CompositeQuery);
    assert_eq!(modes["ow"], MethodMode::Oneway);
}

#[test]
fn distinct_service_methods_with_a_candid_hash_collision_remain_valid() {
    let compilation = compile(
        r#"
        service : {
          mydihazu: () -> ();
          mmnuxsdg: () -> ();
        };
        "#,
    );
    let methods = service_methods(compilation.contract());
    assert_eq!(methods.len(), 2);
    assert_ne!(methods[0].name, methods[1].name);
    assert_eq!(methods[0].id, methods[1].id);
    assert_eq!(methods[0].id, 3_085_626_469);
    assert!(compilation.contract().validate().is_ok());
}

#[test]
fn service_classes_keep_init_argument_order_and_service_target() {
    let compilation = compile(
        r#"
        type Init = record { owner: principal; flags: vec bool };
        type Endpoint = service { ping: () -> (nat) query };
        service : (Init, nat64) -> Endpoint;
        "#,
    );
    let Actor::Class { class } = compilation
        .contract()
        .actor()
        .as_ref()
        .expect("class actor")
    else {
        panic!("expected class actor")
    };
    let TypeNode::Class { init, service } = &compilation.contract().types()[*class as usize] else {
        panic!("actor class ref must target class")
    };
    assert_eq!(init.len(), 2);
    assert_eq!(init[0], declaration(compilation.contract(), "Init"));
    assert!(matches!(
        compilation.contract().types()[*service as usize],
        TypeNode::Service { .. }
    ));
}

#[test]
fn no_actor_empty_actor_and_zero_arg_constructor_remain_distinct() {
    let no_actor = compile("type OnlyType = nat;");
    let empty_actor = compile("service : {};");
    let empty_constructor = compile("service : () -> {};");
    assert!(no_actor.contract().actor().is_none());
    assert!(matches!(
        empty_actor.contract().actor(),
        Some(Actor::Service { .. })
    ));
    assert!(matches!(
        empty_constructor.contract().actor(),
        Some(Actor::Class { .. })
    ));
    assert_ne!(
        no_actor.contract().contract_id(),
        empty_actor.contract().contract_id()
    );
    assert_ne!(
        empty_actor.contract().contract_id(),
        empty_constructor.contract().contract_id()
    );
}

#[test]
fn byte_escapes_that_break_utf8_are_reported_not_panicked() {
    // `candid_parser` stores an unescaped string literal by pushing raw bytes
    // through `String::as_mut_vec`, so `\B8` leaves `Token::Text` holding a
    // lone UTF-8 continuation byte. Rendering that token panics inside
    // `<str as Debug>::fmt`, and `Display for candid_parser::Error` renders
    // the token for every parse error — so this five-byte source used to abort
    // the caller instead of returning `CompileError`.
    let error = compile_did("\"\\B8\"").expect_err("a bare text literal is not a program");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "did_parse_error");
    assert_eq!(diagnostic.phase, candid_core::DiagnosticPhase::Parse);
    assert!(!diagnostic.message.is_empty());
    assert!(
        diagnostic.span.is_some(),
        "the offending range must survive even when the token cannot be rendered"
    );

    // The same escape in the positions the fuzzer explored: unterminated,
    // repeated, and embedded after a well-formed prefix.
    let poisoned = [
        "\"\\B8\"",
        "\"\\B8",
        "\"\\B8\\B8\\B8\"",
        "type A = nat; service : { f: (text) -> () }; \"\\B8\"",
        "\"\\FF\\FE\"",
        "\"\\80\"",
    ];
    for source in poisoned {
        let result = compile_did(source);
        assert!(
            result.is_err(),
            "{source:?} is not a valid program and must be rejected"
        );
        let error = result.unwrap_err();
        assert!(
            !error.diagnostics.is_empty(),
            "{source:?} must produce at least one diagnostic"
        );
    }

    // The same inputs through the resolver entry point. This shares the panic
    // site with `compile_did` above — both reach `candid_error` via
    // `parse_program` — so it adds no mutation coverage today; deleting the fix
    // is already caught by the loop above. It is here to pin the *entry point*:
    // a future change that gave the resolver route its own error rendering
    // would otherwise reintroduce the panic with every test still green.
    //
    // `compile_did_file` shares this path. Neither route reaches
    // `candid_file_error`'s `Parse` arm with these inputs, because loading
    // parses every unit before `check_file` runs.
    for source in poisoned {
        let resolver = MemoryResolver::new()
            .with_source("memory:/entry.did", source)
            .expect("memory sources are well-formed ids");
        let error = match compile_with_resolver(
            "memory:/entry.did",
            &resolver,
            CompileOptions::default(),
            &RuntimeContext::default(),
        ) {
            Ok(_) => panic!("{source:?} is not a valid program and must be rejected"),
            Err(error) => error,
        };
        let diagnostic = &error.diagnostics[0];
        let span = diagnostic
            .span
            .as_ref()
            .expect("the offending range must survive the resolver route too");
        assert_eq!(
            span.source_name.as_deref(),
            Some("memory:/entry.did"),
            "the span must name the source it came from"
        );
    }
}

#[test]
fn well_formed_sources_are_unaffected_by_the_parse_error_rendering() {
    // The fix changes only how a `Parse` error is described. Type-check errors
    // still render through the upstream `Display`, so their text must be
    // unchanged and still name the offending symbol.
    let type_error = compile_did("service : { broken: (Missing) -> () };").unwrap_err();
    let diagnostic = &type_error.diagnostics[0];
    assert_eq!(diagnostic.phase, candid_core::DiagnosticPhase::TypeCheck);
    assert!(diagnostic.message.contains("Missing") || diagnostic.message.contains("Unbound"));

    // A parse error still carries a usable span and the expected-token notes.
    let syntax = compile_did("service : { broken: (nat) -> ( };").unwrap_err();
    let diagnostic = &syntax.diagnostics[0];
    let span = diagnostic.span.as_ref().expect("parse errors carry a span");
    assert!(span.end_byte > span.start_byte);
    assert!(
        !diagnostic.notes.is_empty(),
        "the expected-token list must still reach the caller"
    );
}

#[test]
fn invalid_did_returns_actionable_structured_diagnostics() {
    let syntax = compile_did("service : { broken: (nat) -> ( };").unwrap_err();
    let diagnostic = &syntax.diagnostics[0];
    assert_eq!(diagnostic.phase, candid_core::DiagnosticPhase::Parse);
    assert_eq!(diagnostic.code, "did_parse_error");
    assert!(diagnostic.span.is_some());
    assert!(!diagnostic.message.is_empty());

    let type_error = compile_did("service : { broken: (Missing) -> () };").unwrap_err();
    let diagnostic = &type_error.diagnostics[0];
    assert_eq!(diagnostic.phase, candid_core::DiagnosticPhase::TypeCheck);
    assert_eq!(diagnostic.code, "did_type_check_error");
    assert!(diagnostic.message.contains("Missing") || diagnostic.message.contains("Unbound"));

    let oneway = compile_did("service : { broken: () -> (nat) oneway };").unwrap_err();
    assert_eq!(
        oneway.diagnostics[0].phase,
        candid_core::DiagnosticPhase::TypeCheck
    );
}

#[test]
fn file_compilation_uses_the_authoritative_import_resolver() {
    let fixture = format!(
        "{}/tests/fixtures/imports/root.did",
        env!("CARGO_MANIFEST_DIR")
    );
    let compilation = compile_did_file(fixture).unwrap();
    assert!(compilation
        .contract()
        .declarations()
        .iter()
        .any(|declaration| declaration.name == "Imported"));
    assert!(compilation
        .contract()
        .contract_id()
        .starts_with("candid-core:contract:v1:sha256:"));
    let source_info = compilation.source_info().unwrap();
    assert_eq!(source_info.sources().len(), 2);
    assert!(source_info
        .declarations()
        .iter()
        .any(|declaration| declaration.name == "Imported"
            && declaration.source.ends_with("types.did")));
    assert!(source_info.field_labels().iter().any(|field| matches!(
        &field.origin,
        SourceOrigin::Declaration { source, name }
            if source.ends_with("types.did") && name == "Imported"
    )));
    assert!(compile_did("import \"types.did\"; service : {};").is_err());

    let invalid = format!(
        "{}/tests/fixtures/imports/invalid.did",
        env!("CARGO_MANIFEST_DIR")
    );
    let error = compile_did_file(invalid).unwrap_err();
    assert_eq!(
        error.diagnostics[0].phase,
        candid_core::DiagnosticPhase::TypeCheck
    );

    let invalid_syntax = format!(
        "{}/tests/fixtures/imports/invalid_syntax.did",
        env!("CARGO_MANIFEST_DIR")
    );
    let error = compile_did_file(invalid_syntax).unwrap_err();
    assert_eq!(
        error.diagnostics[0].phase,
        candid_core::DiagnosticPhase::Parse
    );
    assert!(error.diagnostics[0]
        .span
        .as_ref()
        .and_then(|span| span.source_name.as_deref())
        .is_some_and(|name| name.ends_with("invalid_syntax.did")));

    let imported_service = format!(
        "{}/tests/fixtures/imports/root_service.did",
        env!("CARGO_MANIFEST_DIR")
    );
    let imported_service = compile_did_file(imported_service).unwrap();
    assert_eq!(service_methods(imported_service.contract()).len(), 2);
    let imported_source_info = imported_service.source_info().unwrap();
    assert!(imported_source_info
        .methods()
        .iter()
        .any(|method| method.name == "imported_method"
            && matches!(
                &method.origin,
                SourceOrigin::Actor { source } if source.ends_with("service.did")
            )));
    assert!(imported_source_info.actors().iter().any(|actor| {
        actor.source.ends_with("service.did")
            && actor.docs == vec!["Imported service documentation.".to_string()]
    }));
}

#[test]
fn contract_json_is_strict_validated_and_graph_invariants_are_enforced() {
    let compilation = compile(
        r#"
        type Item = record { value: nat };
        service : { put: (Item) -> () };
        "#,
    );
    let json = compilation.contract().to_json_pretty().unwrap();
    let round_trip = Contract::from_json(&json).unwrap();
    assert_eq!(round_trip, compilation.contract().clone());
    assert!(Contract::from_json("{not JSON").is_err());

    let mut with_ui_metadata: serde_json::Value = serde_json::from_str(&json).unwrap();
    with_ui_metadata["widget"] = serde_json::json!("date-picker");
    assert!(Contract::from_json(&serde_json::to_string(&with_ui_metadata).unwrap()).is_err());

    let mut unsupported_version: serde_json::Value = serde_json::from_str(&json).unwrap();
    unsupported_version["format_version"] = serde_json::json!(999);
    assert!(Contract::from_json(&serde_json::to_string(&unsupported_version).unwrap()).is_err());

    let mut invented_type_kind: serde_json::Value = serde_json::from_str(&json).unwrap();
    invented_type_kind["types"][0]["kind"] = serde_json::json!("tuple");
    assert!(Contract::from_json(&serde_json::to_string(&invented_type_kind).unwrap()).is_err());

    let mut malformed_graph = RawContract::from(compilation.contract());
    let item = raw_declaration(&malformed_graph, "Item") as usize;
    let TypeNode::Record { fields } = &mut malformed_graph.types[item] else {
        panic!("Item must be a record")
    };
    fields[0].ty = u32::MAX;
    let graph_error = Contract::try_from_raw(malformed_graph).unwrap_err();
    assert!(graph_error
        .violations
        .iter()
        .any(|violation| violation.code == "dangling_type_ref"));

    let mut wrong_identity = RawContract::from(compilation.contract());
    wrong_identity.identities.contract =
        "candid-core:contract:v1:sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string();
    assert!(Contract::try_from_raw(wrong_identity.clone()).is_err());
    assert!(Contract::from_json(&serde_json::to_string(&wrong_identity).unwrap()).is_err());
}

#[test]
fn graph_validator_rejects_each_constrained_edge_kind_and_duplicate_id() {
    let compilation = compile(
        r#"
        type Item = record { value: nat };
        service : { put: (Item) -> () };
        "#,
    );

    let mut duplicate_field = RawContract::from(compilation.contract());
    let item = raw_declaration(&duplicate_field, "Item") as usize;
    let TypeNode::Record { fields } = &mut duplicate_field.types[item] else {
        panic!("Item must be a record")
    };
    fields.push(fields[0].clone());
    assert!(Contract::try_from_raw(duplicate_field)
        .unwrap_err()
        .violations
        .iter()
        .any(|violation| violation.code == "duplicate_field_id"));

    let mut bad_method_target = RawContract::from(compilation.contract());
    let Actor::Service { service } = bad_method_target.actor.as_ref().unwrap() else {
        panic!("service actor")
    };
    let TypeNode::Service { methods } = &mut bad_method_target.types[*service as usize] else {
        panic!("service node")
    };
    methods[0].function = *service;
    assert!(Contract::try_from_raw(bad_method_target)
        .unwrap_err()
        .violations
        .iter()
        .any(|violation| violation.code == "service_method_not_function"));

    let constructor = compile("service : (nat) -> {};");
    let mut bad_class = RawContract::from(constructor.contract());
    let Actor::Class { class } = bad_class.actor.as_ref().unwrap() else {
        panic!("class actor")
    };
    let TypeNode::Class { init, service } = &mut bad_class.types[*class as usize] else {
        panic!("class node")
    };
    *service = init[0];
    assert!(Contract::try_from_raw(bad_class)
        .unwrap_err()
        .violations
        .iter()
        .any(|violation| violation.code == "class_service_not_service"));

    let rootless_compilation = compile("type Item = nat;");
    let mut rootless = RawContract::from(rootless_compilation.contract());
    rootless.declarations.clear();
    assert!(Contract::try_from_raw(rootless)
        .unwrap_err()
        .violations
        .iter()
        .any(|violation| violation.code == "rootless_type_arena"));

    let class_as_argument = RawContract::new(
        vec![
            TypeNode::Class {
                init: vec![],
                service: 3,
            },
            TypeNode::Func {
                args: vec![0],
                results: vec![],
                mode: MethodMode::Update,
            },
            TypeNode::Service {
                methods: vec![ServiceMethod {
                    name: "call".to_string(),
                    id: candid_parser::candid::idl_hash("call"),
                    function: 1,
                }],
            },
            TypeNode::Service { methods: vec![] },
        ],
        vec![],
        Some(Actor::Service { service: 2 }),
    );
    assert!(Contract::build_raw(class_as_argument, &Limits::default())
        .unwrap_err()
        .violations
        .iter()
        .any(|violation| violation.code == "class_not_first_class_type"));
}

#[test]
fn source_sidecar_is_optional_and_never_changes_contract_identity() {
    let source = r#"
        // The comment is source provenance only.
        type Item = record { value: nat };
        service : { put: (Item) -> () };
    "#;
    let with_source = compile(source);
    let without_source = compile_did_with_options(
        source,
        CompileOptions {
            include_source_info: false,
        },
    )
    .unwrap();
    assert!(with_source.source_info().is_some());
    assert!(without_source.source_info().is_none());
    assert_eq!(with_source.contract(), without_source.contract());
}

#[test]
fn interface_ids_are_deterministic_and_ignore_provenance_but_track_wire_semantics() {
    let left = compile(
        r#"
        type Payload = record { foo: nat; 1: text };
        service : {
          z: (Payload) -> () query;
          a: (Payload) -> () query;
        };
        "#,
    );
    let right = compile(
        r#"
        // Formatting, field order, service order, and source alias differ.
        type RenamedPayload = record { 1: text; foo: nat };
        service : {
          a: (RenamedPayload) -> () query;
          z: (RenamedPayload) -> () query;
        };
        "#,
    );
    assert_eq!(
        left.contract().interface_id(),
        right.contract().interface_id()
    );
    assert_eq!(
        left.contract().to_json_pretty().unwrap(),
        left.contract().to_json_pretty().unwrap()
    );
    assert_ne!(
        left.contract().interface_id(),
        compile(
            r#"
            type Payload = record { foo: nat; 2: text };
            service : { a: (Payload) -> () query; z: (Payload) -> () query };
            "#
        )
        .contract()
        .interface_id()
    );
    assert_ne!(
        left.contract().interface_id(),
        compile(
            r#"
            type Payload = record { foo: nat; 1: text };
            service : { a: (Payload) -> () ; z: (Payload) -> () query };
            "#
        )
        .contract()
        .interface_id()
    );
}

#[test]
fn contract_id_is_invariant_under_type_ref_reindexing_and_duplicate_semantic_nodes() {
    let first_raw = RawContract::new(
        vec![
            TypeNode::Record {
                fields: vec![Field { id: 0, ty: 2 }, Field { id: 1, ty: 2 }],
            },
            TypeNode::Record {
                fields: vec![Field { id: 0, ty: 3 }, Field { id: 1, ty: 4 }],
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
        ],
        vec![
            Declaration {
                name: "A".to_string(),
                ty: 0,
            },
            Declaration {
                name: "B".to_string(),
                ty: 1,
            },
        ],
        None,
    );
    let reindexed_raw = RawContract::new(
        vec![
            TypeNode::Record {
                fields: vec![Field { id: 0, ty: 1 }, Field { id: 1, ty: 2 }],
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
            TypeNode::Record {
                fields: vec![Field { id: 0, ty: 4 }, Field { id: 1, ty: 4 }],
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
        ],
        vec![
            Declaration {
                name: "A".to_string(),
                ty: 3,
            },
            Declaration {
                name: "B".to_string(),
                ty: 0,
            },
        ],
        None,
    );
    let first = Contract::build_raw(first_raw, &Limits::default()).unwrap();
    let reindexed = Contract::build_raw(reindexed_raw.clone(), &Limits::default()).unwrap();
    let mut reindexed_input = reindexed_raw;
    reindexed_input.identities = first.identities().clone();
    assert!(Contract::try_from_raw(reindexed_input.clone()).is_ok());
    assert_eq!(
        Contract::from_json(&serde_json::to_string(&reindexed_input).unwrap()).unwrap(),
        first
    );
    assert_eq!(first.contract_id(), reindexed.contract_id());
    assert_eq!(first.types(), reindexed.types());
}
