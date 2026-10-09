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
use candid_core_ts::{
    generate_module, GeneratedModule, Omission, OmissionKind, OmissionReason, TsNames, TsOptions,
};

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
        .module
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
        .expect("the colliding source generates")
        .module;
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
/// declared aliases, recursion through opt (`Pong = opt Ping`, `Ping = opt
/// record { pong : Pong }`), variant arms, and an actor — box as `{ some: T } | null`, while
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

/// Issue #191: a declared primitive names only itself. The fixture pins the
/// capture cases — `Memo` beside a bare `nat64`, `Byte` beside `blob`, two
/// aliases of one primitive side by side, and the ICRC-1 ledger's `Tokens` and
/// `BlockIndex` — through the tsc equality gate, and its Contract, names and
/// envelope goldens feed the loader crosscheck (`ts/tests/crosscheck.test.ts`),
/// which holds the loaded schemas to the same value domains.
#[test]
fn golden_fidelity() {
    assert_golden("fidelity");
}

/// Issue #244: each `Actor` method carries its mode — `query`,
/// `composite_query`, `update`, `oneway` — as `$.WithMode<...>` intersected
/// with its call signature, whether the method is written inline, typed by a
/// declared func, or has a quoted name. The tsc gate compiles the golden, and
/// `ts/tests/method-modes.test.ts` reads every mode back with `ModeOf` and
/// proves the call signatures are the ones emitted before the marks.
#[test]
fn golden_modes() {
    assert_golden("modes");
}

/// Issue #244: an actor with no methods generates an empty `Actor`, which
/// has no member to carry a mode — the case a mode map would have had to
/// render as an empty object.
#[test]
fn golden_methodless() {
    assert_golden("methodless");
}

/// Issue #245: each declaration's type is declared under its Candid name
/// (`export type Account = …`) and only its value binds the `$` local, so a
/// consumer's compiler errors name `Account`. A name that cannot be a type in
/// the module — an ambient type a lowering references, or a word TypeScript
/// refuses there — keeps the `$` local for its type too, wherever it is
/// referenced. The fixture declares every such word Candid source admits, and
/// every contextual keyword, which stays plain, and references each from a
/// record and, for the contextual keywords and `intrinsic`, as the start of an
/// alias's body (`type A = X`, `type A = opt X`); the tsc equality gate
/// compiles the golden, so a word missing from the fallback list fails it.
/// `ts/tests/type-names.test.ts` reads the compiler's own error messages.
#[test]
fn golden_typenames() {
    assert_golden("typenames");
}

/// Issue #191: `.did` doc comments and argument names as JSDoc — on types,
/// consts, record properties, union arms and `Actor` methods — including the
/// hostile and degenerate texts the escaping rules exist for. The golden is
/// compiled by the tsc equality gate and read back through the TypeScript
/// compiler's own doc queries by `ts/tests/jsdoc.test.ts`.
#[test]
fn golden_docs() {
    assert_golden("docs");
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
        "fidelity",
        "omissions",
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
/// property, not an aspiration. The omitted list and the header that lists
/// it are held to the same standard (issue #189).
#[test]
fn generation_is_deterministic_across_serde_round_trips() {
    for fixture in ["recursion", "omissions"] {
        let source = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(format!("{fixture}.did")),
        )
        .expect("fixture must be readable");
        let compilation = compile_did(&source).expect("fixture must compile");
        let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
        let options = TsOptions::default();

        let first = generate_module(compilation.contract(), &names, &options).expect("generate");
        let second = generate_module(compilation.contract(), &names, &options).expect("generate");
        assert_eq!(first, second, "{fixture}");

        let json = serde_json::to_string(compilation.contract()).expect("serialize");
        let reparsed = candid_core::Contract::from_json(&json).expect("reparse");
        let third = generate_module(&reparsed, &names, &options).expect("generate");
        assert_eq!(first, third, "{fixture}");
        assert_eq!(fixture == "omissions", !first.omitted.is_empty());
    }
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
        .expect("anonymous nested func generates since #104")
        .module;
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
        .expect("a reference to a func alias generates since #104")
        .module;
    assert!(
        output.contains("export type Callback = { principal: $.Principal; method: string };"),
        "{output}"
    );
    assert!(output.contains("hook: Callback"), "{output}");
    assert!(output.contains("hook: $Callback"), "the builder: {output}");
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
            "export type DoubleOpt = { some: bigint | null } | null;",
        ),
        (
            "type OptNull = opt null;",
            "export type OptNull = { some: null } | null;",
        ),
        (
            "type OptReserved = opt reserved;",
            "export type OptReserved = { some: unknown } | null;",
        ),
        (
            "type Inner = opt nat;\ntype Outer = opt Inner;",
            "export type Outer = { some: Inner } | null;",
        ),
        ("type E = opt empty;", "export type E = never | null;"),
        ("type N = opt nat;", "export type N = bigint | null;"),
    ] {
        let compilation = compile_did(source).expect("source must compile");
        let output = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{source} must generate: {error}"))
        .module;
        assert!(output.contains(alias), "{source}: {output}");
        // The builder is `c.opt` either way; the box is a domain shape the
        // runtime's `OptDomain` and walkers derive from the same node rule.
        assert!(output.contains("$.c.opt("), "{source}: {output}");
    }

    // An opt whose inner is its own node boxes too. The compiler refuses
    // `type L = opt L;` since issue #234 (a cycle through opt alone), but a
    // Contract document holding that graph still loads, so the generator
    // still meets it: the same graph, built through the model.
    let contract = candid_core::ContractDraft::new(
        vec![candid_core::TypeNode::Opt { inner: 0 }],
        vec![candid_core::Declaration {
            name: "L".to_string(),
            ty: 0,
        }],
        None,
    )
    .build()
    .expect("the model accepts a cycle through opt alone");
    let output = generate_module(&contract, &TsNames::new(), &TsOptions::default())
        .expect("a self-cycle through opt generates")
        .module;
    assert!(
        output.contains("export type L = { some: L } | null;"),
        "{output}"
    );
    assert!(output.contains("$.c.opt("), "{output}");
}

/// The caller-supplied module specifier is escaped, never interpolated: a
/// hostile or accidental quote cannot produce syntactically invalid output.
/// A non-default module is imported as the bare `Principal`, which no
/// `$`-prefixed declaration local can collide with — a declaration of that
/// very name included (issues #188 and #187): its type then keeps the `$`
/// local (issue #245), because declaring it as `Principal` would shadow the
/// import every principal lowering means. At the default the runtime type is
/// `$.Principal`, and the declaration's type is declared plain.
#[test]
fn principal_import_is_escaped() {
    let compilation = compile_did("type Who = principal;\ntype Principal = record { who : Who };")
        .expect("compile");
    let options = TsOptions {
        principal_import: "bad\"path".to_string(),
    };
    let output = generate_module(compilation.contract(), &TsNames::new(), &options)
        .expect("generate")
        .module;
    assert!(
        output.contains("import type { Principal } from \"bad\\\"path\";\n"),
        "specifier must be escaped: {output}"
    );
    assert!(output.contains("export type Who = Principal;"), "{output}");
    assert!(
        output.contains("\ntype $Principal = { _5941054_: Principal };\n"),
        "{output}"
    );
    assert!(
        output.contains("const $Principal: $.Schema<$Principal> = "),
        "{output}"
    );
    assert!(
        output.contains("export { $Principal as Principal };"),
        "{output}"
    );
    assert!(!output.contains("export type Principal"), "{output}");
    assert!(!output.contains("$.Principal"), "{output}");
    assert!(!output.contains("PrincipalValue"), "{output}");

    // At the default, the type comes through the runtime namespace and no
    // second import is emitted.
    let output = generate_module(
        compilation.contract(),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("generate")
    .module;
    assert!(
        output.contains("export type Who = $.Principal;"),
        "{output}"
    );
    assert!(
        output.contains("export type Principal = { _5941054_: $.Principal };\n"),
        "{output}"
    );
    assert!(
        output.contains("export { $Principal as Principal };"),
        "{output}"
    );
    assert!(!output.contains("import type"), "{output}");
    assert!(!output.contains("PrincipalValue"), "{output}");
}

/// Issue #187, criterion 8: the runtime's principal type is now named
/// `Principal`, and a contract declaring `type Principal = record { p :
/// principal }` still generates — the declaration's value binds
/// `$Principal` and its type is declared as `Principal` (issue #245), the
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
        .expect("a declaration named Principal generates")
        .module;
    assert_eq!(
        output,
        concat!(
            "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n",
            "import * as $ from \"@candid-core/schema\";\n",
            "\n",
            "export type Principal = { p: $.Principal };\n",
            "const $Principal: $.Schema<Principal> = $.c.rec(() => $.c.record({ p: $.c.principal }));\n",
            "export { $Principal as Principal };\n",
        ),
    );
}

