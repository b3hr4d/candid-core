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

fn assert_golden_file(file_name: &str, generated: &str) {
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(file_name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&golden_path, generated).expect("golden must be writable");
        return;
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|_| panic!("missing golden {golden_path:?}; run with UPDATE_GOLDENS=1"));
    assert_eq!(
        generated, golden,
        "generated content for `{file_name}` diverged from its golden; \
         if the change is intended, regenerate with UPDATE_GOLDENS=1 and review the diff"
    );
}

fn assert_golden(name: &str) {
    assert_golden_file(&format!("{name}.ts"), &generate_fixture(name));
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

/// `"__proto__"` is a legal quoted Candid name; the builder must render it as
/// a computed key, because a non-computed one sets the prototype at runtime —
/// a divergence the tsc equality gate provably cannot see (#114).
#[test]
fn golden_proto() {
    assert_golden("proto");
}

/// The schema runtime (issue #102) consumes these same fixtures as data: each
/// fixture's Contract JSON document and field-name table are goldens too,
/// read by `ts/tests/crosscheck.test.ts` to prove the dynamically built
/// schema validates exactly the values the generated builder describes.
///
/// Two deliberate normalizations, both proven harmless by re-validating the
/// result through `Contract::from_json`:
/// - the `producer` block is a fixed sentinel, so release version bumps do
///   not churn these goldens — producer metadata is untrusted provenance
///   excluded from the semantic identities by documented design;
/// - the document is pretty-printed with serde_json's sorted keys for
///   reviewability. It is a valid Contract document in the canonical format,
///   not the canonical byte serialization, which nothing here consumes.
#[test]
fn golden_runtime_contract_documents() {
    for name in [
        "primitives",
        "collections",
        "variants",
        "recursion",
        "quoting",
        "deferred",
        "proto",
    ] {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
        let source = std::fs::read_to_string(root.join("fixtures").join(format!("{name}.did")))
            .expect("fixture must be readable");
        let compilation = compile_did(&source).expect("fixture must compile");

        let mut document =
            serde_json::to_value(compilation.contract()).expect("contract must serialize");
        document["producer"] = serde_json::json!({
            "name": "candid-core",
            "version": "0.0.0-golden",
            "candid_version": "0.0.0-golden",
            "candid_parser_version": "0.0.0-golden",
        });
        let normalized = serde_json::to_string(&document).expect("document must serialize");
        let reparsed = candid_core::Contract::from_json(&normalized)
            .expect("the normalized document must still be a valid canonical Contract");
        let pretty = serde_json::to_value(&reparsed).expect("contract must serialize");
        let mut text = serde_json::to_string_pretty(&pretty).expect("document must pretty-print");
        text.push('\n');
        assert_golden_file(&format!("{name}.contract.json"), &text);

        // The name table the runtime needs: `(container, id, name)` triples,
        // named labels only — a numeric Candid label has no name and must
        // render by the `_id_` convention, the same rule as
        // `TsNames::from_source_info`.
        let source_info = compilation
            .source_info()
            .expect("compile_did retains provenance by default");
        let mut triples: Vec<(u32, u32, &str)> = source_info
            .field_labels()
            .iter()
            .filter_map(|provenance| match &provenance.label {
                candid_core::SourceLabel::Named { name } => {
                    Some((provenance.container, provenance.id, name.as_str()))
                }
                _ => None,
            })
            .collect();
        triples.sort();
        triples.dedup();
        let entries: Vec<serde_json::Value> = triples
            .iter()
            .map(|(container, id, label)| serde_json::json!([container, id, label]))
            .collect();
        let mut names_text = serde_json::to_string_pretty(&serde_json::Value::Array(entries))
            .expect("name table must serialize");
        names_text.push('\n');
        assert_golden_file(&format!("{name}.names.json"), &names_text);
    }
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

/// A source name shaped like the `_N_` id rendering is refused (issue #115):
/// erased to a schema key it is indistinguishable from numeric label id N,
/// so the codec would encode wire id N instead of the name's hash — the
/// silent-wrong-id fail-open the reservation exists to prevent. The loader
/// enforces the identical reservation on its name table (issue #103), so
/// both paths reject the same documents; the TS suite pins the loader half.
#[test]
fn numeric_shaped_source_names_are_refused() {
    for source in [
        "type Holder = record { _123_ : nat8 };",
        "type Event = variant { _123_ : nat8; idle };",
        // The original #115 collision repro: the reservation refuses it
        // before any duplicate key could render.
        "type T = record { _123_ : nat8; 123 : text };",
    ] {
        let compilation = compile_did(source).expect("compile");
        let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
        let error = generate_module(compilation.contract(), &names, &TsOptions::default())
            .expect_err("a _N_-shaped source name must refuse generation");
        assert!(
            matches!(&error, TsGenError::ReservedFieldName { name, .. } if name == "_123_"),
            "unexpected error for {source}: {error}"
        );
    }
    // Non-canonical shapes are ordinary names: a leading zero never renders
    // from a numeric id, so `_007_` stays hash-addressed and unambiguous.
    let compilation = compile_did("type Ok = record { _007_ : nat8 };").expect("compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    let output = generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect("a non-canonical shape is not reserved");
    assert!(output.contains("_007_"), "the name must render: {output}");
}

/// A declaration named after one of the module's own imports would emit a
/// module that can never load (`export const c` against `import { c }`) —
/// refused instead, unconditionally: `Principal` is reserved even in a
/// contract that never uses the principal primitive (issue #116).
#[test]
fn import_shadowing_declaration_names_are_refused() {
    for source in [
        "type c = nat8;",
        "type Schema = nat8;",
        "type Principal = record { p : principal };",
        // No principal primitive anywhere: still refused by decision.
        "type Principal = nat8;",
        // The ambient types the lowerings emit are bindings too: a
        // declaration by these names shadows them module-wide.
        "type Array = nat8; type V = vec text;",
        "type Record = nat8; type E = record {};",
        "type Uint8Array = text; type B = blob;",
        // And unconditionally, without the lowering that references them.
        "type Array = nat8;",
    ] {
        let compilation = compile_did(source).expect("compile");
        let error = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .expect_err("an import-shadowing declaration name must refuse generation");
        assert!(
            matches!(error, TsGenError::ReservedDeclarationName { .. }),
            "unexpected error for {source}: {error}"
        );
        // Message hygiene: a joined multi-line literal bakes indentation
        // into the user-facing text, which no format gate catches.
        assert!(
            !error.to_string().contains("  "),
            "error message carries embedded space runs: {error}"
        );
    }
    // Near-misses are ordinary names.
    let compilation = compile_did("type Principal2 = nat8;").expect("compile");
    let output = generate_module(
        compilation.contract(),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("a near-miss name is not reserved");
    assert!(output.contains("Principal2"));
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
