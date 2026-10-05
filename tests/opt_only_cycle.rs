//! Issue #234: the compiler refuses a type that lies on a cycle passing only
//! through `opt`, such as `type T = opt T;`.
//!
//! Decoding a value that is not an `opt` at such a type unwraps `opt` without
//! end (Candid's coercion has no finite derivation there), so the reference
//! `candid` crate stops only at its stack guard and a runtime only at its
//! depth budget. `candid_parser` accepts these types; candid-core refuses
//! them at compile time, on every entry point, with `did_type_check_error`
//! naming the type. Any other constructor on the cycle makes it productive,
//! and those types compile exactly as before. The Contract loaders do not
//! change: a Contract document that already holds such a type still loads.

use candid_core::{
    compile_did, compile_did_with_context, compile_did_with_options, compile_with_resolver,
    CompileError, CompileOptions, Contract, DiagnosticPhase, MemoryResolver, RawSourceInfo,
    RuntimeContext, SourceInfo, SourceSpan,
};
use sha2::{Digest, Sha256};

const INLINE: &str = "memory:/inline.did";

/// Every shape the rule refuses, with the type the diagnostic names: the
/// first declaration, in name order, that lies on such a cycle.
const REFUSED: &[(&str, &str)] = &[
    ("type T = opt T;", "T"),
    ("type T = opt opt T;", "T"),
    ("type T = opt opt opt opt T;", "T"),
    ("type A = opt B; type B = opt A;", "A"),
    ("type B = opt A; type A = opt B;", "A"),
    // An alias in the cycle: aliases are resolved before the rule applies.
    ("type A = B; type B = opt A;", "A"),
    ("type A = opt B; type B = C; type C = opt A;", "A"),
    // A cycle that only a method argument reaches.
    ("type T = opt T; service : { m : (opt T) -> () };", "T"),
    // Written inline in a service signature, in a func type's argument, and
    // in a class actor's init argument.
    ("type S = service { m : (A) -> () }; type A = opt A; service : S;", "A"),
    ("type F = func (opt B) -> (); type B = opt opt B;", "B"),
    ("type T = opt T; service : (opt T) -> {};", "T"),
    // Beside productive declarations, which do not hide it.
    (
        "type L = opt record { head : nat; tail : L }; type Z = opt Z; service : { m : (L) -> () };",
        "Z",
    ),
];

/// Shapes the rule accepts: a cycle with any constructor other than `opt` on
/// it is productive.
const ACCEPTED: &[&str] = &[
    "type T = opt record { T };",
    "type T = opt vec T;",
    "type T = opt variant { a : T };",
    "type T = record { a : opt T };",
    "type L = opt record { head : nat; tail : L };",
    "type T = opt func (T) -> ();",
    "type T = opt service { m : (T) -> () };",
    "type T = variant { a : opt T; b };",
    "type T = vec opt T;",
    "type A = opt B; type B = record { a : opt A };",
    // An opt chain that ends is no cycle.
    "type A = opt B; type B = opt C; type C = nat;",
    // An opt pointing *into* a productive cycle is not on an opt-only one.
    "type P = opt Q; type Q = record { next : opt Q };",
];

fn assert_refusal(error: &CompileError, name: &str, source_name: Option<&str>, case: &str) {
    assert_eq!(error.diagnostics.len(), 1, "{case}: {error:#?}");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "did_type_check_error", "{case}");
    assert_eq!(diagnostic.phase, Some(DiagnosticPhase::TypeCheck), "{case}");
    assert_eq!(
        diagnostic.message,
        format!(
            "type {name} lies on a cycle that passes only through opt; candid-core refuses \
             it, because decoding a value that is not an opt at such a type unwraps opt \
             without end"
        ),
        "{case}"
    );
    assert_eq!(
        diagnostic.span,
        source_name.map(SourceSpan::source_only),
        "{case}"
    );
    assert_eq!(diagnostic.path, None, "{case}");
    assert!(diagnostic.resource_limit.is_none(), "{case}");
}

#[test]
fn every_opt_only_cycle_is_refused_naming_the_type() {
    for (source, name) in REFUSED {
        let error = compile_did(source).expect_err(source);
        assert_refusal(&error, name, Some(INLINE), source);
    }
}

#[test]
fn the_upstream_checker_accepts_what_candid_core_refuses() {
    // The refusal is candid-core's own: `candid_parser` type-checks every
    // refused source, so no upstream diagnostic could have taken its place.
    for (source, _) in REFUSED {
        let program: candid_parser::syntax::IDLProg = source.parse().unwrap();
        let mut environment = candid_parser::candid::TypeEnv::new();
        candid_parser::check_prog(&mut environment, &program)
            .unwrap_or_else(|error| panic!("{source}: upstream refused: {error}"));
    }
}

#[test]
fn productive_cycles_and_finite_opt_chains_compile() {
    for source in ACCEPTED {
        compile_did(source).unwrap_or_else(|error| panic!("{source}: {error:#?}"));
    }
}

#[test]
fn the_refusal_does_not_depend_on_provenance_or_the_context() {
    for (source, name) in REFUSED {
        for include_source_info in [false, true] {
            let options = CompileOptions {
                include_source_info,
            };
            let error = compile_did_with_options(source, options).expect_err(source);
            assert_refusal(&error, name, Some(INLINE), source);
            let error = compile_did_with_context(source, options, &RuntimeContext::default())
                .expect_err(source);
            assert_refusal(&error, name, Some(INLINE), source);
        }
    }
}

