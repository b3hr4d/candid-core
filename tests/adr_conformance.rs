use candid_core::{
    compile_did, compile_did_file, compile_did_with_context, compile_with_resolver,
    validate_host_value, Actor, Compilation, CompileOptions, Contract, ContractEnvelope,
    ContractMethodRef, ContractTypeRef, Declaration, Field, HostValue, Limits, MemoryResolver,
    PrimitiveType, RawContract, ResolveError, ResolvedSource, RuntimeContext, SourceId,
    SourceResolver, TypeNode, CANONICALIZATION_PROFILE, CONTRACT_FORMAT, FORMAT_VERSION,
    SEMANTICS_PROFILE,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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

#[test]
fn identities_make_distinct_equality_claims() {
    let actor_only = compile("service : { ping: () -> (nat) query };");
    let with_unused_declaration = compile(
        r#"
        type InternalOnly = record { note: text };
        service : { ping: () -> (nat) query };
        "#,
    );

    assert_eq!(
        actor_only.contract().interface_id(),
        with_unused_declaration.contract().interface_id()
    );
    assert_ne!(
        actor_only.contract().contract_id(),
        with_unused_declaration.contract().contract_id()
    );

    let first_source = compile("// first\nservice : { ping: () -> () };");
    let second_source = compile("// second\nservice : { ping: () -> () };");
    assert_eq!(
        first_source.contract().contract_id(),
        second_source.contract().contract_id()
    );
    assert_ne!(
        first_source.source_info().unwrap().source_bundle_id(),
        second_source.source_info().unwrap().source_bundle_id()
    );
}

#[test]
fn canonical_envelope_profiles_are_explicit_and_fail_closed() {
    let contract = compile("service : {};").contract().clone();
    assert_eq!(contract.format(), CONTRACT_FORMAT);
    assert_eq!(contract.format_version(), FORMAT_VERSION);
    assert_eq!(contract.semantics_profile(), SEMANTICS_PROFILE);
    assert_eq!(
        contract.canonicalization_profile(),
        CANONICALIZATION_PROFILE
    );
    assert!(contract
        .contract_id()
        .starts_with("candid-core:contract:v1:sha256:"));
    assert!(contract
        .interface_id()
        .unwrap()
        .starts_with("candid-core:interface:v1:sha256:"));

    let mut raw = RawContract::from(&contract);
    raw.semantics_profile = "future-candid".to_string();
    let error = Contract::try_from_raw(raw).unwrap_err();
    assert!(error
        .violations
        .iter()
        .any(|violation| violation.code == "unsupported_semantics_profile"));
}

#[test]
fn compilation_deserialization_rejects_a_mismatched_sidecar() {
    let compilation = compile("type Item = record { value: nat }; service : {};");
    let mut json = serde_json::to_value(&compilation).unwrap();
    json["source_info"]["contract_id"] = serde_json::json!(
        "candid-core:contract:v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(serde_json::from_value::<Compilation>(json).is_err());
}

#[test]
fn source_ids_deserialize_through_the_canonical_parser() {
    for invalid in [
        r#"""#,
        r#""not-a-logical-uri/../..""#,
        r#""UPPER:/entry.did""#,
        r#""memory:/../entry.did""#,
    ] {
        assert!(
            serde_json::from_str::<SourceId>(invalid).is_err(),
            "{invalid}"
        );
    }

    let id: SourceId = serde_json::from_str(r#""registry:/catalog/./v1/types.did""#).unwrap();
    assert_eq!(id.as_str(), "registry:/catalog/v1/types.did");
    assert_eq!(id.scheme(), "registry");
    assert_eq!(id.path(), "catalog/v1/types.did");
    assert_eq!(
        serde_json::to_string(&id).unwrap(),
        r#""registry:/catalog/v1/types.did""#
    );
}

#[test]
fn source_id_construction_routes_share_normalization() {
    let parsed = SourceId::parse("registry:/catalog/./v1/types.did").unwrap();
    let from_str = "registry:/catalog/./v1/types.did"
        .parse::<SourceId>()
        .unwrap();
    let tried = SourceId::try_from("registry:/catalog/./v1/types.did").unwrap();
    assert_eq!(parsed, from_str);
    assert_eq!(parsed, tried);
}

#[test]
fn memory_resolver_compiles_one_immutable_logical_source_bundle() {
    let mut resolver = MemoryResolver::new();
    resolver
        .insert(
            "memory:/api/root.did",
            r#"import "types.did"; service : { read: () -> (Item) query };"#,
        )
        .unwrap();
    resolver
        .insert(
            "memory:/api/types.did",
            "type Item = record { id: nat64; label: text };",
        )
        .unwrap();

    let compilation = compile_with_resolver(
        "memory:/api/root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap();
    let source_info = compilation.source_info().unwrap();
    assert_eq!(source_info.sources().len(), 2);
    assert_eq!(source_info.imports().len(), 1);
    assert_eq!(source_info.imports()[0].from, "memory:/api/root.did");
    assert_eq!(source_info.imports()[0].to, "memory:/api/types.did");
    assert!(source_info
        .source_bundle_id()
        .starts_with("candid-core:source-bundle:v1:sha256:"));
}

struct AliasResolver {
    sources: BTreeMap<SourceId, String>,
}

impl AliasResolver {
    fn new() -> Self {
        let mut sources = BTreeMap::new();
        sources.insert(
            SourceId::parse("registry:/entry.did").unwrap(),
            r#"import "types.did"; service : { read: () -> (Item) query };"#.to_string(),
        );
        sources.insert(
            SourceId::parse("registry:/catalog/v1/types.did").unwrap(),
            "type Item = nat;".to_string(),
        );
        Self { sources }
    }
}

impl SourceResolver for AliasResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        match (from.map(SourceId::as_str), import) {
            (None, "entry") => SourceId::parse("registry:/entry.did"),
            (Some("registry:/entry.did"), "types.did") => {
                SourceId::parse("registry:/catalog/v1/types.did")
            }
            _ => Err(ResolveError {
                code: "did_source_not_found".to_string(),
                message: format!("no alias mapping for {import:?}"),
                resource_limit: None,
            }),
        }
    }

    fn load(&self, id: &SourceId, _limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        let source = self.sources.get(id).cloned().ok_or_else(|| ResolveError {
            code: "did_source_not_found".to_string(),
            message: format!("missing source {:?}", id.as_str()),
            resource_limit: None,
        })?;
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(source.as_bytes())));
        Ok(ResolvedSource {
            id: id.clone(),
            source,
            digest,
        })
    }
}

