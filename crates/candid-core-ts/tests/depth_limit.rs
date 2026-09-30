//! The depth property of issue #192 (decision D1 as the maintainer settled
//! it), compiler half: any type the candid-core compiler accepts encodes
//! through its generated module at the runtime's default limits, however the
//! type is split into declarations.
//!
//! Each shape below is built at the compiler's maximum accepted Candid depth
//! — a composite at depth 256, the default `max_type_depth` — inline, through
//! a chain of 250 aliases, and through a chain that cycles every composite
//! kind (record field, opt, variant arm, tuple element, func argument,
//! service method). This test asserts the compiler accepts each one and
//! refuses the same shape one level deeper, then emits the generated module
//! and the Contract envelope into `tests/goldens/depth/`;
//! `ts/tests/depth-limit.test.ts` validates, encodes and decodes through both
//! at default limits. Regenerate with
//! `UPDATE_GOLDENS=1 cargo test -p candid-core-ts --features compiler`.
#![cfg(feature = "compiler")]

use std::path::PathBuf;

use candid_core::compile_did;
use candid_core_ts::{generate_module, TsNames, TsOptions};

/// The compiler's default semantic and lexical depth limit.
const LIMIT: usize = 256;

/// `type Deep = vec vec … record {}`: the record sits at Candid depth `depth`.
fn inline(depth: usize) -> String {
    format!("type Deep = {}record {{}};\n", "vec ".repeat(depth))
}

/// 250 declarations `type A{i} = vec A{i+1}`, the last nesting inline so its
/// `record {}` sits at Candid depth `depth`. An alias adds no depth.
fn aliases(depth: usize) -> String {
    let chain = 250;
    let mut source = String::new();
    for i in 0..chain {
        source.push_str(&format!("type A{i} = vec A{};\n", i + 1));
    }
    source.push_str(&format!(
        "type A{chain} = {}record {{}};\n",
        "vec ".repeat(depth - chain)
    ));
    source
}

/// A chain of declarations cycling through every composite kind, each
/// reaching the next through a different position, then inline nesting so the
/// final `record {}` sits at Candid depth `depth`. A service method's type is
/// a func, so a service step is two levels.
fn mixed(depth: usize) -> String {
    let mut source = String::new();
    let mut at = 0;
    let mut i = 0;
    while at + 2 < depth - 4 {
        let next = format!("M{}", i + 1);
        let (declaration, levels) = match i % 6 {
            0 => (format!("record {{ next : {next} }}"), 1),
            1 => (format!("opt {next}"), 1),
            2 => (format!("variant {{ more : {next}; done }}"), 1),
            3 => (format!("record {{ nat; {next} }}"), 1),
            4 => (format!("func ({next}) -> ()"), 1),
            _ => (format!("service {{ call : ({next}) -> () }}"), 2),
        };
        source.push_str(&format!("type M{i} = {declaration};\n"));
        at += levels;
        i += 1;
    }
    source.push_str(&format!(
        "type M{i} = {}record {{}};\n",
        "vec ".repeat(depth - at)
    ));
    source
}

/// A shape: its name, and the Candid source placing a composite at a depth.
type Shape = (&'static str, fn(usize) -> String);

const SHAPES: &[Shape] = &[("inline", inline), ("aliases", aliases), ("mixed", mixed)];

fn goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("depth")
}

fn assert_golden(file_name: &str, text: &str) {
    let path = goldens_dir().join(file_name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(goldens_dir()).expect("goldens dir must be creatable");
        std::fs::write(&path, text).expect("golden must be writable");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {path:?}; run with UPDATE_GOLDENS=1"));
    assert_eq!(
        text, golden,
        "`{file_name}` diverged; regenerate deliberately with UPDATE_GOLDENS=1 and review"
    );
}

#[test]
fn the_compiler_accepts_depth_256_and_refuses_257_in_every_shape() {
    for (name, shape) in SHAPES {
        let accepted = compile_did(&shape(LIMIT));
        assert!(
            accepted.is_ok(),
            "{name}: Candid depth {LIMIT} must compile: {:?}",
            accepted.err().map(|error| error.to_string())
        );
        let refused = compile_did(&shape(LIMIT + 1))
            .err()
            .unwrap_or_else(|| panic!("{name}: Candid depth {} must be refused", LIMIT + 1))
            .to_string();
        assert!(
            refused.starts_with("resource_limit_exceeded:")
                && refused.contains("257 exceeds limit 256"),
            "{name}: refused for the wrong reason: {refused}"
        );
    }
}

/// The generated module and the Contract envelope for each shape at the
/// compiler's maximum, as `ts/tests/depth-limit.test.ts` consumes them.
///
/// Run on a thread with a 64 MiB stack: `generate_module` recurses once per
/// nesting level and overflows a default 2 MiB test thread on the depth-256
/// inline shape (measured). That is the Rust generator's own recursion, not
/// the TypeScript runtime this test is about, and is reported separately.
#[test]
fn depth_limit_modules_and_envelopes() {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(emit_depth_limit_goldens)
        .expect("the generation thread must start")
        .join()
        .expect("generation must not panic");
}

fn emit_depth_limit_goldens() {
    for (name, shape) in SHAPES {
        let compilation = compile_did(&shape(LIMIT)).expect("the maximum depth compiles");
        let source_info = compilation
            .source_info()
            .expect("compile_did retains provenance by default");
        let module = generate_module(
            compilation.contract(),
            &TsNames::from_source_info(source_info),
            &TsOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{name}: the maximum depth must generate: {error}"));
        assert_golden(&format!("{name}.ts"), &module);

        let mut document =
            serde_json::to_value(compilation.contract()).expect("contract must serialize");
        document["producer"] = serde_json::json!({
            "name": "candid-core",
            "version": "0.0.0-golden",
            "candid_version": "0.0.0-golden",
            "candid_parser_version": "0.0.0-golden",
        });
        let contract = candid_core::Contract::from_json(
            &serde_json::to_string(&document).expect("document must serialize"),
        )
        .expect("the normalized document must still be a valid canonical Contract");
        let mut names = std::collections::BTreeMap::new();
        for provenance in source_info.field_labels() {
            if let candid_core::SourceLabel::Named { name } = &provenance.label {
                names.insert((provenance.container, provenance.id), name.clone());
            }
        }
        let triples: Vec<serde_json::Value> = names
            .into_iter()
            .map(|((container, id), label)| serde_json::json!([container, id, label]))
            .collect();
        let mut envelope = candid_core::ContractEnvelope::new(contract);
        envelope
            .insert_extension(
                "org.candid-core.field-names/v1",
                serde_json::Value::Array(triples),
                &candid_core::Limits::default(),
            )
            .expect("the field-names extension must validate");
        let mut text = serde_json::to_string(&envelope).expect("envelope must serialize");
        text.push('\n');
        assert_golden(&format!("{name}.envelope.json"), &text);
    }
}
