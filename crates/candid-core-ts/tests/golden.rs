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

/// The issue #104 headline: the real ledger corpus — whose nested
/// `ArchiveCallback` func made generation refuse for three slices — now
/// generates end-to-end. The fixture is a byte-identical copy of the corpus
/// file, pinned in sync below so the two can never drift.
#[test]
fn golden_ledger() {
    assert_golden("ledger");
}

#[test]
fn ledger_fixture_matches_the_corpus() {
    let fixture = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ledger.did"),
    )
    .expect("fixture must be readable");
    let corpus = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benches/corpus/ledger.did"),
    )
    .expect("corpus must be readable");
    assert_eq!(
        fixture, corpus,
        "the ledger fixture mirrors the corpus byte-for-byte"
    );
}

/// `"__proto__"` is a legal quoted Candid name; the builder must render it as
/// a computed key, because a non-computed one sets the prototype at runtime —
/// a divergence the tsc equality gate provably cannot see (#114).
#[test]
fn golden_proto() {
    assert_golden("proto");
}

/// Issue #126: `empty` in a record field, tuple element, and func arg — the
/// positions the `AnyFieldSchema` bound admits — generates modules the tsc
/// equality gate accepts, with `vec empty`/`opt empty` pinned as the
/// unchanged controls. Variant arms live in the `arms` fixture (#127).
#[test]
fn golden_empties() {
    assert_golden("empties");
}

