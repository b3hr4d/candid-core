//! Golden tests: each `.did` fixture compiles to a Contract, generates through
//! the real `SourceInfo` name bridge, and must match its checked-in `.ts`
//! byte-exactly. Regenerate deliberately with `UPDATE_GOLDENS=1`; the diff is
//! then reviewed like any other code change, because the goldens are where the
//! per-type mapping decisions live.
//!
//! Gated on this crate's `compiler` feature so the bridge under test is the
//! shipped one, not a test reimplementation: run with
//! `cargo test -p candid-core-ts --features compiler`.
#![cfg(feature = "compiler")]

use std::path::PathBuf;

use candid_core::compile_did;
use candid_core_ts::{generate_module, TsGenError, TsNames, TsOptions};

fn generate_fixture(name: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let source = std::fs::read_to_string(root.join("fixtures").join(format!("{name}.did")))
        .expect("fixture must be readable");
    let compilation = compile_did(&source).expect("fixture must compile");
    let names = TsNames::from_source_info(
        compilation
            .source_info()
            .expect("compile_did retains provenance by default"),
    );
    generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect("fixture must generate")
}

fn assert_golden(name: &str) {
    let generated = generate_fixture(name);
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(format!("{name}.ts"));
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&golden_path, &generated).expect("golden must be writable");
        return;
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|_| panic!("missing golden {golden_path:?}; run with UPDATE_GOLDENS=1"));
    assert_eq!(
        generated, golden,
        "generated TypeScript for `{name}` diverged from its golden; \
         if the change is intended, regenerate with UPDATE_GOLDENS=1 and review the diff"
    );
}

#[test]
fn golden_primitives() {
    assert_golden("primitives");
}

#[test]
fn golden_collections() {
    assert_golden("collections");
}

#[test]
fn golden_variants() {
    assert_golden("variants");
}

#[test]
fn golden_recursion() {
    assert_golden("recursion");
}

#[test]
fn golden_quoting() {
    assert_golden("quoting");
}

#[test]
fn golden_deferred() {
    assert_golden("deferred");
}

/// Byte-identical output for the same Contract, and for the same Contract
/// round-tripped through its serialized form — determinism is a pinned
/// property, not an aspiration.
#[test]
fn generation_is_deterministic_across_serde_round_trips() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/recursion.did"),
    )
    .expect("fixture must be readable");
    let compilation = compile_did(&source).expect("fixture must compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    let options = TsOptions::default();

    let first = generate_module(compilation.contract(), &names, &options).expect("generate");
    let second = generate_module(compilation.contract(), &names, &options).expect("generate");
    assert_eq!(first, second);

    let json = serde_json::to_string(compilation.contract()).expect("serialize");
    let reparsed = candid_core::Contract::from_json(&json).expect("reparse");
    let third = generate_module(&reparsed, &names, &options).expect("generate");
    assert_eq!(first, third);
}

/// A deferred construct nested inside a supported type fails closed rather
/// than silently emitting `unknown` — whether it is anonymous or reached
/// through a declared name. The named case is the sharper one: the alias the
/// reference would name was *skipped*, so emitting it would produce an
/// undefined TypeScript type while reporting success (review finding on the
/// first slice, and this test originally constructed exactly that case while
/// classifying it as safe).
#[test]
fn nested_func_fails_closed() {
    let anonymous = compile_did("type Holder = record { hook : func (nat) -> (text) };")
        .expect("source must compile");
    let error = generate_module(anonymous.contract(), &TsNames::new(), &TsOptions::default())
        .expect_err("anonymous nested func must fail closed");
    assert!(matches!(
        error,
        TsGenError::UnsupportedConstruct { kind: "func", .. }
    ));

    let named = compile_did(
        "type Callback = func (nat) -> (text);\ntype Holder = record { hook : Callback };",
    )
    .expect("source must compile");
    let error = generate_module(named.contract(), &TsNames::new(), &TsOptions::default())
        .expect_err("a reference to a skipped deferred alias must fail closed");
    assert!(matches!(
        error,
        TsGenError::UnsupportedConstruct { kind: "func", .. }
    ));
}

/// `T | null` cannot carry Candid optionality when the inner type can itself
/// be `null` in TypeScript. Every collapsing shape fails closed — including an
/// opt reached through a declared alias, because the check is on the node, not
/// its spelling.
#[test]
fn collapsing_options_fail_closed() {
    for (source, inner) in [
        ("type DoubleOpt = opt opt nat;", "opt"),
        ("type OptNull = opt null;", "null"),
        ("type OptReserved = opt reserved;", "reserved"),
        ("type Inner = opt nat;\ntype Outer = opt Inner;", "opt"),
    ] {
        let compilation = compile_did(source).expect("source must compile");
        let error = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .expect_err(source);
        match error {
            TsGenError::UnrepresentableOption { inner: got, .. } => {
                assert_eq!(got, inner, "{source}")
            }
            other => panic!("expected UnrepresentableOption for {source}, got {other:?}"),
        }
    }
}

/// The caller-supplied module specifier is escaped, never interpolated: a
/// hostile or accidental quote cannot produce syntactically invalid output.
#[test]
fn principal_import_is_escaped() {
    let compilation = compile_did("type Who = principal;").expect("compile");
    let options = TsOptions {
        principal_import: "bad\"path".to_string(),
    };
    let output =
        generate_module(compilation.contract(), &TsNames::new(), &options).expect("generate");
    assert!(
        output.contains("from \"bad\\\"path\";"),
        "specifier must be escaped: {output}"
    );
}

/// Without a name table every field renders by the `_id_` convention — the
/// documented base-surface behaviour, pinned so it cannot drift silently.
#[test]
fn missing_names_render_by_id_convention() {
    let compilation =
        compile_did("type Item = record { id : nat32; label : text };").expect("compile");
    let output = generate_module(
        compilation.contract(),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("generate");
    let id = candid_parser_id("id");
    let label = candid_parser_id("label");
    assert!(output.contains(&format!("_{id}_")), "{output}");
    assert!(output.contains(&format!("_{label}_")), "{output}");
}

/// The reference hash for a Candid field name, taken from the official
/// implementation the repository already pins as its authority.
fn candid_parser_id(name: &str) -> u32 {
    candid_parser::candid::idl_hash(name)
}
