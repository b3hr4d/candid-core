use candid_core::{
    compile_did, compile_did_file, compile_did_file_with_context, compile_did_with_context,
    compile_with_resolver, validate_host_value, Actor, CancellationToken, Compilation,
    CompileError, CompileOptions, Contract, ContractEnvelope, ContractMethodRef, ContractTypeRef,
    Declaration, Field, HostValue, Limits, MemoryResolver, PrimitiveType, RawContract,
    RawSourceInfo, ResolveError, ResolvedSource, RuntimeContext, SourceId, SourceInfo, SourceLabel,
    SourceOrigin, SourceResolver, TypeNode, CANONICALIZATION_PROFILE, CONTRACT_FORMAT,
    FORMAT_VERSION, SEMANTICS_PROFILE,
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
fn compilation_parsing_rejects_a_mismatched_sidecar() {
    let compilation = compile("type Item = record { value: nat }; service : {};");
    let mut json = serde_json::to_value(&compilation).unwrap();
    json["source_info"]["contract_id"] = serde_json::json!(
        "candid-core:contract:v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(Compilation::from_json_with_limits(&json.to_string(), &Limits::default()).is_err());
}

#[test]
fn compilation_atomically_remaps_raw_source_references() {
    let expected = compile("type A = nat; type B = text; service : {};");
    let mut raw_contract = RawContract::from(expected.contract());
    let mut raw_source_info: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(expected.source_info().unwrap()).unwrap())
            .unwrap();

    let a = raw_contract
        .declarations
        .iter()
        .find(|declaration| declaration.name == "A")
        .unwrap()
        .ty;
    let b = raw_contract
        .declarations
        .iter()
        .find(|declaration| declaration.name == "B")
        .unwrap()
        .ty;
    raw_contract.types.swap(a as usize, b as usize);
    for declaration in &mut raw_contract.declarations {
        declaration.ty = match declaration.ty {
            ty if ty == a => b,
            ty if ty == b => a,
            ty => ty,
        };
    }
    for declaration in &mut raw_source_info.declarations {
        declaration.ty = match declaration.ty {
            ty if ty == a => b,
            ty if ty == b => a,
            ty => ty,
        };
    }

    let actual =
        Compilation::try_from_raw(raw_contract, Some(raw_source_info), &Limits::default()).unwrap();
    assert_eq!(actual, expected);
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
fn logical_source_path_grammar_is_platform_independent() {
    for (input, expected) in [
        ("entry.did", "memory:/entry.did"),
        ("memory:/dir/./entry.did", "memory:/dir/entry.did"),
        ("memory:/dir/nested/../entry.did", "memory:/dir/entry.did"),
    ] {
        assert_eq!(
            SourceId::parse(input).unwrap().as_str(),
            expected,
            "{input}"
        );
    }

    for invalid in [
        "/entry.did",
        "memory://entry.did",
        "memory:/dir//entry.did",
        "memory:/entry.did/",
        "memory:/dir\\entry.did",
        "C:/entry.did",
        "c:/entry.did",
        "C:\\entry.did",
        "memory:/C:/entry.did",
        "memory:/dir:entry.did",
        "memory:/entry\0.did",
        "memory:/entry\n.did",
        "memory:/../entry.did",
        "memory:/dir/../..",
        "1memory:/entry.did",
        "-memory:/entry.did",
    ] {
        assert!(SourceId::parse(invalid).is_err(), "accepted {invalid:?}");
    }

    let resolver = MemoryResolver::new();
    let parent = SourceId::parse("memory:/dir/parent.did").unwrap();
    assert_eq!(
        resolver
            .identify(Some(&parent), "nested/../child.did")
            .unwrap()
            .as_str(),
        "memory:/dir/child.did"
    );
    for invalid in [
        "",
        "/child.did",
        "dir//child.did",
        "dir\\child.did",
        "C:/child.did",
    ] {
        assert!(
            resolver.identify(Some(&parent), invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn source_info_rejects_invalid_and_noncanonical_logical_ids() {
    let compilation = compile("service : {};");
    let mut json = serde_json::to_value(compilation.source_info().unwrap()).unwrap();
    json["sources"][0]["name"] = serde_json::json!("UPPER:/entry.did");
    let error = SourceInfo::try_from_raw(
        serde_json::from_value::<RawSourceInfo>(json).unwrap(),
        compilation.contract(),
        &Limits::default(),
    )
    .unwrap_err();
    assert_eq!(error.violations[0].code, "invalid_source_id");
    assert_eq!(error.violations[0].path, "$.sources[0].name");

    let mut resolver = MemoryResolver::new();
    resolver
        .insert("root.did", r#"import "types.did"; service : {};"#)
        .unwrap();
    resolver.insert("types.did", "type Item = nat;").unwrap();
    let compilation = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap();
    let json = serde_json::to_value(compilation.source_info().unwrap()).unwrap();
    for (field, invalid) in [
        ("from", "memory:/root/../root.did"),
        ("to", "memory:/../types.did"),
    ] {
        let mut candidate = json.clone();
        candidate["imports"][0][field] = serde_json::json!(invalid);
        let error = SourceInfo::try_from_raw(
            serde_json::from_value::<RawSourceInfo>(candidate).unwrap(),
            compilation.contract(),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(error.violations[0].code, "invalid_source_id");
        assert_eq!(error.violations[0].path, format!("$.imports[0].{field}"));
    }
}

#[test]
fn source_info_raw_construction_is_fallible_and_contract_bound() {
    let compilation = compile("service : {};");
    let raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        SourceInfo::try_from_raw(raw.clone(), compilation.contract(), &Limits::default()).unwrap(),
        compilation.source_info().unwrap().clone()
    );

    let mut unsupported = raw.clone();
    unsupported.source_info_version = 999;
    let error = SourceInfo::try_from_raw(unsupported, compilation.contract(), &Limits::default())
        .unwrap_err();
    assert_eq!(error.violations[0].code, "unsupported_source_info_version");
    assert_eq!(error.violations[0].path, "$.source_info_version");

    let other = compile("service : { ping: () -> () };");
    let error = SourceInfo::try_from_raw(raw, other.contract(), &Limits::default()).unwrap_err();
    assert_eq!(error.violations[0].code, "source_contract_id_mismatch");
    assert_eq!(error.violations[0].path, "$.contract_id");
}

fn assert_provenance_mismatch(raw: RawSourceInfo, contract: &Contract, path: &str) {
    let error = SourceInfo::try_from_raw(raw, contract, &Limits::default()).unwrap_err();
    assert_eq!(error.violations[0].code, "source_info_provenance_mismatch");
    assert_eq!(error.violations[0].path, path);
}

#[test]
fn source_info_rederivation_rejects_every_derived_provenance_category() {
    let compilation = compile(
        r#"
        type Honest = record { honest: nat };
        type Other = record { other: text };
        service : {
            ping: (input: Honest) -> (output: nat);
            pong: (input: text) -> (output: nat);
        };
        "#,
    );
    let raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();

    let mut declaration = raw.clone();
    declaration
        .declarations
        .iter_mut()
        .find(|item| item.name == "Honest")
        .unwrap()
        .name = "Forged".to_string();
    assert_provenance_mismatch(
        declaration.clone(),
        compilation.contract(),
        "$.declarations",
    );
    let compilation_error = Compilation::try_from_raw(
        RawContract::from(compilation.contract()),
        Some(declaration),
        &Limits::default(),
    )
    .unwrap_err();
    assert_eq!(
        compilation_error.violations[0].code,
        "source_info_provenance_mismatch"
    );
    assert_eq!(compilation_error.violations[0].path, "$.declarations");

    let honest_field = raw
        .field_labels
        .iter()
        .position(|field| {
            matches!(
                &field.origin,
                SourceOrigin::Declaration { name, .. } if name == "Honest"
            )
        })
        .unwrap();
    let mut origin = raw.clone();
    let SourceOrigin::Declaration { name, .. } = &mut origin.field_labels[honest_field].origin
    else {
        unreachable!()
    };
    *name = "Other".to_string();
    assert_provenance_mismatch(origin, compilation.contract(), "$.field_labels");

    let mut label = raw.clone();
    let SourceLabel::Named { name } = &mut label.field_labels[honest_field].label else {
        unreachable!()
    };
    *name = "forged".to_string();
    assert_provenance_mismatch(label, compilation.contract(), "$.field_labels");

    let mut path = raw.clone();
    path.field_labels[honest_field].path = "type:Honest.forged".to_string();
    assert_provenance_mismatch(path, compilation.contract(), "$.field_labels");

    let mut docs = raw.clone();
    docs.field_labels[honest_field]
        .docs
        .push("forged documentation".to_string());
    assert_provenance_mismatch(docs, compilation.contract(), "$.field_labels");

    let mut method = raw.clone();
    method
        .methods
        .iter_mut()
        .find(|item| item.name == "ping")
        .unwrap()
        .name = "pong".to_string();
    assert_provenance_mismatch(method, compilation.contract(), "$.methods");

    let service = match compilation.contract().actor().unwrap() {
        Actor::Service { service } => *service,
        Actor::Class { .. } => unreachable!(),
    };
    let TypeNode::Service { methods } = &compilation.contract().types()[service as usize] else {
        unreachable!()
    };
    let ping_function = methods
        .iter()
        .find(|method| method.name == "ping")
        .unwrap()
        .function;
    let pong_function = methods
        .iter()
        .find(|method| method.name == "pong")
        .unwrap()
        .function;
    let mut argument = raw.clone();
    argument
        .function_arguments
        .iter_mut()
        .find(|item| item.function == ping_function)
        .unwrap()
        .function = pong_function;
    assert_provenance_mismatch(argument, compilation.contract(), "$.function_arguments");

    let mut argument_name = raw;
    argument_name.function_arguments[0].name = "forged".to_string();
    assert_provenance_mismatch(
        argument_name,
        compilation.contract(),
        "$.function_arguments",
    );
}

#[test]
fn source_info_rederivation_verifies_imported_actor_relationships() {
    let fixture = format!(
        "{}/tests/fixtures/imports/root_service.did",
        env!("CARGO_MANIFEST_DIR")
    );
    let compilation = compile_did_file(fixture).unwrap();
    let mut raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();
    assert_eq!(raw.actors.len(), 2);
    let first = raw.actors[0].source.clone();
    raw.actors[0].source = raw.actors[1].source.clone();
    raw.actors[1].source = first;
    assert_provenance_mismatch(raw, compilation.contract(), "$.actors");
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
    let raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(source_info).unwrap()).unwrap();
    assert_eq!(
        SourceInfo::try_from_raw(raw, compilation.contract(), &Limits::default()).unwrap(),
        source_info.clone()
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
    let raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        SourceInfo::try_from_raw(raw, compilation.contract(), &Limits::default()).unwrap(),
        compilation.source_info().unwrap().clone()
    );
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
    let context = RuntimeContext::new(Limits {
        max_source_bytes: 8,
        ..Limits::default()
    });
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

struct IgnoringLimitsResolver {
    source: String,
    digest: Option<String>,
}

struct CancellingResolver {
    source: String,
    cancellation: CancellationToken,
}

impl SourceResolver for CancellingResolver {
    fn identify(&self, _from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        SourceId::parse(import)
    }

    fn load(&self, id: &SourceId, _limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        self.cancellation.cancel();
        Ok(ResolvedSource {
            id: id.clone(),
            source: self.source.clone(),
            digest: format!(
                "sha256:{}",
                hex::encode(Sha256::digest(self.source.as_bytes()))
            ),
        })
    }
}

impl SourceResolver for IgnoringLimitsResolver {
    fn identify(&self, _from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        SourceId::parse(import)
    }

    fn load(&self, id: &SourceId, _limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        Ok(ResolvedSource {
            id: id.clone(),
            source: self.source.clone(),
            digest: self.digest.clone().unwrap_or_else(|| {
                format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(self.source.as_bytes()))
                )
            }),
        })
    }
}

fn assert_source_limit(error: CompileError, limit: usize, observed: usize) {
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_bytes");
    assert_eq!(resource.limit, limit);
    assert_eq!(resource.observed, observed);
}

#[test]
fn compiler_owns_source_limit_enforcement_for_every_resolver_path() {
    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    let source = "service : {};";
    let limit = source.len() - 1;
    let context = RuntimeContext::new(Limits {
        max_source_bytes: limit,
        ..Limits::default()
    });

    let mut memory = MemoryResolver::new();
    memory.insert("entry.did", source).unwrap();
    let custom = IgnoringLimitsResolver {
        source: source.to_string(),
        digest: None,
    };
    let directory = std::env::temp_dir().join(format!(
        "candid-core-source-limits-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("entry.did");
    std::fs::write(&path, source).unwrap();

    for include_source_info in [false, true] {
        let options = CompileOptions {
            include_source_info,
        };
        assert_source_limit(
            compile_did_with_context(source, options, &context).unwrap_err(),
            limit,
            source.len(),
        );
        assert_source_limit(
            compile_with_resolver("entry.did", &memory, options, &context).unwrap_err(),
            limit,
            source.len(),
        );
        assert_source_limit(
            compile_with_resolver("entry.did", &custom, options, &context).unwrap_err(),
            limit,
            source.len(),
        );
        assert_source_limit(
            compile_did_file_with_context(&path, options, &context).unwrap_err(),
            limit,
            source.len(),
        );
    }

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compiler_source_accounting_accepts_exact_boundaries_and_rejects_next_source() {
    let source = "service : {};";
    let resolver = IgnoringLimitsResolver {
        source: source.to_string(),
        digest: None,
    };
    let exact = RuntimeContext::new(Limits {
        max_source_bytes: source.len(),
        max_bundle_bytes: source.len(),
        max_sources: 1,
        ..Limits::default()
    });
    compile_with_resolver("entry.did", &resolver, CompileOptions::default(), &exact).unwrap();

    for (limits, resource, limit, observed) in [
        (
            Limits {
                max_sources: 0,
                ..Limits::default()
            },
            "sources",
            0,
            1,
        ),
        (
            Limits {
                max_bundle_bytes: source.len() - 1,
                ..Limits::default()
            },
            "bundle_bytes",
            source.len() - 1,
            source.len(),
        ),
    ] {
        for include_source_info in [false, true] {
            let error = compile_did_with_context(
                source,
                CompileOptions {
                    include_source_info,
                },
                &RuntimeContext::new(limits.clone()),
            )
            .unwrap_err();
            let info = error.diagnostics[0].resource_limit.as_ref().unwrap();
            assert_eq!(info.resource, resource);
            assert_eq!(info.limit, limit);
            assert_eq!(info.observed, observed);
        }
    }
}

#[test]
fn compiler_checks_source_limits_before_digesting_resolver_content() {
    let source = "service : {};";
    let resolver = IgnoringLimitsResolver {
        source: source.to_string(),
        digest: Some("sha256:not-a-digest".to_string()),
    };
    let limit = source.len() - 1;
    let context = RuntimeContext::new(Limits {
        max_source_bytes: limit,
        ..Limits::default()
    });
    let error = compile_with_resolver("entry.did", &resolver, CompileOptions::default(), &context)
        .unwrap_err();
    assert_source_limit(error, limit, source.len());
}

#[test]
fn elapsed_deadlines_abort_work_without_partial_artifacts() {
    let context = RuntimeContext::new(Limits {
        deadline_unix_ms: Some(1),
        ..Limits::default()
    });
    let error =
        compile_did_with_context("service : {};", CompileOptions::default(), &context).unwrap_err();
    assert_eq!(error.diagnostics[0].code, "operation_deadline_exceeded");
}

#[test]
fn resolver_context_methods_enforce_elapsed_deadlines() {
    let mut resolver = MemoryResolver::new();
    resolver
        .insert("memory:/entry.did", "service : {};")
        .unwrap();
    let id = SourceId::parse("memory:/entry.did").unwrap();
    let context = RuntimeContext::new(Limits {
        deadline_unix_ms: Some(1),
        ..Limits::default()
    });

    let load_error = resolver.load_with_context(&id, &context).unwrap_err();
    assert_eq!(load_error.code, "operation_deadline_exceeded");

    let resolve_error = resolver
        .resolve_with_context(None, "memory:/entry.did", &context)
        .unwrap_err();
    assert_eq!(resolve_error.code, "operation_deadline_exceeded");
}

#[test]
fn runtime_context_cancellation_is_cooperative_and_not_serialized() {
    let compilation = compile("service : {};");
    let token = CancellationToken::new();
    let context = RuntimeContext::new(Limits::default()).with_cancellation(token.clone());
    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 1);
    assert!(json.get("limits").is_some());
    let decoded: RuntimeContext = serde_json::from_value(json).unwrap();
    assert!(!decoded.cancellation_token().is_cancelled());

    token.cancel();
    assert_eq!(decoded, context);
    let compile_error =
        compile_did_with_context("service : {};", CompileOptions::default(), &context).unwrap_err();
    assert_eq!(compile_error.diagnostics[0].code, "operation_cancelled");

    let contract_error = compilation
        .contract()
        .validate_with_context(&context)
        .unwrap_err();
    assert_eq!(contract_error.violations[0].code, "operation_cancelled");

    let json_error = HostValue::from_json_with_context(r#"{"kind":"null"}"#, &context).unwrap_err();
    assert!(matches!(
        json_error,
        candid_core::HostValueJsonError::Cancelled { ref path } if path == "$"
    ));
}

#[test]
fn resolver_cancellation_is_observed_before_accepting_loaded_content() {
    let token = CancellationToken::new();
    let resolver = CancellingResolver {
        source: "service : {};".to_string(),
        cancellation: token.clone(),
    };
    let context = RuntimeContext::new(Limits::default()).with_cancellation(token);
    let error = compile_with_resolver(
        "memory:/entry.did",
        &resolver,
        CompileOptions::default(),
        &context,
    )
    .unwrap_err();
    assert_eq!(error.diagnostics[0].code, "operation_cancelled");
}

#[test]
fn provenance_budget_failures_retain_stable_resource_metadata() {
    let compilation = compile("service : { ping: () -> () query };");
    let raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();
    assert_eq!(raw.methods.len(), 1);
    let context = RuntimeContext::new(Limits {
        max_methods: 0,
        ..Limits::default()
    });
    let error =
        SourceInfo::try_from_raw_with_context(raw, compilation.contract(), &context).unwrap_err();
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    let resource = violation.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_methods");
    assert_eq!(resource.limit, 0);
    assert_eq!(resource.observed, 1);
}

#[test]
fn one_operation_cannot_reset_canonicalization_work_between_stages() {
    let compilation = compile("service : {};");
    let raw = RawContract::from(compilation.contract());
    let nodes = raw.types.len();
    assert_eq!(nodes, 1);
    let limit = nodes * 3;
    let context = RuntimeContext::new(Limits {
        max_canonicalization_work: limit,
        ..Limits::default()
    });
    let error = Contract::try_from_raw_with_context(raw, &context).unwrap_err();
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    let resource = violation.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "canonicalization_work");
    assert_eq!(resource.limit, limit);
    assert_eq!(resource.observed, nodes * 4);
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
    assert!(ContractEnvelope::from_json_with_limits(&raw.to_string(), &Limits::default()).is_err());
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
    let decoded = Contract::from_json(&raw.to_string()).unwrap();
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

fn conformance_fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/conformance/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Issue #13: an actorless Contract's identity preimage is pinned to exact
/// bytes that omit the `actor` property. The digest is recomputed here from
/// the fixture's literal bytes with `sha2` alone, so a change to the payload
/// shape, JCS writer, domain tag, or hash construction moves this test even
/// if serialization and hashing drift in lockstep.
#[test]
fn actorless_identity_bytes_and_id_match_the_pinned_golden_fixture() {
    let golden: serde_json::Value =
        serde_json::from_str(&conformance_fixture("actorless.identity.json")).unwrap();
    let domain = golden["domain"].as_str().unwrap();
    let jcs = golden["jcs"].as_str().unwrap();
    let contract_id = golden["contract_id"].as_str().unwrap();

    assert_eq!(domain, "candid-core:contract:v1");
    assert!(
        !jcs.contains("\"actor\""),
        "the pinned actorless identity payload must omit the actor property"
    );
    assert_eq!(
        hex::decode(golden["jcs_hex"].as_str().unwrap()).unwrap(),
        jcs.as_bytes(),
        "the pinned JCS text and its pinned UTF-8 bytes must agree"
    );

    let mut preimage = domain.as_bytes().to_vec();
    preimage.push(0);
    preimage.extend_from_slice(jcs.as_bytes());
    assert_eq!(
        hex::decode(golden["preimage_hex"].as_str().unwrap()).unwrap(),
        preimage,
        "the pinned domain preimage must be domain, one zero byte, then the JCS bytes"
    );
    assert_eq!(
        contract_id,
        format!("{domain}:sha256:{}", hex::encode(Sha256::digest(&preimage))),
        "the pinned Contract ID must be the SHA-256 of the pinned preimage"
    );

    let did = format!(
        "{}/tests/fixtures/conformance/actorless.did",
        env!("CARGO_MANIFEST_DIR")
    );
    let contract = compile_did_file(did).unwrap().into_parts().0;
    assert_eq!(contract.contract_id(), contract_id);
    assert_eq!(contract.interface_id(), None);
}

/// Issue #13: absence is the only v1 wire spelling of "no actor". `None`
/// serializes as an omitted property on every serialization path, and an
/// explicit `"actor": null` fails decoding instead of aliasing omission.
#[test]
fn actorless_contracts_omit_actor_on_the_wire_and_reject_explicit_null() {
    let contract = compile("type OnlyType = nat;").contract().clone();

    let wire = serde_json::to_value(&contract).unwrap();
    assert!(
        !wire.as_object().unwrap().contains_key("actor"),
        "canonical serialization must omit an absent actor, not encode null"
    );
    let raw_wire = serde_json::to_value(RawContract::from(&contract)).unwrap();
    assert!(!raw_wire.as_object().unwrap().contains_key("actor"));
    assert!(!contract.to_json_pretty().unwrap().contains("\"actor\""));

    let reparsed = Contract::from_json(&wire.to_string()).unwrap();
    assert_eq!(reparsed.contract_id(), contract.contract_id());

    let mut with_null = wire;
    with_null["actor"] = serde_json::Value::Null;
    let error = Contract::from_json(&with_null.to_string()).unwrap_err();
    assert!(
        matches!(error, candid_core::ContractJsonError::MalformedJson(_)),
        "an explicit actor null must fail JSON decoding, found {error:?}"
    );
    assert!(
        serde_json::from_value::<RawContract>(with_null).is_err(),
        "the raw DTO decode path must also reject an explicit actor null"
    );
}

/// Issue #13: omitting the absent actor must not collapse an actorless
/// Contract into one with an explicitly empty service. The empty-actor IDs
/// are pinned inline as the cross-release regression anchor proving the
/// actorless encoding fix left Contracts with an actor untouched.
#[test]
fn actorless_and_explicitly_empty_actor_stay_distinct_on_the_wire() {
    let actorless = compile("type OnlyType = nat;").contract().clone();
    let empty_actor = compile("service : {};").contract().clone();

    let empty_wire = serde_json::to_value(&empty_actor).unwrap();
    assert_eq!(
        empty_wire["actor"],
        serde_json::json!({ "kind": "service", "service": 0 }),
        "an explicitly empty service keeps its actor property"
    );
    assert_ne!(actorless.contract_id(), empty_actor.contract_id());

    assert_eq!(
        empty_actor.contract_id(),
        "candid-core:contract:v1:sha256:edfb9052c7234db653686a79e02e317241b4ccfe74e989a2f5735aa2c8a70ba5"
    );
    assert_eq!(
        empty_actor.interface_id().unwrap(),
        "candid-core:interface:v1:sha256:ea1e1478267a0f0bdffeb8f5acc0ac91c3a496f285c46c10c104fac61698e401"
    );
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