#[test]
fn materialization_honors_custom_resolver_aliases() {
    let compilation = compile_with_resolver(
        "entry",
        &AliasResolver::new(),
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap();
    let source_info = compilation.source_info().unwrap();
    assert_eq!(source_info.imports().len(), 1);
    assert_eq!(source_info.imports()[0].import, "types.did");
    assert_eq!(
        source_info.imports()[0].to,
        "registry:/catalog/v1/types.did"
    );
}

#[derive(Clone)]
struct CountingResolver {
    inner: MemoryResolver,
    loads: Arc<AtomicUsize>,
}

impl SourceResolver for CountingResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        self.inner.identify(from, import)
    }

    fn load(&self, id: &SourceId, limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load(id, limits)
    }
}

#[test]
fn diamond_imports_are_snapshotted_and_loaded_once() {
    let mut inner = MemoryResolver::new();
    inner
        .insert(
            "root.did",
            r#"import "a.did"; import "b.did"; service : { get: () -> (Common) };"#,
        )
        .unwrap();
    inner
        .insert("a.did", r#"import "common.did"; type A = Common;"#)
        .unwrap();
    inner
        .insert("b.did", r#"import "common.did"; type B = Common;"#)
        .unwrap();
    inner.insert("common.did", "type Common = nat;").unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    let resolver = CountingResolver {
        inner,
        loads: loads.clone(),
    };
    let compilation = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap();
    assert_eq!(compilation.source_info().unwrap().sources().len(), 4);
    assert_eq!(loads.load(Ordering::SeqCst), 4);
}

#[test]
fn contract_validation_caps_retained_diagnostics() {
    let raw = RawContract::new(
        vec![
            TypeNode::Record {
                fields: (0..16).map(|_| Field { id: 0, ty: 1 }).collect(),
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
        ],
        vec![Declaration {
            name: "Repeated".to_string(),
            ty: 0,
        }],
        None,
    );
    let limits = Limits {
        max_diagnostics: 3,
        ..Limits::default()
    };
    let error = Contract::build_raw(raw, &limits).unwrap_err();
    assert_eq!(error.violations.len(), limits.max_diagnostics);
    let cap = error.violations.last().unwrap();
    assert_eq!(cap.code, "resource_limit_exceeded");
    let info = cap.resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "diagnostics");
    assert_eq!(info.limit, limits.max_diagnostics);
    assert!(info.observed > limits.max_diagnostics);
}

#[test]
fn resolver_rejects_authority_escape_and_import_cycles() {
    let mut escape = MemoryResolver::new();
    escape
        .insert("root.did", "import \"../secret.did\"; service : {};")
        .unwrap();
    let error = compile_with_resolver(
        "root.did",
        &escape,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap_err();
    assert_eq!(error.diagnostics[0].code, "did_import_outside_workspace");

    let mut cycle = MemoryResolver::new();
    cycle
        .insert("a.did", "import \"b.did\"; service : {};")
        .unwrap();
    cycle.insert("b.did", "import \"a.did\";").unwrap();
    let error = compile_with_resolver(
        "a.did",
        &cycle,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap_err();
    assert_eq!(error.diagnostics[0].code, "did_import_cycle");
}

#[test]
fn operational_limits_fail_with_machine_stable_diagnostics() {
    let context = RuntimeContext {
        limits: Limits {
            max_source_bytes: 8,
            ..Limits::default()
        },
    };
    let error =
        compile_did_with_context("service : {};", CompileOptions::default(), &context).unwrap_err();
    assert_eq!(error.diagnostics[0].code, "resource_limit_exceeded");
    assert_eq!(
        error.diagnostics[0]
            .resource_limit
            .as_ref()
            .unwrap()
            .resource,
        "source_bytes"
    );

    let json = compile("service : {};")
        .contract()
        .to_json_pretty()
        .unwrap();
    let limits = Limits {
        max_input_bytes: json.len() - 1,
        ..Limits::default()
    };
    let error = Contract::from_json_with_limits(&json, &limits).unwrap_err();
    assert!(error.to_string().contains("validation failed"));
}

#[test]
fn elapsed_deadlines_abort_work_without_partial_artifacts() {
    let context = RuntimeContext {
        limits: Limits {
            deadline_unix_ms: Some(1),
            ..Limits::default()
        },
    };
    let error =
        compile_did_with_context("service : {};", CompileOptions::default(), &context).unwrap_err();
    assert_eq!(error.diagnostics[0].code, "operation_deadline_exceeded");
}

#[test]
fn iterative_canonical_traversal_handles_deep_graphs_and_honors_work_limits() {
    let depth = 256u32;
    let mut types = (0..depth)
        .map(|index| TypeNode::Opt { inner: index + 1 })
        .collect::<Vec<_>>();
    types.push(TypeNode::Primitive {
        primitive: PrimitiveType::Nat,
    });
    let raw = RawContract::new(
        types,
        vec![Declaration {
            name: "Deep".to_string(),
            ty: 0,
        }],
        None,
    );
    let contract = Contract::build_raw(raw.clone(), &Limits::default()).unwrap();
    assert_eq!(contract.types().len(), depth as usize + 1);

    let limits = Limits {
        max_canonicalization_work: 10,
        ..Limits::default()
    };
    let error = Contract::build_raw(raw, &limits).unwrap_err();
    assert!(error
        .violations
        .iter()
        .any(|violation| violation.code == "resource_limit_exceeded"));
}

#[test]
fn tagged_host_values_preserve_bigints_float_bits_and_wire_field_ids() {
    let compilation = compile(
        r#"
        type Payload = record { big: nat; ratio: float64; owner: principal };
        service : { submit: (Payload) -> () };
        "#,
    );
    let contract = compilation.contract();
    let selector = contract
        .bind_type(declaration(contract, "Payload"))
        .unwrap();
    let value = HostValue::from_json_with_limits(
        &serde_json::json!({
            "kind": "record",
            "fields": [
                { "id": candid_parser::candid::idl_hash("big"), "value": { "kind": "nat", "value": "340282366920938463463374607431768211456" } },
                { "id": candid_parser::candid::idl_hash("ratio"), "value": { "kind": "float64", "bits": "7ff8000000000001" } },
                { "id": candid_parser::candid::idl_hash("owner"), "value": { "kind": "principal", "value": "aaaaa-aa" } },
            ],
        })
        .to_string(),
        &Limits::default(),
    )
    .unwrap();
    validate_host_value(contract, &selector, &value, &Limits::default()).unwrap();

    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        HostValue::from_json_with_limits(&json, &Limits::default()).unwrap(),
        value
    );
}

#[test]
fn host_values_reject_coercions_and_unbound_contract_references() {
    let compilation = compile("type Amount = nat; service : {};");
    let contract = compilation.contract();
    let mut selector = contract.bind_type(declaration(contract, "Amount")).unwrap();
    assert!(HostValue::nat("001").is_err());

    selector.contract_id =
        "candid-core:contract:v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string();
    assert!(validate_host_value(
        contract,
        &selector,
        &HostValue::nat("1").unwrap(),
        &Limits::default()
    )
    .is_err());
}

#[test]
fn extensions_are_namespaced_and_cannot_mutate_the_core() {
    let contract = compile("service : {};").contract().clone();
    let mut envelope = ContractEnvelope::new(contract.clone());
    envelope
        .insert_extension(
            "com.example.form/v1",
            serde_json::json!({ "widget": "button" }),
            &Limits::default(),
        )
        .unwrap();
    envelope.validate(&Limits::default()).unwrap();
    assert_eq!(envelope.contract().contract_id(), contract.contract_id());

    assert!(envelope
        .insert_extension("unversioned", serde_json::json!({}), &Limits::default())
        .is_err());
    assert_eq!(envelope.extensions().len(), 1);

    let mut raw = serde_json::to_value(&envelope).unwrap();
    raw["extensions"]["unversioned"] = serde_json::json!({});
    assert!(serde_json::from_value::<ContractEnvelope>(raw).is_err());
}

#[test]
fn actor_methods_are_persisted_by_contract_identity_and_name() {
    let contract = compile("service : { ping: () -> () query };")
        .contract()
        .clone();
    let selector = contract.bind_method("ping").unwrap();
    assert_eq!(selector.contract_id, contract.contract_id());
    assert_eq!(selector.method_name, "ping");
    assert!(contract.bind_method("missing").is_err());
    assert!(matches!(contract.actor(), Some(Actor::Service { .. })));
}

#[test]
fn persisted_selectors_use_protocol_field_names_and_fail_closed() {
    let contract = compile("type Amount = nat; service : { ping: () -> () query };")
        .contract()
        .clone();
    let type_selector = contract
        .bind_type(declaration(&contract, "Amount"))
        .unwrap();
    let method_selector = contract.bind_method("ping").unwrap();

    let type_json = serde_json::json!({
        "contract_id": contract.contract_id(),
        "type_ref": type_selector.type_ref,
    });
    let method_json = serde_json::json!({
        "contract_id": contract.contract_id(),
        "method_name": "ping",
    });
    assert_eq!(serde_json::to_value(&type_selector).unwrap(), type_json);
    assert_eq!(serde_json::to_value(&method_selector).unwrap(), method_json);
    assert_eq!(
        serde_json::from_value::<ContractTypeRef>(type_json).unwrap(),
        type_selector
    );
    assert_eq!(
        serde_json::from_value::<ContractMethodRef>(method_json).unwrap(),
        method_selector
    );

    assert!(
        serde_json::from_value::<ContractTypeRef>(serde_json::json!({
            "contract_id": contract.contract_id(),
            "type": type_selector.type_ref,
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ContractMethodRef>(serde_json::json!({
            "contract_id": contract.contract_id(),
            "method": "ping",
        }))
        .is_err()
    );
}

#[test]
fn contract_id_changes_when_declaration_names_change() {
    let first = compile("type First = record { value: nat }; service : {};");
    let second = compile("type Second = record { value: nat }; service : {};");
    assert_eq!(
        first.contract().interface_id(),
        second.contract().interface_id()
    );
    assert_ne!(
        first.contract().contract_id(),
        second.contract().contract_id()
    );
}

#[test]
fn jcs_identity_is_independent_of_input_object_key_order() {
    let contract = compile("service : { ping: () -> () };").contract().clone();
    let mut raw = serde_json::to_value(&contract).unwrap();
    let object = raw.as_object_mut().unwrap();
    let actor = object.remove("actor").unwrap();
    object.insert("actor".to_string(), actor);
    let decoded: Contract = serde_json::from_value(raw).unwrap();
    assert_eq!(decoded.contract_id(), contract.contract_id());
}

#[test]
fn raw_graph_builder_calculates_identities_for_producers() {
    let raw = RawContract::new(
        vec![
            TypeNode::Record {
                fields: vec![Field { id: 0, ty: 1 }],
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Text,
            },
        ],
        vec![Declaration {
            name: "LibraryValue".to_string(),
            ty: 0,
        }],
        None,
    );
    let contract = Contract::build_raw(raw, &Limits::default()).unwrap();
    assert!(contract.interface_id().is_none());
    assert!(contract.validate().is_ok());
}

#[test]
fn canonical_contracts_match_checked_in_cross_language_fixtures() {
    for name in ["actorless", "empty_actor", "class", "basic", "recursive"] {
        let did = format!(
            "{}/tests/fixtures/conformance/{name}.did",
            env!("CARGO_MANIFEST_DIR")
        );
        let expected = format!(
            "{}/tests/fixtures/conformance/{name}.contract.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let contract = compile_did_file(did).unwrap().into_parts().0;
        let expected = std::fs::read_to_string(expected).unwrap();
        let expected: Contract = Contract::from_json(&expected).unwrap();
        assert_eq!(contract, expected, "fixture {name} drifted");
    }
}

fn exact_manifest_dependency_version(dependency: &str) -> String {
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let prefix = format!("{dependency} = ");
    manifest
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_prefix('='))
                .and_then(|value| value.split_once('"').map(|(version, _)| version))
        })
        .unwrap_or_else(|| panic!("{dependency} must be exact-pinned in Cargo.toml"))
        .to_string()
}

fn lockfile_versions(package: &str) -> Vec<String> {
    let lockfile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));
    let mut versions = Vec::new();
    let mut current_package_matches = false;

    for line in lockfile.lines().map(str::trim) {
        if line == "[[package]]" {
            current_package_matches = false;
        } else if line == format!("name = \"{package}\"") {
            current_package_matches = true;
        } else if current_package_matches {
            if let Some(version) = line
                .strip_prefix("version = ")
                .and_then(|value| value.strip_prefix('"'))
                .and_then(|value| value.strip_suffix('"'))
            {
                versions.push(version.to_string());
                current_package_matches = false;
            }
        }
    }

    versions
}

#[test]
fn producer_reports_exact_selected_candid_engine_versions() {
    let contract = compile("service : {};").contract().clone();
    let producer = contract.producer();

    assert_eq!(producer.name, env!("CARGO_PKG_NAME"));
    assert_eq!(producer.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        producer.candid_version,
        exact_manifest_dependency_version("candid")
    );
    assert_eq!(
        producer.candid_parser_version,
        exact_manifest_dependency_version("candid_parser")
    );
    assert_eq!(contract.semantics_profile(), SEMANTICS_PROFILE);
}

#[test]
fn identity_relevant_candid_dependencies_are_exact_and_not_duplicated() {
    let candid_version = exact_manifest_dependency_version("candid");
    let candid_parser_version = exact_manifest_dependency_version("candid_parser");

    assert_eq!(lockfile_versions("candid"), vec![candid_version]);
    assert_eq!(
        lockfile_versions("candid_parser"),
        vec![candid_parser_version]
    );
}