/// A source name shaped like the `_N_` id rendering cannot become a schema
/// key (issue #115): erased to one it is indistinguishable from numeric label
/// id N, so the codec would encode wire id N instead of the name's hash —
/// the silent-wrong-id fail-open the reservation exists to prevent. Since
/// issue #189 the declaration holding it is omitted, with every declaration
/// and actor method that references it, and everything unrelated generates.
/// `schemaFromContract` omits the same (`ts/tests/contract.test.ts`).
#[test]
fn numeric_shaped_source_names_are_omitted() {
    for (declaration, bad) in [
        ("type Holder = record { _123_ : nat8 };", "Holder"),
        ("type Event = variant { _123_ : nat8; idle };", "Event"),
        // The original #115 collision repro: omitted before any duplicate
        // key could render.
        ("type T = record { _123_ : nat8; 123 : text };", "T"),
    ] {
        let source = format!(
            "{declaration}\ntype Uses = record {{ inner : {bad} }};\n\
             type Fine = record {{ a : nat }};\n\
             service : {{ keep : (Fine) -> (); drop : ({bad}) -> () }}"
        );
        let generated = generate_source_full(&source);
        assert_eq!(
            generated.omitted,
            vec![
                declaration_omitted(bad, OmissionReason::ReservedFieldName),
                declaration_omitted_via("Uses", bad),
                method_omitted_via("drop", bad),
            ],
            "{source}"
        );
        let module = &generated.module;
        assert!(
            !module.contains("_123_"),
            "the reserved name renders nowhere: {module}"
        );
        assert!(!references_declaration(module, bad), "{module}");
        assert!(!references_declaration(module, "Uses"), "{module}");
        assert!(module.contains("export { $Fine as Fine };"), "{module}");
        assert!(
            module.contains("keep: ((arg0: Fine) => Promise<void>) & $.WithMode<\"update\">;"),
            "{module}"
        );
        assert!(
            !without_omitted_lines(module).contains("drop"),
            "only the header names the omitted method: {module}"
        );
    }
    // Non-canonical shapes are ordinary names: a leading zero never renders
    // from a numeric id, so `_007_` stays hash-addressed and unambiguous.
    let generated = generate_source_full("type Ok = record { _007_ : nat8 };");
    assert!(generated.omitted.is_empty(), "{:?}", generated.omitted);
    assert!(
        generated.module.contains("_007_"),
        "the name must render: {}",
        generated.module
    );
}

/// Issue #188: the declaration names #116 and #130 refused — the module's
/// imports, the ambient types its lowerings reference, and (new) reserved
/// words — generate, because every declaration's value binds as a
/// `$`-prefixed local and leaves under its Candid name. Since issue #245 the
/// type is declared under the Candid name itself (`export type c = …`)
/// unless that name cannot be a type in the module: an ambient type a
/// lowering references, or a word TypeScript refuses there, keeps the `$`
/// local for its type too (`type $Array = …`). The `shadowing` golden
/// carries them through the tsc gate and Node, and the `typenames` golden
/// every refused word; this pins each former refusal individually, actor or
/// not, so no single case can regress unnoticed.
#[test]
fn former_binding_names_generate() {
    for (source, name, plain) in [
        ("type c = nat8;", "c", true),
        ("type Schema = nat8;", "Schema", true),
        (
            "type PrincipalValue = record { p : principal };",
            "PrincipalValue",
            true,
        ),
        ("type PrincipalValue = nat8;", "PrincipalValue", true),
        ("type Array = nat8; type V = vec text;", "Array", false),
        ("type Record = nat8; type E = record {};", "Record", false),
        (
            "type Uint8Array = text; type B = blob;",
            "Uint8Array",
            false,
        ),
        ("type Array = nat8;", "Array", false),
        ("type Promise = nat8;", "Promise", false),
        (
            "type Promise = record { id : nat }; service : { ping : () -> () };",
            "Promise",
            false,
        ),
        ("type delete = text;", "delete", false),
        ("type string = nat;", "string", false),
        ("type default = bool;", "default", false),
        ("type Principal = nat8;", "Principal", true),
        (
            "type Principal = record { p : principal };",
            "Principal",
            true,
        ),
    ] {
        let compilation = compile_did(source).expect("compile");
        let output = generate_module(
            compilation.contract(),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{source} must generate since #188: {error}"))
        .module;
        assert!(
            output.contains(&format!("\nexport {{ ${name} as {name} }};\n")),
            "{source}: {output}"
        );
        let ty = if plain {
            name.to_string()
        } else {
            format!("${name}")
        };
        let head = if plain { "export type" } else { "type" };
        assert!(
            output.contains(&format!("\n{head} {ty} = ")),
            "{source}: {output}"
        );
        assert!(
            output.contains(&format!("\nconst ${name}: $.Schema<{ty}> = $.c.rec(")),
            "{source}: {output}"
        );
        // Exactly one of the two layouts.
        let other = if plain {
            format!("\ntype ${name} = ")
        } else {
            format!("export type {name} = ")
        };
        assert!(!output.contains(&other), "{source}: {output}");
    }
}

/// Issue #245: the names only a Contract document can declare — a reserved
/// word Candid source spells as a token (`null`, `true`, `false`, `import`),
/// and a name containing `$` — keep the `$` local for their type, and a type
/// that references one names that local. A fallback type name always starts
/// with `$` and a plain one never does, so no plain name can collide with a
/// fallback (`$x`'s type is `$$x`, not `$x`, which is `x`'s value).
#[test]
fn names_only_a_document_declares_fall_back() {
    for (name, ty) in [
        ("null", "$null"),
        ("true", "$true"),
        ("false", "$false"),
        ("import", "$import"),
        ("$", "$$"),
        ("$x", "$$x"),
        ("x$", "x$"),
        ("type", "type"),
    ] {
        let contract = candid_core::ContractDraft::new(
            vec![
                candid_core::TypeNode::Record {
                    fields: vec![candid_core::Field { id: 5, ty: 2 }],
                },
                candid_core::TypeNode::Record {
                    fields: vec![candid_core::Field { id: 7, ty: 0 }],
                },
                candid_core::TypeNode::Primitive {
                    primitive: candid_core::PrimitiveType::Nat8,
                },
            ],
            vec![
                candid_core::Declaration {
                    name: name.to_string(),
                    ty: 0,
                },
                candid_core::Declaration {
                    name: "Holder".to_string(),
                    ty: 1,
                },
            ],
            None,
        )
        .build()
        .expect("the model accepts the contract");
        let output = generate_module(&contract, &TsNames::new(), &TsOptions::default())
            .unwrap_or_else(|error| panic!("{name} must generate: {error}"))
            .module;
        let head = if ty == name { "export type" } else { "type" };
        assert!(
            output.contains(&format!("\n{head} {ty} = {{ _5_: number }};\n")),
            "{name}: {output}"
        );
        assert!(
            output.contains(&format!("\nconst ${name}: $.Schema<{ty}> = ")),
            "{name}: {output}"
        );
        assert!(
            output.contains(&format!("\nexport {{ ${name} as {name} }};\n")),
            "{name}: {output}"
        );
        assert!(
            output.contains(&format!("\nexport type Holder = {{ _7_: {ty} }};\n")),
            "{name}: {output}"
        );
        assert!(
            output.contains(&format!("$.c.record({{ _7_: ${name} }})")),
            "{name}: {output}"
        );
    }
}

/// The module's own export names stay reserved: the actor surface exports
/// `actor` and `Actor`, so a declaration by either name would be a
/// duplicate export. Since issue #189 such a declaration is omitted
/// (`reserved_export_name`) — with or without an actor, by the #116 locality
/// rule — and the rest of the module generates.
#[test]
fn export_name_declarations_are_omitted() {
    let header = "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n";
    let actor_surface = concat!(
        "import * as $ from \"@candid-core/schema\";\n",
        "\n",
        "const $actor: $.Schema<$.Principal> = $.c.rec(() => $.c.service({ ping: $.c.func([], [], \"update\") }));\n",
        "export type Actor = {\n",
        "  ping: (() => Promise<void>) & $.WithMode<\"update\">;\n",
        "};\n",
        "export { $actor as actor };\n",
    );
    for (source, name, rest) in [
        ("type actor = nat8;", "actor", ""),
        (
            "type actor = nat8; service : { ping : () -> () };",
            "actor",
            actor_surface,
        ),
        ("type Actor = nat8;", "Actor", ""),
        (
            "type Actor = nat8; service : { ping : () -> () };",
            "Actor",
            actor_surface,
        ),
    ] {
        let generated = generate_source_full(source);
        assert_eq!(
            generated.omitted,
            vec![declaration_omitted(
                name,
                OmissionReason::ReservedExportName
            )],
            "{source}"
        );
        assert_eq!(
            generated.module,
            format!("{header}// Omitted: type {name} (reserved_export_name)\n{rest}"),
            "{source}"
        );
    }

    // Issue #189, criterion 4: the methods and declarations that reference
    // the omitted declaration go with it, from both actor surfaces; the rest
    // of the actor stays.
    let generated = generate_source_full(
        "type actor = record { id : nat };\ntype Keep = record { k : nat };\n\
         type Uses = vec actor;\n\
         service : { use_it : (actor) -> (); keep : (Keep) -> (Keep) query }",
    );
    assert_eq!(
        generated.omitted,
        vec![
            declaration_omitted_via("Uses", "actor"),
            declaration_omitted("actor", OmissionReason::ReservedExportName),
            method_omitted_via("use_it", "actor"),
        ]
    );
    let module = &generated.module;
    assert!(
        !without_omitted_lines(module).contains("use_it"),
        "only the header names the omitted method: {module}"
    );
    assert!(
        module.contains("$.c.service({ keep: $.c.func([$Keep], [$Keep], \"query\") })"),
        "{module}"
    );
    assert!(
        module.contains("  keep: ((arg0: Keep) => Promise<Keep>) & $.WithMode<\"query\">;\n"),
        "{module}"
    );
    assert_eq!(
        module.matches("export { $actor as actor };").count(),
        1,
        "{module}"
    );

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
        let generated = generate_source_full(source);
        assert!(generated.omitted.is_empty(), "{source}");
        assert!(
            generated
                .module
                .contains(&format!("export {{ ${name} as {name} }};")),
            "{source}: {}",
            generated.module
        );
        assert!(
            generated.module.contains("export { $actor as actor };"),
            "{source}: {}",
            generated.module
        );
    }
}

