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

/// The `(container, id, name)` triples the goldens and the envelope carry:
/// named labels only, one name per `(container, id)`, retained with
/// `TsNames::from_source_info`'s own overwrite order (later provenance
/// wins), emitted in key order. The collapse is load-bearing: two spellings
/// of one label id are reachable from ordinary Candid — hash-colliding
/// names in structurally identical declarations deduplicate to one semantic
/// node — and emitting both would let the loader's last-entry-wins table
/// render a different key than the generated builder. The native binary's
/// `field_name_triples` mirrors this derivation exactly.
fn name_triples(source_info: &candid_core::SourceInfo) -> Vec<(u32, u32, String)> {
    let mut names: std::collections::BTreeMap<(u32, u32), &str> = std::collections::BTreeMap::new();
    for provenance in source_info.field_labels() {
        if let candid_core::SourceLabel::Named { name } = &provenance.label {
            names.insert((provenance.container, provenance.id), name.as_str());
        }
    }
    names
        .into_iter()
        .map(|((container, id), label)| (container, id, label.to_string()))
        .collect()
}

/// Hash-colliding spellings collapse to the name the generator renders: the
/// two record declarations below are structurally identical (their fields
/// share one label id — `cemxzwyk` and `amxawvks` are a Candid hash
/// collision), so canonicalization deduplicates them into one semantic node
/// with two provenance spellings. The emitted table must carry exactly the
/// spelling `TsNames` retains, or the envelope path would render a
/// different field key than the generated module (PR #159 review).
#[test]
fn hash_colliding_spellings_collapse_to_the_generator_name() {
    let source = "type A = record { cemxzwyk : nat };\ntype B = record { amxawvks : nat };";
    let compilation = compile_did(source).expect("compile");
    let source_info = compilation.source_info().expect("provenance");

    let triples = name_triples(source_info);
    assert_eq!(
        triples.len(),
        1,
        "one entry per (container, id): {triples:?}"
    );
    let winner = triples[0].2.as_str();
    assert!(
        winner == "cemxzwyk" || winner == "amxawvks",
        "{winner:?} must be one of the colliding spellings"
    );

    let names = TsNames::from_source_info(source_info);
    let module = generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect("the colliding source generates");
    assert!(
        module.contains(&format!("{winner}: $.c.nat")),
        "the generated module must render the same spelling the table carries: \
         table={winner:?}, module:\n{module}"
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

/// Collapsing options — `opt opt`, `opt null`, `opt reserved`, through
/// declared aliases, recursion (`type Chain = opt Chain`), mutual recursion,
/// variant arms, and an actor — box as `{ some: T } | null`, while
/// `opt empty` stays the plain `null`. The tsc equality gate proves every
/// boxed alias equals what `c.opt` infers through `OptDomain`.
#[test]
fn golden_options() {
    assert_golden("options");
}

/// Issue #188: declarations named after the module's former bindings — the
/// runtime's `c`, `Schema` and `PrincipalValue` (the principal type until
/// issue #187 made it `Principal`; see `principal_named_declaration_generates`),
/// the ambient `Array`,
/// `Record`, `Uint8Array` and `Promise`, and the reserved words `delete`,
/// `string` and `default` — generate beside the lowerings that reference
/// those ambients. The tsc equality gate compiles the golden, and
/// `ts/tests/shadowing.test.ts` loads it under Node and imports every
/// declaration through its Candid export name.
#[test]
fn golden_shadowing() {
    assert_golden("shadowing");
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
        "options",
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
        let entries: Vec<serde_json::Value> = name_triples(source_info)
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
        output.contains("hook: { principal: $.Principal; method: string }"),
        "{output}"
    );
    assert!(
        output.contains("$.c.func([$.c.nat], [$.c.text], \"update\")"),
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
        output.contains("type $Callback = { principal: $.Principal; method: string };"),
        "{output}"
    );
    assert!(output.contains("hook: $Callback"), "{output}");
    assert!(output.contains("\"query\""), "{output}");
}

/// `T | null` cannot carry Candid optionality when the inner type can itself
/// be `null` in TypeScript, so every collapsing shape boxes its present value
/// as `{ some: T } | null` — including an opt reached through a declared
/// alias, because the test is on the node, not its spelling — and nothing
/// else boxes: `opt empty` and `opt nat` keep `T | null`.
#[test]
fn collapsing_options_box() {
    for (source, alias) in [
        (
            "type DoubleOpt = opt opt nat;",
            "type $DoubleOpt = { some: bigint | null } | null;",
        ),
        (
            "type OptNull = opt null;",
            "type $OptNull = { some: null } | null;",
        ),
        (
            "type OptReserved = opt reserved;",
            "type $OptReserved = { some: unknown } | null;",
        ),
        (
            "type Inner = opt nat;\ntype Outer = opt Inner;",
            "type $Outer = { some: $Inner } | null;",
        ),
        ("type L = opt L;", "type $L = { some: $L } | null;"),
        ("type E = opt empty;", "type $E = never | null;"),
        ("type N = opt nat;", "type $N = bigint | null;"),
    ] {
        let compilation = compile_did(source).expect("source must compile");
        let output = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{source} must generate: {error}"));
        assert!(output.contains(alias), "{source}: {output}");
        // The builder is `c.opt` either way; the box is a domain shape the
        // runtime's `OptDomain` and walkers derive from the same node rule.
        assert!(output.contains("$.c.opt("), "{source}: {output}");
    }
}

/// The caller-supplied module specifier is escaped, never interpolated: a
/// hostile or accidental quote cannot produce syntactically invalid output.
/// A non-default module is imported as the bare `Principal`, which no
/// `$`-prefixed declaration local can collide with — a declaration of that
/// very name included (issues #188 and #187).
#[test]
fn principal_import_is_escaped() {
    let compilation = compile_did("type Who = principal;\ntype Principal = record { who : Who };")
        .expect("compile");
    let options = TsOptions {
        principal_import: "bad\"path".to_string(),
    };
    let output =
        generate_module(compilation.contract(), &TsNames::new(), &options).expect("generate");
    assert!(
        output.contains("import type { Principal } from \"bad\\\"path\";\n"),
        "specifier must be escaped: {output}"
    );
    assert!(output.contains("type $Who = Principal;"), "{output}");
    assert!(
        output.contains("export { $Principal as Principal };"),
        "{output}"
    );
    assert!(!output.contains("$.Principal"), "{output}");
    assert!(!output.contains("PrincipalValue"), "{output}");

    // At the default, the type comes through the runtime namespace and no
    // second import is emitted.
    let output = generate_module(
        compilation.contract(),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("generate");
    assert!(output.contains("type $Who = $.Principal;"), "{output}");
    assert!(!output.contains("import type"), "{output}");
    assert!(!output.contains("PrincipalValue"), "{output}");
}

/// Issue #187, criterion 8: the runtime's principal type is now named
/// `Principal`, and a contract declaring `type Principal = record { p :
/// principal }` still generates — the declaration binds `$Principal`, the
/// runtime type is `$.Principal`, and the two never meet. Pinned line by
/// line, so an emitter that referenced the runtime type through any bare
/// local (an unprefixed `import type { Principal }` at the default, say)
/// fails here. The module is the `shadowing` golden's `PrincipalValue`
/// declaration under another name, and that golden compiles under the tsc
/// equality gate.
#[test]
fn principal_named_declaration_generates() {
    let compilation = compile_did("type Principal = record { p : principal };").expect("compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    let output = generate_module(compilation.contract(), &names, &TsOptions::default())
        .expect("a declaration named Principal generates");
    assert_eq!(
        output,
        concat!(
            "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n",
            "import * as $ from \"@candid-core/schema\";\n",
            "\n",
            "type $Principal = { p: $.Principal };\n",
            "const $Principal: $.Schema<$Principal> = $.c.rec(() => $.c.record({ p: $.c.principal }));\n",
            "export { $Principal as Principal };\n",
        ),
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

/// Issue #188: the declaration names #116 and #130 refused — the module's
/// imports, the ambient types its lowerings reference, and (new) reserved
/// words — generate, because every declaration binds as a `$`-prefixed
/// local and leaves under its Candid name. The `shadowing` golden carries
/// them through the tsc gate and Node; this pins each former refusal
/// individually, actor or not, so no single case can regress unnoticed.
#[test]
fn former_binding_names_generate() {
    for (source, name) in [
        ("type c = nat8;", "c"),
        ("type Schema = nat8;", "Schema"),
        (
            "type PrincipalValue = record { p : principal };",
            "PrincipalValue",
        ),
        ("type PrincipalValue = nat8;", "PrincipalValue"),
        ("type Array = nat8; type V = vec text;", "Array"),
        ("type Record = nat8; type E = record {};", "Record"),
        ("type Uint8Array = text; type B = blob;", "Uint8Array"),
        ("type Array = nat8;", "Array"),
        ("type Promise = nat8;", "Promise"),
        (
            "type Promise = record { id : nat }; service : { ping : () -> () };",
            "Promise",
        ),
        ("type delete = text;", "delete"),
        ("type string = nat;", "string"),
        ("type default = bool;", "default"),
        ("type Principal = nat8;", "Principal"),
        ("type Principal = record { p : principal };", "Principal"),
    ] {
        let compilation = compile_did(source).expect("compile");
        let output = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{source} must generate since #188: {error}"));
        assert!(
            output.contains(&format!("\nexport {{ ${name} as {name} }};\n")),
            "{source}: {output}"
        );
        assert!(
            output.contains(&format!("\ntype ${name} = ")),
            "{source}: {output}"
        );
        assert!(
            output.contains(&format!("\nconst ${name}: $.Schema<${name}> = $.c.rec(")),
            "{source}: {output}"
        );
    }
}

/// The module's own export names stay reserved: the actor surface exports
/// `actor` and `Actor`, so a declaration by either name would be a
/// duplicate export. Refused unconditionally — with or without an actor —
/// by the #116 locality rule; issue #189 turns this into an omission.
#[test]
fn export_name_declarations_are_refused() {
    for source in [
        "type actor = nat8;",
        "type actor = nat8; service : { ping : () -> () };",
        "type Actor = nat8;",
        "type Actor = nat8; service : { ping : () -> () };",
    ] {
        let compilation = compile_did(source).expect("compile");
        let error = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .expect_err("a declaration named after an export must refuse generation");
        assert!(
            matches!(
                &error,
                TsGenError::ReservedDeclarationName { name } if name == "actor" || name == "Actor"
            ),
            "unexpected error for {source}: {error}"
        );
        // Message hygiene: a joined multi-line literal bakes indentation
        // into the user-facing text, which no format gate catches.
        assert!(
            !error.to_string().contains("  "),
            "error message carries embedded space runs: {error}"
        );
    }
    // The reservation is the exact names: case variants and near-misses
    // are ordinary declarations beside the actor's exports.
    for (source, name) in [
        ("type ACTOR = nat8; service : { ping : () -> () };", "ACTOR"),
        (
            "type actors = nat8; service : { ping : () -> () };",
            "actors",
        ),
        (
            "type Actor2 = nat8; service : { ping : () -> () };",
            "Actor2",
        ),
    ] {
        let compilation = compile_did(source).expect("compile");
        let output = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert!(
            output.contains(&format!("export {{ ${name} as {name} }};")),
            "{source}: {output}"
        );
        assert!(
            output.contains("export { $actor as actor, type $Actor as Actor };"),
            "{source}: {output}"
        );
    }
}

/// A Contract document admits any non-empty declaration name; Candid source
/// admits only identifiers. A name that is not identifier-shaped cannot
/// become a `$`-prefixed local, so it still fails closed — reachable only
/// through a hand-built or JSON-loaded Contract.
#[test]
fn non_identifier_declaration_names_are_refused() {
    let contract_named = |name: &str| {
        candid_core::ContractDraft::new(
            vec![candid_core::TypeNode::Primitive {
                primitive: candid_core::PrimitiveType::Nat,
            }],
            vec![candid_core::Declaration {
                name: name.to_string(),
                ty: 0,
            }],
            None,
        )
        .build()
        .unwrap_or_else(|error| panic!("{name:?}: a valid Contract: {error}"))
    };
    for name in ["has space", "na\u{ef}ve", "1abc", "a-b"] {
        let error = generate_module(
            &contract_named(name),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .expect_err("a non-identifier declaration name must refuse generation");
        assert!(
            matches!(&error, TsGenError::InvalidDeclarationName { name: refused } if refused == name),
            "unexpected error for {name:?}: {error}"
        );
    }
    // `$` is identifier-shaped, and `$` plus a `$`-bearing name is still an
    // injective, valid local.
    let output = generate_module(
        &contract_named("$ok"),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("an identifier-shaped name generates");
    assert!(output.contains("export { $$ok as $ok };"), "{output}");
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
    assert!(
        output.contains("export { $actor as actor, type $Actor as Actor };"),
        "{output}"
    );
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
        output.contains("$.c.service({ [\"__proto__\"]: $.c.func"),
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