/// Issue #127: the variant-arm classification rows — `empty` keeps
/// `value: never`, anonymous `opt empty` keeps `value: never | null`, and a
/// declared alias of `null` is a bare tag through the reference — generate
/// modules the tsc equality gate accepts, proving `VariantInfer` and the
/// emitter classify identically for every emitted shape.
#[test]
fn golden_arms() {
    assert_golden("arms");
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
        "ledger",
        "empties",
        "arms",
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
        let mut names_text =
            serde_json::to_string_pretty(&serde_json::Value::Array(entries.clone()))
                .expect("name table must serialize");
        names_text.push('\n');
        assert_golden_file(&format!("{name}.names.json"), &names_text);

        // The one-document form (issue #152): the same contract and the same
        // triples as one `ContractEnvelope` carrying the recorded
        // `org.candid-core.field-names/v1` extension — built through the real
        // envelope type so the extension name and value pass the envelope's
        // own validation, exactly as the `candid-core compile --envelope`
        // path builds it. `ts/tests/crosscheck.test.ts` proves this document
        // yields verdict-for-verdict the same schemas as the two files above.
        let mut envelope = candid_core::ContractEnvelope::new(reparsed);
        envelope
            .insert_extension(
                "org.candid-core.field-names/v1",
                serde_json::Value::Array(entries),
                &candid_core::Limits::default(),
            )
            .expect("the field-names extension must validate");
        let envelope_value = serde_json::to_value(&envelope).expect("envelope must serialize");
        let mut envelope_text =
            serde_json::to_string_pretty(&envelope_value).expect("envelope must pretty-print");
        envelope_text.push('\n');
        assert_golden_file(&format!("{name}.envelope.json"), &envelope_text);
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
/// than silently emitting `unknown`. Since issue #104 lifted the func
/// deferral, both the anonymous and the named nesting generate — the value
/// type is the inert `{ principal, method }` reference — and the standing
/// refusal is pinned in reverse.
#[test]
fn nested_func_generates() {
    let anonymous = compile_did("type Holder = record { hook : func (nat) -> (text) };")
        .expect("source must compile");
    let names = TsNames::from_source_info(anonymous.source_info().expect("provenance"));
    let output = generate_module(anonymous.contract(), &names, &TsOptions::default())
        .expect("anonymous nested func generates since #104");
    assert!(
        output.contains("hook: { principal: PrincipalValue; method: string }"),
        "{output}"
    );
    assert!(
        output.contains("c.func([c.nat], [c.text], \"update\")"),
        "{output}"
    );

    let named = compile_did(
        "type Callback = func (nat) -> (text) query;\ntype Holder = record { hook : Callback };",
    )
    .expect("source must compile");
    let names = TsNames::from_source_info(named.source_info().expect("provenance"));
    let output = generate_module(named.contract(), &names, &TsOptions::default())
        .expect("a reference to a func alias generates since #104");
    assert!(
        output.contains("export type Callback = { principal: PrincipalValue; method: string };"),
        "{output}"
    );
    assert!(output.contains("hook: Callback"), "{output}");
    assert!(output.contains("\"query\""), "{output}");
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
/// refused instead, unconditionally: `PrincipalValue` is reserved even in a
/// contract that never uses the principal primitive (issue #116; the
/// reserved name tracked the import when #150 replaced the SDK `Principal`
/// with the structural type).
#[test]
fn import_shadowing_declaration_names_are_refused() {
    for source in [
        "type c = nat8;",
        "type Schema = nat8;",
        "type PrincipalValue = record { p : principal };",
        // No principal primitive anywhere: still refused by decision.
        "type PrincipalValue = nat8;",
        // The ambient types the lowerings emit are bindings too: a
        // declaration by these names shadows them module-wide.
        "type Array = nat8; type V = vec text;",
        "type Record = nat8; type E = record {};",
        "type Uint8Array = text; type B = blob;",
        // And unconditionally, without the lowering that references them.
        "type Array = nat8;",
        // The actor lowering's ambient `Promise` is a binding too, and the
        // same unconditional rule applies without any actor (issue #130).
        "type Promise = nat8;",
        // The actor surface's own emission names, reserved since #104 —
        // exercised here rather than assumed, actor or not.
        "type actor = nat8;",
        "type Actor = nat8; service : { ping : () -> () };",
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
    // Near-misses are ordinary names — including `Principal` itself, which
    // stopped being a referenced binding when #150 switched the emitted
    // import to `PrincipalValue`: a contract may now declare it freely.
    for near_miss in ["type Principal2 = nat8;", "type Principal = nat8;"] {
        let compilation = compile_did(near_miss).expect("compile");
        let output = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .expect("a non-referenced name is not reserved");
        assert!(output.contains("export type Principal"));
    }
}

/// The actor lowering references the ambient global `Promise` on every
/// method signature, so a declaration by that name shadows it module-wide
/// and the emitted `Promise<T>` stops compiling (TS2315) — the same
/// cannot-compile class as `Array`/`Record`/`Uint8Array`, refused with the
/// same unconditional rule (issue #130).
#[test]
fn promise_shadowing_declaration_is_refused() {
    // The original repro: an actor plus a declaration literally named
    // `Promise` passed both name guards and generated known-broken text.
    let source = "type Promise = record { id : nat }; service : { ping : () -> () };";
    let compilation = compile_did(source).expect("compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    let error = generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect_err("a Promise declaration must refuse generation");
    assert!(
        matches!(&error, TsGenError::ReservedDeclarationName { name } if name == "Promise"),
        "the error must name the collision: {error}"
    );
    // The reservation is the exact name: a case variant is an ordinary
    // declaration and coexists with the actor's `Promise<void>`.
    let compilation =
        compile_did("type promise = nat8; service : { ping : () -> () };").expect("compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    let output = generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect("a case variant is not reserved");
    assert!(output.contains("export type promise"), "{output}");
    assert!(output.contains("Promise<void>"), "{output}");
}

/// Issue #127: a variant arm whose payload is a *declared* `opt` of a
/// never-domain type renders as a bare reference statically identical to a
/// declared alias of `null` — the one shape no type-level classification
/// can carry — and is refused instead of emitting text the equality gate
/// would reject. The anonymous form and the null alias stay generable; the
/// `arms` golden pins them.
#[test]
fn declared_opt_empty_variant_arms_are_refused() {
    for source in [
        "type W = opt empty; type V = variant { a : W; b : nat };",
        // The inner may be a declared alias of empty: same arena node.
        "type E = empty; type W = opt E; type V = variant { a : W };",
        // An empty variant is never-domain too.
        "type Never = variant {}; type W = opt Never; type V = variant { a : W };",
        // Dedup alone declares the arm: an anonymous arm and a same-shape
        // declaration share one node, so the arm renders as the name.
        "type V = variant { a : opt empty }; type W = opt empty;",
    ] {
        let compilation = compile_did(source).expect("compile");
        let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
        let error = generate_module(compilation.contract(), &names, &TsOptions::default())
            .expect_err("a declared opt-empty arm must refuse generation");
        assert!(
            matches!(
                &error,
                TsGenError::AmbiguousVariantArm { declaration, arm }
                    if declaration == "V" && arm == "a"
            ),
            "unexpected error for {source}: {error}"
        );
        assert!(
            !error.to_string().contains("  "),
            "error message carries embedded space runs: {error}"
        );
    }
    // Near-misses stay generable: an opt of an inhabited type through a
    // declaration is an ordinary valued arm — the inhabited *variant* case
    // pins the `fields.is_empty()` discrimination in the refusal itself…
    for source in [
        "type W = opt nat; type V = variant { a : W };",
        "type S = variant { x }; type W = opt S; type V = variant { a : W };",
    ] {
        let compilation = compile_did(source).expect("compile");
        let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
        generate_module(compilation.contract(), &names, &TsOptions::default())
            .expect("an opt of an inhabited type is not ambiguous");
    }
    // …and an opt of an uninhabited *record* keeps `value` on its own: the
    // reference's static type is `{ f: never } | null`, not `null`, so the
    // type level classifies it without help.
    let compilation = compile_did("type W = opt record { f : empty }; type V = variant { a : W };")
        .expect("compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect("a non-never-alias inner is not ambiguous");
}

/// The actor surface's sharp edges (issue #104 review): a class actor
/// unwraps to its running service, and a method legally named `__proto__`
/// must render as a computed key — a plain literal would set the prototype
/// and silently drop the method from the schema.
#[test]
fn actor_emission_covers_class_unwrap_and_proto_methods() {
    let class_actor =
        compile_did("service : (nat) -> { ping : () -> () };").expect("a class actor compiles");
    let names = TsNames::from_source_info(class_actor.source_info().expect("provenance"));
    let output = generate_module(class_actor.contract(), &names, &TsOptions::default())
        .expect("a class actor generates its running service");
    assert!(output.contains("export const actor"), "{output}");
    assert!(output.contains("ping: () => Promise<void>;"), "{output}");
    assert!(
        output.contains("init args are install-time"),
        "the class note must be present: {output}"
    );

    let proto = compile_did("service : { \"__proto__\" : () -> () };")
        .expect("a __proto__ method compiles");
    let names = TsNames::from_source_info(proto.source_info().expect("provenance"));
    let output =
        generate_module(proto.contract(), &names, &TsOptions::default()).expect("generate");
    assert!(
        output.contains("c.service({ [\"__proto__\"]: c.func"),
        "the method key must be computed: {output}"
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