/// A Contract document admits any non-empty declaration name; Candid source
/// admits only identifiers. A name that is not identifier-shaped cannot
/// become a `$`-prefixed local, so the declaration is omitted
/// (`invalid_declaration_name`) — reachable only through a hand-built or
/// JSON-loaded Contract. The header quotes such a name, and nothing in it can
/// end the line comment it is listed on.
#[test]
fn non_identifier_declaration_names_are_omitted() {
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
    let header = "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n";
    for (name, listed) in [
        ("has space", "\"has space\""),
        ("na\u{ef}ve", "\"na\u{ef}ve\""),
        ("1abc", "\"1abc\""),
        ("a-b", "\"a-b\""),
        ("line\nbreak", "\"line\\nbreak\""),
        ("para\u{2029}graph", "\"para\\u2029graph\""),
        ("line\u{2028}separator", "\"line\\u2028separator\""),
        ("quote\"d */", "\"quote\\\"d */\""),
    ] {
        let generated = generate_module(
            &contract_named(name),
            &TsNames::new(),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{name:?}: {error}"));
        assert_eq!(
            generated.omitted,
            vec![declaration_omitted(
                name,
                OmissionReason::InvalidDeclarationName
            )],
        );
        assert_eq!(
            generated.module,
            format!("{header}// Omitted: type {listed} (invalid_declaration_name)\n"),
            "{name:?}"
        );
    }

    // A declaration that renders the omitted one's name goes with it, and
    // the header quotes the `via` name the same way.
    let contract = candid_core::ContractDraft::new(
        vec![
            candid_core::TypeNode::Record {
                fields: vec![candid_core::Field { id: 1, ty: 1 }],
            },
            candid_core::TypeNode::Record {
                fields: vec![candid_core::Field { id: 2, ty: 2 }],
            },
            candid_core::TypeNode::Primitive {
                primitive: candid_core::PrimitiveType::Nat,
            },
        ],
        vec![
            candid_core::Declaration {
                name: "a-b".to_string(),
                ty: 1,
            },
            candid_core::Declaration {
                name: "Uses".to_string(),
                ty: 0,
            },
        ],
        None,
    )
    .build()
    .expect("a valid Contract");
    let generated = generate_module(&contract, &TsNames::new(), &TsOptions::default())
        .expect("an invalid name is an omission, not a refusal");
    assert_eq!(
        generated.omitted,
        vec![
            declaration_omitted_via("Uses", "a-b"),
            declaration_omitted("a-b", OmissionReason::InvalidDeclarationName),
        ]
    );
    assert!(
        generated
            .module
            .contains("// Omitted: type Uses (references_omitted via \"a-b\")\n"),
        "{}",
        generated.module
    );

    // `$` is identifier-shaped, and `$` plus a `$`-bearing name is still an
    // injective, valid local.
    let generated = generate_module(
        &contract_named("$ok"),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("an identifier-shaped name generates");
    assert!(generated.omitted.is_empty());
    assert!(
        generated.module.contains("export { $$ok as $ok };"),
        "{}",
        generated.module
    );
}

/// Issue #127: a variant arm whose payload is a *declared* `opt` of a
/// never-domain type renders as a bare reference statically identical to a
/// declared alias of `null` — the one shape no type-level classification
/// can carry. Since issue #189 the variant is omitted
/// (`ambiguous_variant_arm`) instead of refusing the module; the `opt`
/// declaration itself generates, and the anonymous form and the null alias
/// stay generable (the `arms` golden pins them).
#[test]
fn declared_opt_empty_variant_arms_are_omitted() {
    for (source, kept) in [
        (
            "type W = opt empty; type V = variant { a : W; b : nat };",
            &["W"][..],
        ),
        // The inner may be a declared alias of empty: same arena node.
        (
            "type E = empty; type W = opt E; type V = variant { a : W };",
            &["E", "W"][..],
        ),
        // An empty variant is never-domain too.
        (
            "type Never = variant {}; type W = opt Never; type V = variant { a : W };",
            &["Never", "W"][..],
        ),
        // Dedup alone declares the arm: an anonymous arm and a same-shape
        // declaration share one node, so the arm renders as the name.
        (
            "type V = variant { a : opt empty }; type W = opt empty;",
            &["W"][..],
        ),
    ] {
        let generated = generate_source_full(source);
        assert_eq!(
            generated.omitted,
            vec![declaration_omitted(
                "V",
                OmissionReason::AmbiguousVariantArm
            )],
            "{source}"
        );
        assert!(!references_declaration(&generated.module, "V"), "{source}");
        for name in kept {
            assert!(
                generated
                    .module
                    .contains(&format!("export {{ ${name} as {name} }};")),
                "{source}: {}",
                generated.module
            );
        }
    }
    // Near-misses stay generable: an opt of an inhabited type through a
    // declaration is an ordinary valued arm — the inhabited *variant* case
    // pins the `fields.is_empty()` discrimination in the cause itself…
    for source in [
        "type W = opt nat; type V = variant { a : W };",
        "type S = variant { x }; type W = opt S; type V = variant { a : W };",
        // …and an opt of an uninhabited *record* keeps `value` on its own:
        // the reference's static type is `{ f: never } | null`, not `null`,
        // so the type level classifies it without help.
        "type W = opt record { f : empty }; type V = variant { a : W };",
    ] {
        let generated = generate_source_full(source);
        assert!(
            generated.omitted.is_empty(),
            "{source}: {:?}",
            generated.omitted
        );
        assert!(generated.module.contains("export { $V as V };"), "{source}");
    }
}

/// Issue #189: the `omissions` fixture — every direct cause a Candid source
/// can reach, and the closure through every kind of edge (a field, an
/// alias, anonymous `vec`/`opt`/record nesting, a nested `service` type in
/// value position, a `func` declaration, recursion) — generates a module the
/// tsc equality gate compiles, and its omitted list is a golden too: the
/// loader crosscheck (`ts/tests/crosscheck.test.ts`) and the wasm parity
/// tests hold theirs equal to it.
#[test]
fn golden_omissions() {
    let generated = generate_fixture_full("omissions");
    assert_golden_file("omissions.ts", &generated.module);
    let mut text = serde_json::to_string_pretty(&omitted_json(&generated.omitted))
        .expect("the omitted list serializes");
    text.push('\n');
    assert_golden_file("omissions.omitted.json", &text);
}

/// The closure proof: nothing the module emits references an omitted
/// declaration — neither its `$` local nor its type name appears in code —
/// and each omitted method is gone from *both* actor surfaces, the `actor`
/// schema and the `Actor` type, which are rendered by separate paths. The
/// listed order is declarations, then methods, each by name, and the header
/// lists exactly that order.
#[test]
fn omissions_leave_no_reference_behind() {
    let generated = generate_fixture_full("omissions");
    let module = &generated.module;

    let mut sorted = generated.omitted.clone();
    sorted.sort_by(|left, right| (left.kind, &left.name).cmp(&(right.kind, &right.name)));
    assert_eq!(
        generated.omitted, sorted,
        "declarations, then methods, by name"
    );
    let header: Vec<&str> = module
        .lines()
        .filter(|line| line.starts_with("// Omitted: "))
        .collect();
    let listed: Vec<String> = generated
        .omitted
        .iter()
        .map(|omission| format!("// Omitted: {omission}"))
        .collect();
    assert_eq!(header, listed);
    assert_eq!(
        module
            .lines()
            .skip(1)
            .take(listed.len())
            .collect::<Vec<_>>(),
        header,
        "the list sits directly under the first header line"
    );

    let omitted_declarations: Vec<&str> = generated
        .omitted
        .iter()
        .filter(|omission| omission.kind == OmissionKind::Declaration)
        .map(|omission| omission.name.as_str())
        .collect();
    for omission in &generated.omitted {
        if let Some(via) = &omission.via {
            assert_eq!(omission.reason, OmissionReason::ReferencesOmitted);
            assert!(
                omitted_declarations.contains(&via.as_str()),
                "{omission}: `via` must name an omitted declaration"
            );
        } else {
            assert_ne!(omission.reason, OmissionReason::ReferencesOmitted);
        }
    }
    // The actor surface binds `$actor` and declares `Actor` itself; with its
    // three lines set aside, not even an omitted `actor` or `Actor` is
    // referenced.
    let declarations_only: String = module
        .split_inclusive('\n')
        .filter(|line| {
            !line.starts_with("const $actor: ")
                && !line.starts_with("export type Actor = {")
                && !line.starts_with("export { $actor as actor };")
        })
        .collect();
    for name in &omitted_declarations {
        assert!(
            !references_declaration(&declarations_only, name),
            "the module still references omitted `{name}`:\n{module}"
        );
    }

    let schema = module
        .lines()
        .find(|line| line.starts_with("const $actor: "))
        .expect("the actor schema");
    let interface = module
        .split("export type Actor = {\n")
        .nth(1)
        .and_then(|rest| rest.split("\n};").next())
        .expect("the Actor type");
    let kept = ["ok", "list", "directory"];
    assert_eq!(
        schema.matches(": $.c.func(").count(),
        kept.len(),
        "{schema}"
    );
    assert_eq!(interface.lines().count(), kept.len(), "{interface}");
    for method in kept {
        assert!(
            schema.contains(&format!(" {method}: $.c.func(")),
            "{schema}"
        );
        assert!(interface.contains(&format!("  {method}: (")), "{interface}");
    }
    for omission in &generated.omitted {
        if omission.kind == OmissionKind::Method {
            let name = &omission.name;
            assert!(!schema.contains(&format!(" {name}: ")), "{schema}");
            assert!(!interface.contains(&format!("  {name}: ")), "{interface}");
        }
    }
}

/// The wire-type proof: omission changes nothing it keeps. The module
/// generated from the `omissions` fixture, minus its `// Omitted:` lines, is
/// byte-identical to the module generated from the same source with every
/// omitted declaration and method deleted by hand — so every emitted alias,
/// builder, `Actor` signature and actor-schema method is exactly what it
/// would be had the omitted declarations never existed, and the wire type of
/// every emitted method is unchanged.
#[test]
fn omission_leaves_everything_it_keeps_unchanged() {
    let control = generate_source_full(OMISSIONS_CONTROL);
    assert!(control.omitted.is_empty(), "{:?}", control.omitted);
    let generated = generate_fixture_full("omissions");
    assert!(!generated.omitted.is_empty());
    assert_eq!(without_omitted_lines(&generated.module), control.module);
}

/// `omissions.did` with every omitted declaration and method removed.
const OMISSIONS_CONTROL: &str = "
type Good = record { a : nat };
type NoValue = opt empty;
type List = opt record { head : nat; tail : List };
type Directory = record { svc : service { g : (Good) -> () } };
service : {
  ok : (Good) -> (Good) query;
  list : () -> (List) query;
  directory : (Directory) -> ();
}
";

/// An actor written as a declared service (`service : S`) whose declaration
/// is omitted keeps its surviving methods: the actor renders the service
/// inline without the omitted ones — sound only for the actor, whose own
/// service type no call ever encodes — and is byte-identical to an actor
/// declared without them.
#[test]
fn an_omitted_actor_service_declaration_keeps_the_actor() {
    let generated = generate_source_full(
        "type Good = record { a : nat };\ntype Bad = record { _0_ : nat };\n\
         type S = service { ok : (Good) -> (); bad : (Bad) -> () };\nservice : S",
    );
    assert_eq!(
        generated.omitted,
        vec![
            declaration_omitted("Bad", OmissionReason::ReservedFieldName),
            declaration_omitted_via("S", "Bad"),
            method_omitted_via("bad", "Bad"),
        ]
    );
    let control =
        generate_source_full("type Good = record { a : nat };\nservice : { ok : (Good) -> () }");
    assert_eq!(without_omitted_lines(&generated.module), control.module);

    // A declared service with nothing omitted still renders by its name.
    let kept = generate_source_full(
        "type Good = record { a : nat };\ntype S = service { ok : (Good) -> () };\nservice : S",
    );
    assert!(kept.omitted.is_empty());
    assert!(
        kept.module
            .contains("const $actor: $.Schema<$.Principal> = $.c.rec(() => $S);"),
        "{}",
        kept.module
    );
}

/// A class actor's init args are install-time metadata the module never
/// renders, so an omitted declaration they reference drops nothing from the
/// actor: the actor surface itself is never omitted.
#[test]
fn a_class_actor_keeps_its_methods_when_its_init_args_are_omitted() {
    let generated = generate_source_full(
        "type Bad = record { _0_ : nat };\nservice : (Bad) -> { ping : () -> () }",
    );
    assert_eq!(
        generated.omitted,
        vec![declaration_omitted(
            "Bad",
            OmissionReason::ReservedFieldName
        )]
    );
    assert!(
        generated
            .module
            .contains("  ping: (() => Promise<void>) & $.WithMode<\"update\">;\n"),
        "{}",
        generated.module
    );
    assert!(
        generated.module.contains("init args are install-time"),
        "{}",
        generated.module
    );
}

/// A later declaration of the same composite node renders the *first*
/// declaration's name (the arena de-duplicates structure), so when that first
/// declaration is omitted for its name, the alias goes with it — the rule the
/// emitter's rendering implies, pinned so it is a decision, not an accident.
#[test]
fn an_alias_of_an_omitted_first_declaration_is_omitted_through_it() {
    let generated =
        generate_source_full("type Actor = record { a : nat };\ntype Z = record { a : nat };");
    assert_eq!(
        generated.omitted,
        vec![
            declaration_omitted("Actor", OmissionReason::ReservedExportName),
            declaration_omitted_via("Z", "Actor"),
        ]
    );
}

/// A module with nothing to omit is byte-identical to what the generator
/// emitted before issue #189: every other golden still matches its reviewed
/// text (the `golden_*` tests), and none of them omits anything or carries an
/// `// Omitted:` line.
#[test]
fn modules_without_omissions_are_unchanged() {
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
        "shadowing",
        "fidelity",
        "docs",
    ] {
        let generated = generate_fixture_full(name);
        assert!(
            generated.omitted.is_empty(),
            "{name}: {:?}",
            generated.omitted
        );
        assert!(!generated.module.contains("// Omitted"), "{name}");
    }
}

/// The reason codes are the serialized contract (`@candid-core/cli`'s
/// `ModuleSuccess.omitted`, the loader's `omitted`): snake_case, distinct,
/// and closed — the exhaustive match below stops compiling when a reason is
/// added without a code.
#[test]
fn omission_codes_are_stable() {
    let reasons = [
        OmissionReason::ReservedFieldName,
        OmissionReason::AmbiguousVariantArm,
        OmissionReason::ReservedExportName,
        OmissionReason::InvalidDeclarationName,
        OmissionReason::ReferencesOmitted,
    ];
    let codes: Vec<&str> = reasons
        .iter()
        .map(|reason| match reason {
            OmissionReason::ReservedFieldName
            | OmissionReason::AmbiguousVariantArm
            | OmissionReason::ReservedExportName
            | OmissionReason::InvalidDeclarationName
            | OmissionReason::ReferencesOmitted => reason.code(),
        })
        .collect();
    assert_eq!(
        codes,
        [
            "reserved_field_name",
            "ambiguous_variant_arm",
            "reserved_export_name",
            "invalid_declaration_name",
            "references_omitted",
        ]
    );
    assert_eq!(OmissionKind::Declaration.code(), "declaration");
    assert_eq!(OmissionKind::Method.code(), "method");
}

/// Two spellings with one Candid hash can address one `(container, id)` in a
/// name table: `_0_` and `` 6,/`U`` both hash to 4735054. `TsNames` keeps
/// the last one inserted, and only that winner is classified: a reserved
/// `_N_` spelling that loses to a later ordinary one renders the ordinary
/// key, and one that wins omits the declaration. `ts/tests/contract.test.ts`
/// pins the same two outcomes for `schemaFromContract` over the same table
/// in both orders (PR #210 review).
#[test]
fn the_last_name_for_a_key_wins_before_it_is_classified() {
    let reserved = "_0_";
    let collision = " 6,/`U";
    let id = candid_parser_id(reserved);
    assert_eq!(id, candid_parser_id(collision), "the two spellings collide");
    assert_eq!(id, 4_735_054);
    let contract = candid_core::ContractDraft::new(
        vec![
            candid_core::TypeNode::Record {
                fields: vec![candid_core::Field { id, ty: 1 }],
            },
            candid_core::TypeNode::Primitive {
                primitive: candid_core::PrimitiveType::Nat,
            },
        ],
        vec![candid_core::Declaration {
            name: "A".to_string(),
            ty: 0,
        }],
        None,
    )
    .build()
    .expect("a valid Contract");
    let record = contract
        .types()
        .iter()
        .position(|node| matches!(node, candid_core::TypeNode::Record { .. }))
        .expect("the record node") as u32;

    // Reserved first, ordinary last: the ordinary spelling wins and renders.
    let names = TsNames::from_pairs([(record, id, reserved), (record, id, collision)]);
    let generated = generate_module(&contract, &names, &TsOptions::default()).expect("generates");
    assert!(generated.omitted.is_empty(), "{:?}", generated.omitted);
    assert!(
        generated
            .module
            .contains("$.c.record({ \" 6,/`U\": $.c.nat })"),
        "{}",
        generated.module
    );

    // Ordinary first, reserved last: the reserved spelling wins and omits.
    let names = TsNames::from_pairs([(record, id, collision), (record, id, reserved)]);
    let generated = generate_module(&contract, &names, &TsOptions::default()).expect("generates");
    assert_eq!(
        generated.omitted,
        vec![declaration_omitted("A", OmissionReason::ReservedFieldName)]
    );
}

/// A quoted method name may hold any character, including the four that end
/// an ECMAScript `//` comment (LF, CR, U+2028, U+2029) and other controls.
/// Its header line is still one line: JSON-style escapes, plus `\u2028` and
/// `\u2029`, which JSON leaves raw. The CLI's `warning: omitted` line is
/// held to the same text by `npm/test/cli.test.js` (PR #210 review).
#[test]
fn an_omitted_method_with_line_terminators_in_its_name_stays_on_one_line() {
    let generated = generate_source_full(
        "type Bad = record { _0_ : nat };\n\
         service : { \"a\\u{2028}b\\u{2029}c\\nd\\re\\u{8}f\" : (Bad) -> (); ok : () -> () }",
    );
    assert_eq!(
        generated.omitted,
        vec![
            declaration_omitted("Bad", OmissionReason::ReservedFieldName),
            method_omitted_via("a\u{2028}b\u{2029}c\nd\re\u{8}f", "Bad"),
        ]
    );
    let header =
        "// Omitted: method \"a\\u2028b\\u2029c\\nd\\re\\u0008f\" (references_omitted via Bad)\n";
    assert!(generated.module.contains(header), "{:?}", generated.module);
    for terminator in ['\r', '\u{2028}', '\u{2029}'] {
        assert!(
            !generated.module.contains(terminator),
            "{:?}",
            generated.module
        );
    }
    assert_eq!(
        generated
            .module
            .lines()
            .filter(|line| line.starts_with("// Omitted: "))
            .count(),
        2
    );
}

/// Generate a fixture through the real provenance bridge, omissions included.
fn generate_fixture_full(name: &str) -> GeneratedModule {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let source = std::fs::read_to_string(root.join("fixtures").join(format!("{name}.did")))
        .expect("fixture must be readable");
    generate_source_full(&source)
}

/// Generate from Candid text through the real provenance bridge, omissions
/// included.
fn generate_source_full(source: &str) -> GeneratedModule {
    let compilation = compile_did(source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    generate_module(compilation.contract(), &names, &TsOptions::default())
        .unwrap_or_else(|error| panic!("{source} must generate: {error}"))
}

fn declaration_omitted(name: &str, reason: OmissionReason) -> Omission {
    Omission {
        kind: OmissionKind::Declaration,
        name: name.to_string(),
        reason,
        via: None,
    }
}

fn declaration_omitted_via(name: &str, via: &str) -> Omission {
    Omission {
        kind: OmissionKind::Declaration,
        name: name.to_string(),
        reason: OmissionReason::ReferencesOmitted,
        via: Some(via.to_string()),
    }
}

fn method_omitted_via(name: &str, via: &str) -> Omission {
    Omission {
        kind: OmissionKind::Method,
        name: name.to_string(),
        reason: OmissionReason::ReferencesOmitted,
        via: Some(via.to_string()),
    }
}

/// Whether the module's code — its comment lines set aside — references
/// declaration `name`: by its `$` local (`$name`: the value, and the type
/// where the name falls back), or by its plain type name (`name` as a whole
/// identifier that is neither a property key, a member of `$`, nor inside a
/// string). Since issue #245 a type references a declaration by its plain
/// name, so checking the `$` local alone would miss every type reference.
fn references_declaration(module: &str, name: &str) -> bool {
    let code: Vec<&str> = module
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !(line.starts_with("//") || line.starts_with("/**") || line.starts_with('*'))
        })
        .collect();
    let code = code.join("\n");
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    code.match_indices(name).any(|(at, _)| {
        let mut before = code[..at].chars().rev();
        let after = code[at + name.len()..].chars().next();
        if after.is_some_and(identifier) {
            return false;
        }
        match before.next() {
            // `$name`, unless that `$` itself continues an identifier.
            Some('$') => !before.next().is_some_and(identifier),
            Some(c) if identifier(c) || c == '.' || c == '"' => false,
            _ => !matches!(after, Some(':' | '"' | '?')),
        }
    })
}

