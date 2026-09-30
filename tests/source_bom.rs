//! A single leading UTF-8 byte-order mark (`EF BB BF`, U+FEFF) on a `.did`
//! source is accepted by every entry point that takes `.did` text.
//!
//! Editors on Windows save UTF-8 with a BOM by default. The mark is not
//! Candid syntax, so the compiler skips it before tokenizing — and only
//! there. Everything that describes the *raw* source keeps it: the
//! `SourceInfo` source text, `source_bundle_id` (ADR 0001: a raw-source
//! bundle identity), the per-source and bundle byte limits, and diagnostic
//! byte offsets, which still index the original bytes. The semantic
//! identities (`contract_id`, `interface_id`) are those of the BOM-less twin.
//!
//! Only one mark, only at byte 0: a second mark, a mark after any other byte,
//! and the UTF-16 marks stay rejected exactly as before.

use candid_core::{
    compile_did, compile_did_with_context, compile_with_resolver, Compilation, CompileError,
    CompileOptions, Limits, MemoryResolver, RuntimeContext,
};

const BOM: &str = "\u{FEFF}";
const SERVICE: &str = "type Item = record { id: nat64; label: text };\n\
    // Reads one item.\n\
    service : { get: (nat64) -> (opt Item) query };\n";

fn with_bom(source: &str) -> String {
    format!("{BOM}{source}")
}

fn identities(compilation: &Compilation) -> (String, Option<String>) {
    (
        compilation.contract().contract_id().to_string(),
        compilation.contract().interface_id().map(str::to_string),
    )
}

fn bundle_id(compilation: &Compilation) -> &str {
    compilation
        .source_info()
        .expect("provenance is on by default")
        .source_bundle_id()
}

fn first_diagnostic(error: CompileError) -> candid_core::Diagnostic {
    error
        .diagnostics
        .into_iter()
        .next()
        .expect("one diagnostic")
}

fn span_bytes(diagnostic: &candid_core::Diagnostic) -> (Option<u64>, Option<u64>) {
    let span = diagnostic.span.as_ref().expect("parse errors carry a span");
    (span.start_byte, span.end_byte)
}

#[test]
fn leading_bom_compiles_to_the_bomless_semantic_identities() {
    let plain = compile_did(SERVICE).unwrap();
    let marked = compile_did(&with_bom(SERVICE)).unwrap();

    assert_eq!(identities(&marked), identities(&plain));
    assert!(identities(&marked).1.is_some(), "the fixture has an actor");
    assert_eq!(
        marked.contract().to_json_pretty().unwrap(),
        plain.contract().to_json_pretty().unwrap(),
        "the whole canonical Contract is the BOM-less one"
    );
    // The doc comment directly after the mark is still attached: the mark is
    // skipped before trivia collection, not treated as a token.
    let marked_info = marked.source_info().unwrap();
    let plain_info = plain.source_info().unwrap();
    assert_eq!(marked_info.methods(), plain_info.methods());
    assert_eq!(marked_info.declarations(), plain_info.declarations());
}

#[test]
fn source_bundle_id_hashes_the_raw_bytes_including_the_bom() {
    let plain = compile_did(SERVICE).unwrap();
    let marked = compile_did(&with_bom(SERVICE)).unwrap();

    assert_ne!(
        bundle_id(&marked),
        bundle_id(&plain),
        "source_bundle_id is a raw-source identity (ADR 0001)"
    );
    let source = &marked.source_info().unwrap().sources()[0].source;
    assert_eq!(source, &with_bom(SERVICE), "the sidecar keeps the raw text");
    assert_eq!(&source.as_bytes()[..3], b"\xEF\xBB\xBF");
}

#[test]
fn bom_sidecar_authenticates_and_its_raw_text_is_load_bearing() {
    let marked = compile_did(&with_bom(SERVICE)).unwrap();
    let json = marked
        .to_json_pretty_with_limits(&Limits::default())
        .unwrap();

    // Validation re-derives the bundle from the sidecar's raw sources; the
    // mark must survive that round trip.
    let parsed = Compilation::from_json_with_limits(&json, &Limits::default()).unwrap();
    assert_eq!(parsed, marked);

    // Dropping the mark from the sidecar text while keeping the recorded
    // identity is a different raw bundle, and is refused as such.
    let mut document: serde_json::Value = serde_json::from_str(&json).unwrap();
    let text = &mut document["source_info"]["sources"][0]["source"];
    let stripped = text
        .as_str()
        .unwrap()
        .strip_prefix(BOM)
        .unwrap()
        .to_string();
    *text = serde_json::Value::String(stripped);
    let error =
        Compilation::from_json_with_limits(&document.to_string(), &Limits::default()).unwrap_err();
    assert!(
        format!("{error:?}").contains("source_bundle_id_mismatch"),
        "{error:?}"
    );
}

