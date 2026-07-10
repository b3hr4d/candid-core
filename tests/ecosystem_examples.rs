use candid_contract_runtime::{compile_did, Actor, Contract, SourceLabel, TypeNode, TypeRef};
use std::collections::BTreeSet;

fn compile(source: &str) -> candid_contract_runtime::Compilation {
    compile_did(source).unwrap_or_else(|error| panic!("compilation failed: {error:#?}"))
}

fn children(node: &TypeNode) -> Vec<TypeRef> {
    match node {
        TypeNode::Primitive { .. } => Vec::new(),
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => vec![*inner],
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            fields.iter().map(|field| field.ty).collect()
        }
        TypeNode::Func { args, results, .. } => args.iter().chain(results).copied().collect(),
        TypeNode::Service { methods } => methods.iter().map(|method| method.function).collect(),
        TypeNode::Class { init, service } => init
            .iter()
            .copied()
            .chain(std::iter::once(*service))
            .collect(),
    }
}

#[test]
fn canonical_contract_is_an_idempotent_cache_value() {
    let compilation = compile(
        r#"
        type Payload = record { owner: principal; amount: nat };
        service : { transfer: (Payload) -> () };
        "#,
    );
    let contract = compilation.contract();

    assert_eq!(contract.canonicalize().unwrap(), contract.clone());
    let json = contract.to_json_pretty().unwrap();
    assert_eq!(Contract::from_json(&json).unwrap(), contract.clone());
    assert_eq!(
        Contract::from_json(&json)
            .unwrap()
            .to_json_pretty()
            .unwrap(),
        json
    );
}

#[test]
fn equivalent_sources_share_semantic_cache_identity_but_not_provenance() {
    let first = compile(
        r#"
        type Payload = record { owner: principal; amount: nat };
        service : { z: (Payload) -> () query; a: (Payload) -> () query };
        "#,
    );
    let second = compile(
        r#"
        // A source-only explanation.
        type Transfer = record { amount: nat; owner: principal };
        service : { a: (Transfer) -> () query; z: (Transfer) -> () query };
        "#,
    );

    assert_eq!(
        first.contract().fingerprint(),
        second.contract().fingerprint()
    );
    assert_ne!(first.source_info(), second.source_info());
}

#[test]
fn recursive_contracts_are_finite_and_safe_for_iterative_tooling() {
    let compilation = compile(
        r#"
        type List = opt record { head: nat; tail: List };
        service : { get: () -> (List) query };
        "#,
    );
    let contract = compilation.contract();
    let root = match contract.actor().as_ref().expect("service actor") {
        Actor::Service { service } => *service,
        Actor::Class { .. } => panic!("expected service actor"),
    };

    let mut seen = BTreeSet::new();
    let mut work = vec![root];
    while let Some(reference) = work.pop() {
        if seen.insert(reference) {
            work.extend(children(&contract.types()[reference as usize]));
        }
    }

    assert_eq!(seen.len(), contract.types().len());
    assert!(
        seen.len() < 10,
        "recursive syntax should remain a finite graph"
    );
}

#[test]
fn source_labels_explain_wire_ids_without_changing_the_contract() {
    let compilation = compile(
        r#"
        type Named = record { item: nat };
        type Numeric = record { 1191203122: nat };
        service : { inspect: (Named) -> (Numeric) };
        "#,
    );
    let source_info = compilation.source_info().expect("source sidecar");
    let labels: BTreeSet<_> = source_info
        .field_labels()
        .iter()
        .map(|field| match &field.label {
            SourceLabel::Named { .. } => "named",
            SourceLabel::Numeric => "numeric",
            SourceLabel::Positional => "positional",
        })
        .collect();

    assert_eq!(labels, BTreeSet::from(["named", "numeric"]));
    assert!(!compilation
        .contract()
        .to_json_pretty()
        .unwrap()
        .contains("\"item\""));
}

#[test]
fn extension_metadata_is_separate_from_the_strict_semantic_core() {
    let compilation = compile("service : { ping: () -> () };");
    let contract = compilation.contract();
    let mut core: serde_json::Value =
        serde_json::from_str(&contract.to_json_pretty().unwrap()).unwrap();
    core["com.example.form/v1"] = serde_json::json!({ "widget": "button" });
    assert!(Contract::from_json(&serde_json::to_string(&core).unwrap()).is_err());

    let envelope = serde_json::json!({
        "contract": contract,
        "extensions": {
            "com.example.form/v1": { "widget": "button" }
        }
    });
    let decoded: Contract = serde_json::from_value(envelope["contract"].clone()).unwrap();
    assert!(decoded.validate().is_ok());
}

#[test]
fn diagnostics_are_directly_serializable_for_agent_tools() {
    let error = compile_did("service : { broken: (Missing) -> () };").unwrap_err();
    let value = serde_json::to_value(&error.diagnostics).unwrap();
    let diagnostic = &value[0];

    assert_eq!(diagnostic["code"], "did_type_check_error");
    assert_eq!(diagnostic["phase"], "type_check");
    assert_eq!(diagnostic["severity"], "error");
    assert!(diagnostic["message"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));
}