/// The module text without its `// Omitted:` header lines.
fn without_omitted_lines(module: &str) -> String {
    module
        .split_inclusive('\n')
        .filter(|line| !line.starts_with("// Omitted: "))
        .collect()
}

/// The omitted list in its serialized form — the shape `@candid-core/cli`'s
/// `didToModule` returns and `schemaFromContract` reports.
fn omitted_json(omitted: &[Omission]) -> serde_json::Value {
    serde_json::Value::Array(
        omitted
            .iter()
            .map(|omission| {
                let mut entry = serde_json::json!({
                    "kind": omission.kind.code(),
                    "name": omission.name,
                    "reason": omission.reason.code(),
                });
                if let Some(via) = &omission.via {
                    entry["via"] = serde_json::json!(via);
                }
                entry
            })
            .collect(),
    )
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
        .expect("a class actor generates its running service")
        .module;
    assert!(output.contains("export { $actor as actor };"), "{output}");
    assert!(
        output.contains("ping: (() => Promise<void>) & $.WithMode<\"update\">;"),
        "{output}"
    );
    assert!(
        output.contains("init args are install-time"),
        "the class note must be present: {output}"
    );

    let proto = compile_did("service : { \"__proto__\" : () -> () };")
        .expect("a __proto__ method compiles");
    let names = TsNames::from_source_info(proto.source_info().expect("provenance"));
    let output = generate_module(proto.contract(), &names, &TsOptions::default())
        .expect("generate")
        .module;
    assert!(
        output.contains("$.c.service({ [\"__proto__\"]: $.c.func"),
        "the method key must be computed: {output}"
    );
}