#[test]
fn resolver_bundle_accepts_a_bom_on_the_entry_and_on_an_import() {
    let entry = "import \"types.did\";\nservice : { get: (nat64) -> (opt Item) query };\n";
    let types = "type Item = record { id: nat64; label: text };\n";
    let compile = |entry: &str, types: &str| {
        let resolver = MemoryResolver::new()
            .with_source("memory:/entry.did", entry)
            .unwrap()
            .with_source("memory:/types.did", types)
            .unwrap();
        compile_with_resolver(
            "memory:/entry.did",
            &resolver,
            CompileOptions::default(),
            &RuntimeContext::default(),
        )
        .unwrap()
    };

    let plain = compile(entry, types);
    for (label, marked) in [
        ("entry", compile(&with_bom(entry), types)),
        ("import", compile(entry, &with_bom(types))),
        ("both", compile(&with_bom(entry), &with_bom(types))),
    ] {
        assert_eq!(identities(&marked), identities(&plain), "{label}");
        assert_ne!(bundle_id(&marked), bundle_id(&plain), "{label}");
    }
}

#[test]
fn parse_error_offsets_index_the_raw_bytes() {
    let broken = "service : { broken: (nat) -> ( };";
    let plain = first_diagnostic(compile_did(broken).unwrap_err());
    let marked = first_diagnostic(compile_did(&with_bom(broken)).unwrap_err());

    assert_eq!(plain.code, "did_parse_error");
    assert_eq!(marked.code, "did_parse_error");
    assert_eq!(span_bytes(&plain), (Some(31), Some(32)));
    assert_eq!(
        span_bytes(&marked),
        (Some(34), Some(35)),
        "offsets are into the original file, BOM included"
    );
    assert_eq!(
        marked.message,
        "Candid parser error: Unexpected token at bytes 34..35"
    );
    assert_eq!(marked.notes, plain.notes);
    assert_eq!(
        marked.span.as_ref().unwrap().source_name.as_deref(),
        Some("memory:/inline.did")
    );
}

#[test]
fn bom_only_source_is_the_empty_source() {
    let empty = compile_did("").unwrap();
    let marked = compile_did(BOM).unwrap();
    assert_eq!(identities(&marked), identities(&empty));
    assert_eq!(identities(&marked).1, None, "an empty source is actorless");
    assert_ne!(bundle_id(&marked), bundle_id(&empty));
}

#[test]
fn a_second_bom_is_still_rejected_at_its_raw_offset() {
    let source = format!("{BOM}{BOM}{SERVICE}");
    let diagnostic = first_diagnostic(compile_did(&source).unwrap_err());
    assert_eq!(diagnostic.code, "did_parse_error");
    assert_eq!(span_bytes(&diagnostic), (Some(3), Some(6)));
}

#[test]
fn a_bom_anywhere_but_byte_zero_is_still_rejected() {
    // After leading whitespace.
    let diagnostic = first_diagnostic(compile_did(&format!(" {BOM}{SERVICE}")).unwrap_err());
    assert_eq!(diagnostic.code, "did_parse_error");
    assert_eq!(span_bytes(&diagnostic), (Some(1), Some(4)));

    // Mid-file, with and without a leading mark: the offset is raw either way.
    let body = "service : {};";
    let diagnostic = first_diagnostic(compile_did(&format!("{body}{BOM}")).unwrap_err());
    assert_eq!(span_bytes(&diagnostic), (Some(13), Some(16)));
    let diagnostic = first_diagnostic(compile_did(&format!("{BOM}{body}{BOM}")).unwrap_err());
    assert_eq!(span_bytes(&diagnostic), (Some(16), Some(19)));
}

#[test]
fn byte_swapped_mark_is_not_a_bom() {
    // U+FFFE is what a UTF-16 BOM read with the wrong byte order decodes to.
    let diagnostic = first_diagnostic(compile_did(&format!("\u{FFFE}{SERVICE}")).unwrap_err());
    assert_eq!(diagnostic.code, "did_parse_error");
    assert_eq!(span_bytes(&diagnostic), (Some(0), Some(3)));
}

