use std::path::{Path, PathBuf};

pub struct DidCase {
    pub name: &'static str,
    pub source: String,
}

pub fn import_free_cases() -> Vec<DidCase> {
    vec![
        DidCase {
            name: "basic",
            source: include_str!("../../tests/fixtures/conformance/basic.did").to_string(),
        },
        DidCase {
            name: "recursive",
            source: include_str!("../../tests/fixtures/conformance/recursive.did").to_string(),
        },
        DidCase {
            name: "ledger",
            source: ledger_source().to_string(),
        },
        DidCase {
            name: "wide_record_256",
            source: wide_record(256),
        },
        DidCase {
            name: "service_methods_128",
            source: service_methods(128),
        },
        DidCase {
            name: "recursive_chain_128",
            source: recursive_chain(128),
        },
    ]
}

pub fn ledger_source() -> &'static str {
    include_str!("../corpus/ledger.did")
}

pub fn imported_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("corpus")
        .join("imports")
        .join("root.did")
}

pub fn imported_bundle_bytes() -> u64 {
    [
        include_str!("../corpus/imports/root.did"),
        include_str!("../corpus/imports/common.did"),
        include_str!("../corpus/imports/archive.did"),
    ]
    .iter()
    .map(|source| source.len() as u64)
    .sum()
}

fn wide_record(fields: usize) -> String {
    let mut source = String::from("type Wide = record {\n");
    for index in 0..fields {
        source.push_str(&format!("  field_{index}: nat;\n"));
    }
    source.push_str("};\nservice : { put: (Wide) -> (); get: () -> (Wide) query };\n");
    source
}

fn service_methods(methods: usize) -> String {
    let mut source = String::from("type Result = variant { ok: text; err: text };\nservice : {\n");
    for index in 0..methods {
        source.push_str(&format!(
            "  method_{index}: (nat64, text) -> (Result) query;\n"
        ));
    }
    source.push_str("};\n");
    source
}

fn recursive_chain(depth: usize) -> String {
    let mut source = String::new();
    for index in 0..depth {
        source.push_str(&format!(
            "type Node{index} = record {{ value: nat; next: opt Node{} }};\n",
            index + 1
        ));
    }
    source.push_str(&format!(
        "type Node{depth} = record {{ value: nat }};\nservice : {{ read: () -> (Node0) query }};\n"
    ));
    source
}