/// Generate a module from Candid text through the real provenance bridge.
fn generate_source(source: &str) -> String {
    let compilation = compile_did(source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    generate_module(compilation.contract(), &names, &TsOptions::default())
        .unwrap_or_else(|error| panic!("{source} must generate: {error}"))
        .module
}

/// Issue #191: a declaration of a primitive names only itself. The arena
/// de-duplicates every use of a primitive into one node, so a name recorded
/// for it used to be rendered at every use — `type Memo = nat64` renamed an
/// unrelated `nat64` field, `type Tokens = nat` beside `type BlockIndex = nat`
/// rendered both as whichever came first, and `type Byte = nat8` turned each
/// `blob` in the interface into `Array<Byte>`. The three measured repros from
/// the issue, each with its declaration still exported.
#[test]
fn a_declared_primitive_names_only_itself() {
    let output = generate_source("type Memo = nat64; type R = record { a : nat64; b : Memo };");
    assert!(
        output.contains("export type R = { a: bigint; b: bigint };"),
        "{output}"
    );
    assert!(
        output.contains("$.c.record({ a: $.c.nat64, b: $.c.nat64 })"),
        "{output}"
    );
    assert!(output.contains("export type Memo = bigint;"), "{output}");
    assert!(output.contains("export { $Memo as Memo };"), "{output}");

    let output =
        generate_source("type Byte = nat8; type R = record { raw : blob; bytes : vec Byte };");
    assert!(output.contains("raw: Uint8Array"), "{output}");
    assert!(output.contains("bytes: Uint8Array"), "{output}");
    assert_eq!(output.matches("$.c.blob()").count(), 2, "{output}");
    assert!(!output.contains("Array<"), "{output}");
    assert!(
        !output.contains("$Byte;") && !output.contains("$Byte,"),
        "{output}"
    );
    assert!(output.contains("export type Byte = number;"), "{output}");

    // The ICRC-1 ledger shape, verbatim from the issue.
    let output = generate_source(
        "type Tokens = nat;\n\
         type BlockIndex = nat;\n\
         type Account = record { owner : principal; subaccount : opt blob };\n\
         type TransferArg = record { to : Account; amount : Tokens; fee : opt Tokens };\n\
         type TransferResult = variant { Ok : BlockIndex; Err : text };\n\
         service : { icrc1_transfer : (TransferArg) -> (TransferResult); \
         icrc1_balance_of : (Account) -> (Tokens) query };",
    );
    assert!(
        output.contains(
            "export type TransferArg = { to: Account; fee: bigint | null; amount: bigint };"
        ),
        "{output}"
    );
    assert!(output.contains("export type Tokens = bigint;"), "{output}");
    assert!(
        output.contains("export type BlockIndex = bigint;"),
        "{output}"
    );
    assert!(output.contains("export { $Tokens as Tokens };"), "{output}");
    assert!(
        output.contains("export { $BlockIndex as BlockIndex };"),
        "{output}"
    );
    assert!(
        output.contains(
            "icrc1_balance_of: ((arg0: Account) => Promise<bigint>) & $.WithMode<\"query\">;"
        ),
        "{output}"
    );
    assert!(
        output.contains("export type TransferResult = { tag: \"Ok\"; value: bigint } | { tag: \"Err\"; value: string };"),
        "{output}"
    );
    // Never one alias standing in for the other.
    assert!(!output.contains("= BlockIndex"), "{output}");
    assert!(!output.contains("= Tokens"), "{output}");
    assert!(!output.contains("= $BlockIndex"), "{output}");
    assert!(!output.contains("= $Tokens"), "{output}");
}

/// Every position of a primitive alias — `vec`, `opt`, nested, tuple, variant
/// arm, func argument, actor method — renders structurally, and a declared
/// principal is the runtime's principal type.
#[test]
fn a_declared_primitive_renders_structurally_everywhere() {
    let output = generate_source(
        "type Id = nat; type Bin = nat8; type Who = principal; type Nothing = null;\n\
         type V = record { ids : vec Id; maybe : opt Id; nested : vec vec Bin; who : Who; \
         pair : record { Id; Bin } };\n\
         type A = variant { a : Id; b : Nothing; c : opt Nothing };\n\
         type F = func (Id, vec Bin) -> (Who);\n\
         service : { m : (Id, Bin, vec Bin) -> (Who, Nothing) };",
    );
    for (needle, why) in [
        ("ids: Array<bigint>", "vec of an alias"),
        ("maybe: bigint | null", "opt of an alias"),
        ("nested: Array<Uint8Array>", "vec vec of an alias"),
        ("who: $.Principal", "principal alias"),
        ("pair: [bigint, number]", "tuple of aliases"),
        ("{ tag: \"a\"; value: bigint }", "variant arm"),
        ("{ tag: \"b\" }", "an alias of null is a bare tag"),
        ("{ some: null } | null", "opt of an alias of null boxes"),
        (
            "m: ((arg0: bigint, arg1: number, arg2: Uint8Array) => Promise<[$.Principal, null]>) & $.WithMode<\"update\">;",
            "actor method",
        ),
        (
            "$.c.func([$.c.nat, $.c.blob()], [$.c.principal], \"update\")",
            "func builder",
        ),
    ] {
        assert!(output.contains(needle), "{why}: {needle}\n{output}");
    }
    // Only the four declarations themselves carry the names: with each
    // one's own three lines set aside, nothing references it.
    for name in ["Id", "Bin", "Who", "Nothing"] {
        let own = [
            format!("export type {name} = "),
            format!("const ${name}: $.Schema<{name}> = "),
            format!("export {{ ${name} as {name} }};"),
        ];
        let lines: Vec<&str> = output.lines().collect();
        for line in &own {
            assert_eq!(
                lines
                    .iter()
                    .filter(|l| l.starts_with(line.as_str()))
                    .count(),
                1,
                "{line}\n{output}"
            );
        }
        let rest: Vec<&str> = lines
            .into_iter()
            .filter(|line| !own.iter().any(|own| line.starts_with(own.as_str())))
            .collect();
        assert!(
            !references_declaration(&rest.join("\n"), name),
            "{name} is referenced outside its own three lines:\n{output}"
        );
    }
}

/// The locality rule of #116 applied to value shapes: adding an unrelated
/// declaration of `nat8` never changes another declaration's rendering.
#[test]
fn an_unrelated_primitive_declaration_changes_nothing_else() {
    let without = generate_source(
        "type R = record { raw : blob; a : nat8; big : nat64 }; type G = vec vec nat8;",
    );
    let with = generate_source(
        "type Byte = nat8; type Word = nat64;\n\
         type R = record { raw : blob; a : nat8; big : nat64 }; type G = vec vec nat8;",
    );
    for line in without.lines().filter(|line| {
        line.starts_with("export type R ")
            || line.starts_with("const $R")
            || line.starts_with("export type G ")
            || line.contains("$G")
    }) {
        assert!(
            with.contains(line),
            "`{line}` changed when unrelated aliases were declared"
        );
    }
}

/// Docs are provenance: the compiler-free base surface has none, so a table
/// built from pairs (or empty) emits no JSDoc for the very same Contract.
#[test]
fn without_provenance_no_docs_are_emitted() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs.did"),
    )
    .expect("fixture must be readable");
    let compilation = compile_did(&source).expect("compile");
    let bare = generate_module(
        compilation.contract(),
        &TsNames::new(),
        &TsOptions::default(),
    )
    .expect("generate")
    .module;
    assert!(!bare.contains("/**"), "{bare}");
    assert!(
        bare.contains("arg0: "),
        "argument names are provenance too: {bare}"
    );
    let documented = generate_fixture("docs");
    assert!(documented.contains("/**"));
    // Same Contract, same types: removing the JSDoc and the layout it forces
    // leaves one declaration set.
    assert_eq!(
        bare.matches("export { $").count(),
        documented.matches("export { $").count()
    );
}