#[test]
fn a_bom_does_not_bypass_the_source_nesting_preflight() {
    // The nesting preflight tokenizes before the recursive parser runs. It
    // must see past the mark, or a marked, hostile source would reach the
    // parser unchecked.
    let nested = |depth: usize| format!("type T = {}nat; service : {{}};", "opt ".repeat(depth));
    let limits = Limits::default()
        .with_max_source_nesting(32)
        .with_max_type_depth(64);
    let context = RuntimeContext::new(limits);
    compile_did_with_context(&with_bom(&nested(32)), CompileOptions::default(), &context).unwrap();
    let diagnostic = first_diagnostic(
        compile_did_with_context(&with_bom(&nested(33)), CompileOptions::default(), &context)
            .unwrap_err(),
    );
    let resource = diagnostic.resource_limit.expect("a resource refusal");
    assert_eq!(resource.resource, "source_nesting");
    assert_eq!((resource.limit, resource.observed), (32, 33));

    // And under default limits, a hostile depth is refused rather than
    // overflowing the stack.
    let diagnostic = first_diagnostic(compile_did(&with_bom(&nested(3_000))).unwrap_err());
    assert_eq!(
        diagnostic
            .resource_limit
            .expect("a resource refusal")
            .resource,
        "source_nesting"
    );
}

#[test]
fn the_bom_counts_against_the_source_byte_limit() {
    // Limits bound raw bytes, the same bytes the digest and bundle id cover.
    let limit = SERVICE.len();
    let context = RuntimeContext::new(Limits::default().with_max_source_bytes(limit));
    compile_did_with_context(SERVICE, CompileOptions::default(), &context).unwrap();
    let diagnostic = first_diagnostic(
        compile_did_with_context(&with_bom(SERVICE), CompileOptions::default(), &context)
            .unwrap_err(),
    );
    let resource = diagnostic.resource_limit.expect("a resource refusal");
    assert_eq!(resource.resource, "source_bytes");
    assert_eq!(resource.observed, (limit + 3) as u64);
}

#[cfg(feature = "filesystem-compiler")]
mod files {
    use super::*;
    use candid_core::{compile_did_file, WorkspaceResolver};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Workspace(PathBuf);

    impl Workspace {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "candid-core-bom-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ENTRY: &str = "import \"types.did\";\nservice : { get: (nat64) -> (opt Item) query };\n";
    const TYPES: &str = "type Item = record { id: nat64; label: text };\n";

    #[test]
    fn native_file_compilation_accepts_bom_files_and_agrees_with_the_resolver_path() {
        let plain = Workspace::new();
        plain.write("types.did", TYPES);
        let plain = compile_did_file(plain.write("service.did", ENTRY)).unwrap();

        let marked = Workspace::new();
        marked.write("types.did", with_bom(TYPES));
        let path = marked.write("service.did", with_bom(ENTRY));
        let native = compile_did_file(&path).unwrap();

        assert_eq!(identities(&native), identities(&plain));
        assert_ne!(bundle_id(&native), bundle_id(&plain));
        for source in native.source_info().unwrap().sources() {
            assert!(
                source.source.starts_with(BOM),
                "{} keeps its mark",
                source.name
            );
        }

        // The in-memory backend over the same workspace reaches the same
        // artifact, sidecar and all.
        let resolver = WorkspaceResolver::new(&marked.0).unwrap();
        let memory = compile_with_resolver(
            "service.did",
            &resolver,
            CompileOptions::default(),
            &RuntimeContext::default(),
        )
        .unwrap();
        assert_eq!(memory, native);
    }

    #[test]
    fn native_parse_error_offsets_index_the_raw_file() {
        let workspace = Workspace::new();
        let path = workspace.write("broken.did", with_bom("service : { broken: (nat) -> ( };"));
        let diagnostic = first_diagnostic(compile_did_file(path).unwrap_err());
        assert_eq!(diagnostic.code, "did_parse_error");
        assert_eq!(span_bytes(&diagnostic), (Some(34), Some(35)));
        assert_eq!(
            diagnostic.span.unwrap().source_name.as_deref(),
            Some("workspace:/broken.did")
        );
    }

    #[test]
    fn utf16_boms_are_still_rejected_as_invalid_utf8() {
        let workspace = Workspace::new();
        for (name, bom) in [("le.did", [0xFF, 0xFE]), ("be.did", [0xFE, 0xFF])] {
            let mut bytes = bom.to_vec();
            bytes.extend_from_slice(b"s\0e\0r\0v\0i\0c\0e\0");
            let diagnostic =
                first_diagnostic(compile_did_file(workspace.write(name, bytes)).unwrap_err());
            assert_eq!(diagnostic.code, "did_file_read_error", "{name}");
        }
    }
}