#[test]
fn the_resolver_path_refuses_and_names_the_declaring_source() {
    let resolver = MemoryResolver::new()
        .with_source(
            "memory:/entry.did",
            "import \"types.did\";\nservice : { m : (opt T) -> () };",
        )
        .unwrap()
        .with_source("memory:/types.did", "type T = opt opt T;")
        .unwrap();
    for include_source_info in [false, true] {
        let error = compile_with_resolver(
            "memory:/entry.did",
            &resolver,
            CompileOptions {
                include_source_info,
            },
            &RuntimeContext::default(),
        )
        .expect_err("an imported opt-only cycle");
        assert_refusal(&error, "T", Some("memory:/types.did"), "imported");
    }
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn native_file_compilation_refuses_too() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "candid-core-opt-cycle-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("types.did"), "type B = opt A; type A = B;").unwrap();
    let entry = directory.join("service.did");
    std::fs::write(
        &entry,
        "import \"types.did\";\nservice : { m : (A) -> () };",
    )
    .unwrap();
    let outcome = candid_core::compile_did_file(&entry);
    let _ = std::fs::remove_dir_all(&directory);
    let error = outcome.expect_err("an imported opt-only cycle");
    let span = error.diagnostics[0].span.clone().expect("a source scope");
    assert_refusal(&error, "A", span.source_name.as_deref(), "native");
    assert!(
        span.source_name.as_deref().unwrap().ends_with("types.did"),
        "{span:?}"
    );
}

/// The source-bundle identity, computed independently of the crate: SHA-256
/// over the domain tag, a zero byte and the JCS bytes of `{imports,
/// sources}`. For these ASCII payloads `serde_json`'s sorted-key compact
/// output is the JCS form.
fn bundle_id(sources: &serde_json::Value, imports: &serde_json::Value) -> String {
    const DOMAIN: &str = "candid-core:source-bundle:v1";
    let payload = serde_json::json!({ "imports": imports, "sources": sources });
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_string(&payload).unwrap().as_bytes());
    format!("{DOMAIN}:sha256:{}", hex::encode(hasher.finalize()))
}

#[test]
fn source_info_rederivation_refuses_an_embedded_opt_only_cycle() {
    // A genuine sidecar, then its one source replaced by an opt-only cycle
    // and its bundle identity recomputed, so authentication reaches the
    // rederivation, which recompiles the bundle.
    let compilation = compile_did("type T = opt nat;\n").unwrap();
    let contract: Contract = compilation.contract().clone();
    let mut raw = serde_json::to_value(compilation.source_info().unwrap()).unwrap();
    assert_eq!(
        bundle_id(&raw["sources"], &raw["imports"]),
        raw["source_bundle_id"],
        "the independent identity must reproduce the crate's"
    );
    raw["sources"][0]["source"] = serde_json::json!("type T = opt T;\n");
    raw["source_bundle_id"] = serde_json::json!(bundle_id(&raw["sources"], &raw["imports"]));
    let raw: RawSourceInfo = serde_json::from_value(raw).unwrap();
    let error = SourceInfo::try_from_raw_with_context(raw, &contract, &RuntimeContext::default())
        .expect_err("rederivation must refuse the embedded cycle");
    assert_eq!(error.violations.len(), 1, "{error:#?}");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "did_type_check_error");
    assert_eq!(violation.path.as_deref(), Some("$"));
    assert!(
        violation.message.ends_with(
            "type T lies on a cycle that passes only through opt; candid-core refuses it, \
             because decoding a value that is not an opt at such a type unwraps opt without end"
        ),
        "{}",
        violation.message
    );
}

/// The Contract loaders do not change (issue #234's decision): a document
/// holding an opt-only cycle still loads. This one is the Contract the
/// compiler wrote for `type T0 = opt T0;` before the refusal, as the
/// differential corpus recorded it (environment `r/depth_opt_127_levels`,
/// producer block normalized), identities included.
const OPT_CYCLE_CONTRACT: &str = r#"{"canonicalization_profile":"candid-core-canon-1","declarations":[{"name":"T0","type":0}],"format":"candid-core","format_version":1,"identities":{"contract":"candid-core:contract:v1:sha256:b01b256c0d924acc4adb55a7e58c4b15966621bd01930bfedacd3e0d78f14486"},"producer":{"candid_parser_version":"0.0.0-golden","candid_version":"0.0.0-golden","name":"candid-core","version":"0.0.0-golden"},"semantics_profile":"candid-1","types":[{"inner":0,"kind":"opt"}]}"#;

#[test]
fn a_contract_document_holding_an_opt_only_cycle_still_loads() {
    let contract = Contract::from_json(OPT_CYCLE_CONTRACT).expect("Contract::from_json loads it");
    assert_eq!(
        contract.contract_id(),
        "candid-core:contract:v1:sha256:b01b256c0d924acc4adb55a7e58c4b15966621bd01930bfedacd3e0d78f14486"
    );
    // The same graph built through the model, and the compiler's refusal of
    // the source that used to produce it.
    let built = candid_core::ContractDraft::new(
        vec![candid_core::TypeNode::Opt { inner: 0 }],
        vec![candid_core::Declaration {
            name: "T0".to_string(),
            ty: 0,
        }],
        None,
    )
    .build()
    .expect("the model accepts an opt-only cycle");
    assert_eq!(built.contract_id(), contract.contract_id());
    let error = compile_did("type T0 = opt T0;\n").expect_err("the compiler refuses it");
    assert_refusal(&error, "T0", Some(INLINE), "T0");
}