/// Byte-identical output across runs, and across the Contract's serialized
/// form: the docs, the layout they force and the parameter names are all
/// functions of the Contract and the sidecar, never of time or map order.
#[test]
fn documented_generation_is_deterministic() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs.did"),
    )
    .expect("fixture must be readable");
    let first = generate_source(&source);
    for _ in 0..3 {
        assert_eq!(first, generate_source(&source));
    }
    let compilation = compile_did(&source).expect("compile");
    let names = TsNames::from_source_info(compilation.source_info().expect("provenance"));
    let json = serde_json::to_string(compilation.contract()).expect("serialize");
    let reparsed = candid_core::Contract::from_json(&json).expect("reparse");
    let again = generate_module(&reparsed, &names, &TsOptions::default())
        .expect("generate")
        .module;
    assert_eq!(first, again);
}

/// What the module's text is outside its block comments, so a test can prove
/// nothing a `.did` comment said escaped into code. Valid only for modules
/// with no quoted name that contains `/*`.
fn outside_comments(module: &str) -> String {
    let mut code = String::new();
    let mut rest = module;
    while let Some(start) = rest.find("/*") {
        code.push_str(&rest[..start]);
        let end = rest[start + 2..]
            .find("*/")
            .unwrap_or_else(|| panic!("an unterminated comment:\n{module}"));
        rest = &rest[start + 2 + end + 2..];
    }
    code.push_str(rest);
    code
}

/// Issue #191, JSDoc safety: doc text that says `*/`, opens tags, fences,
/// smuggles code, or is enormous or spans thousands of lines neither ends its
/// comment early nor reaches the module's code.
#[test]
fn hostile_doc_text_stays_inside_its_comment() {
    let hostile: Vec<String> = [
        "*/ export const pwned = 1; /*",
        "**/ pwned();",
        "*/*/ pwned",
        "ends with * and then */",
        "/** nested opener */ pwned",
        "@param pwned forged",
        "@deprecated pwned",
        "text @returns pwned",
        "{@link pwned",
        "```pwned",
        "` ` ` pwned `",
        "\\*/ pwned",
        "\\",
        "\u{2028}pwned\u{2029}pwned",
        "a\tb\u{85}c\u{feff}d",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    let mut source = String::new();
    for line in &hostile {
        source.push_str(&format!("//{line}\n"));
    }
    source.push_str("type Documented = record {\n");
    for line in &hostile {
        source.push_str(&format!("  //{line}\n"));
    }
    source.push_str("  field : nat;\n};\n");
    let output = generate_source(&source);
    assert!(output.contains("export type Documented ="), "{output}");
    assert!(
        !outside_comments(&output).contains("pwned"),
        "escaped into code:\n{output}"
    );
    // Exactly the generator's own openers and closers, one closer each: an
    // opener is a line that begins a block, and a `/**` said inside a doc is
    // text mid-line, not the start of one.
    let openers = output
        .lines()
        .filter(|line| line.trim_start().starts_with("/**"))
        .count();
    assert_eq!(openers, output.matches("*/").count(), "{output}");
    // Neither `*/` nor a naked tag survives in any doc line.
    for line in output
        .lines()
        .filter(|line| line.trim_start().starts_with('*'))
    {
        assert!(!line.contains("*/") || line.trim() == "*/", "{line}");
        assert!(!line.trim_start().starts_with("* @"), "{line}");
    }
    // Line separators cannot split a doc line: no stray U+2028/U+2029/NEL.
    assert!(
        !output.contains('\u{2028}') && !output.contains('\u{2029}') && !output.contains('\u{85}')
    );
}

/// A 400 KB doc line and a 256-line doc block are emitted whole, with one terminator, and identically on every run.
#[test]
fn very_long_and_very_many_doc_lines_are_emitted_whole() {
    let long = "x".repeat(200_000);
    let source = format!("// {long}*/{long}\ntype T = nat;\n");
    let output = generate_source(&source);
    assert!(
        output.contains(&format!("/** {long}*\\/{long} */\n")),
        "long line kept"
    );
    assert_eq!(
        output.matches("*/").count(),
        2,
        "one closer each for the type and the const"
    );
    assert_eq!(
        output.matches("*\\/").count(),
        2,
        "the said terminator, escaped on both"
    );
    assert_eq!(output, generate_source(&source));

    // Many lines: 256, the most the compiler accepts under default limits.
    // Upstream's `candid_parser` skips each comment of a run by recursing,
    // so `Limits::max_source_nesting` bounds a run of consecutive comments
    // (issue #219), and a longer doc block is refused in the compiler before
    // the generator is reached. That limit is not this generator's to lift.
    let mut many = String::new();
    for index in 0..256 {
        many.push_str(&format!("// line {index} */\n"));
    }
    many.push_str("type T = nat;\n");
    let output = generate_source(&many);
    assert_eq!(output.matches("/**").count(), 2);
    assert!(output.contains(" * line 0 *\\/\n"));
    assert!(output.contains(" * line 255 *\\/\n"));
}

/// CRLF sources document the same as LF ones: the parser keeps a `\r` at the
/// end of a line comment, and the generator trims it.
#[test]
fn crlf_sources_document_identically() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs.did"),
    )
    .expect("fixture must be readable");
    assert_eq!(
        generate_source(&source),
        generate_source(&source.replace('\n', "\r\n"))
    );
}

/// Argument names: the parameters of an `Actor` method take the `.did`'s own
/// names when they are usable TypeScript names, and fall back to `arg{n}` —
/// with no `@param` — for an unnamed argument, a reserved word, a
/// non-identifier, or a fallback that would collide. Two methods that share
/// one function node keep their own names.
#[test]
fn argument_names_become_parameters_and_param_tags() {
    let output = generate_source(
        "service : {\n\
           a : (x : nat) -> ();\n\
           b : (y : nat) -> ();\n\
           c : (nat, text) -> ();\n\
           d : (\"delete\" : nat, \"has space\" : nat, \"ok\" : nat) -> ();\n\
           e : (\"arg1\" : nat, text) -> ();\n\
         };",
    );
    for (needle, why) in [
        ("  /**\n   * @param x\n   */\n  a: ((x: bigint) => Promise<void>) & $.WithMode<\"update\">;", "a"),
        ("  /**\n   * @param y\n   */\n  b: ((y: bigint) => Promise<void>) & $.WithMode<\"update\">;", "b shares a's node"),
        ("  c: ((arg0: bigint, arg1: string) => Promise<void>) & $.WithMode<\"update\">;", "unnamed"),
        (
            "  /**\n   * @param ok\n   */\n  d: ((arg0: bigint, arg1: bigint, ok: bigint) => Promise<void>) & $.WithMode<\"update\">;",
            "reserved and non-identifier names fall back",
        ),
        (
            "  /**\n   * @param arg1\n   */\n  e: ((arg1: bigint, arg1_: string) => Promise<void>) & $.WithMode<\"update\">;",
            "a fallback never steals a declared name",
        ),
    ] {
        assert!(output.contains(needle), "{why}: {needle}\n{output}");
    }
}

/// Where an `Actor` method's docs and argument names come from when the
/// actor's service is not written inline: an actor typed by a declared
/// service (`service : S`) takes the declaration's occurrence; a class actor
/// takes its own; and an actor written inline beside an identical declared
/// service documents itself, not the declaration.
#[test]
fn actor_methods_take_docs_from_the_right_occurrence() {
    let by_reference = generate_source(
        "type S = service {\n  /// from S\n  f : (x : nat) -> ();\n};\n/// the actor\nservice : S;",
    );
    assert!(
        by_reference.contains(
            "  /**\n   * from S\n   * @param x\n   */\n  f: ((x: bigint) => Promise<void>) & $.WithMode<\"update\">;"
        ),
        "{by_reference}"
    );
    assert!(
        by_reference.contains("/** the actor */\nconst $actor"),
        "{by_reference}"
    );

    let class = generate_source(
        "service : (init : nat) -> {\n  /// from the class body\n  g : (y : text) -> ();\n};",
    );
    assert!(
        class.contains("  /**\n   * from the class body\n   * @param y\n   */\n  g: ((y: string) => Promise<void>) & $.WithMode<\"update\">;"),
        "{class}"
    );

    // The inline actor and the declared service are one node: the actor's
    // signature is the actor's own occurrence, the declaration keeps its.
    let both = generate_source(
        "type S = service {\n  /// declared\n  f : (a : nat) -> ();\n};\n\
         service : {\n  /// inline\n  f : (b : nat) -> ();\n};",
    );
    assert!(
        both.contains("   * inline\n   * @param b\n"),
        "the actor documents itself: {both}"
    );
    assert!(!both.contains("declared\n   * @param a"), "{both}");
    assert!(both.contains("export type S = $.Principal;"), "{both}");

    // A method typed by a declared func takes the declaration's names, and
    // its anonymous argument types the declaration's field docs.
    let by_func = generate_source(
        "type H = func (arg : record {\n  /// the id\n  id : nat;\n}) -> ();\n\
         service : { h : H };",
    );
    assert!(by_func.contains("   * @param arg\n"), "{by_func}");
    assert!(by_func.contains("/** the id */\n"), "{by_func}");
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
    .expect("generate")
    .module;
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

/// Issue #218: a documented object spans lines, and each member's nested
/// object indents from the level its parent was at, whatever the members
/// before it rendered. The renderer sets and restores that level around
/// every member; this pins the restore after a nested record and after a
/// union arm with a payload, which no fixture's layout depends on.
#[test]
fn documented_layout_restores_its_level_after_each_member() {
    let source = "type R = record {\n\
                  \x20 /// One.\n\
                  \x20 a : record {\n\
                  \x20   /// Inner one.\n\
                  \x20   x : nat;\n\
                  \x20 };\n\
                  \x20 b : variant { y : nat };\n\
                  \x20 /// Two.\n\
                  \x20 c : record {\n\
                  \x20   /// Inner two.\n\
                  \x20   z : nat;\n\
                  \x20 };\n\
                  };\n";
    assert_eq!(
        generate_source(source),
        "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n\
         import * as $ from \"@candid-core/schema\";\n\
         \n\
         export type R = {\n\
         \x20 /** One. */\n\
         \x20 a: {\n\
         \x20   /** Inner one. */\n\
         \x20   x: bigint;\n\
         \x20 };\n\
         \x20 b: { tag: \"y\"; value: bigint };\n\
         \x20 /** Two. */\n\
         \x20 c: {\n\
         \x20   /** Inner two. */\n\
         \x20   z: bigint;\n\
         \x20 };\n\
         };\n\
         const $R: $.Schema<R> = $.c.rec(() => $.c.record({ a: $.c.record({ x: $.c.nat }), \
         b: $.c.variant({ y: $.c.nat }), c: $.c.record({ z: $.c.nat }) }));\n\
         export { $R as R };\n"
    );
}
