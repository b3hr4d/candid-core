//! The in-memory backend against the native materialized backend, on the same
//! logical bundle.
//!
//! Issue #21 promoted [`super::compile_with_resolver`] onto the
//! filesystem-free [`super::bundle`] backend while [`super::compile_did_file`]
//! kept the materialized `candid_parser::check_file` backend. That split is
//! only sound while the two agree, and "they agree" has to be a checked
//! property of this crate's own output rather than an assumption about
//! upstream behaviour.
//!
//! For every valid bundle these tests require byte-identical Contracts,
//! identities, and provenance from both backends. For invalid bundles they
//! require the same stable diagnostic code and phase — message text is not a
//! stable interface, and the two backends legitimately word some failures
//! differently.
//!
//! Native-only by construction: only one of the two backends has a
//! filesystem to materialize into. The same bundles are exercised in a real
//! browser by `tests/browser_wasm.rs`.

use super::*;
use crate::{MemoryResolver, RuntimeContext};

fn resolver(sources: &[(&str, &str)]) -> MemoryResolver {
    let mut resolver = MemoryResolver::new();
    for (id, source) in sources {
        resolver.insert(id, *source).expect("fixture must be valid");
    }
    resolver
}

fn in_memory(entry: &str, sources: &[(&str, &str)]) -> Result<Compilation, CompileError> {
    let context = RuntimeContext::default();
    let mut budget = context.budget();
    compile_resolved_bundle(
        entry,
        &resolver(sources),
        CompileOptions::default(),
        &context,
        &mut budget,
    )
}

fn materialized(entry: &str, sources: &[(&str, &str)]) -> Result<Compilation, CompileError> {
    compile_materialized_bundle(
        entry,
        &resolver(sources),
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
}

/// Both backends must produce the same Compilation for the same logical input.
///
/// Equality on `Compilation` already covers the whole artifact, but the
/// identities and the canonical JSON are asserted separately so a failure
/// names which invariant broke instead of printing two large graphs.
#[track_caller]
fn assert_backends_agree(label: &str, entry: &str, sources: &[(&str, &str)]) -> Compilation {
    let memory = in_memory(entry, sources)
        .unwrap_or_else(|error| panic!("{label}: the in-memory backend must compile: {error:#?}"));
    let native = materialized(entry, sources).unwrap_or_else(|error| {
        panic!("{label}: the materialized backend must compile: {error:#?}")
    });

    assert_eq!(
        memory.contract().contract_id(),
        native.contract().contract_id(),
        "{label}: contract_id must not depend on the backend"
    );
    assert_eq!(
        memory.contract().interface_id(),
        native.contract().interface_id(),
        "{label}: interface_id must not depend on the backend"
    );
    assert_eq!(
        memory.contract().to_json_pretty().unwrap(),
        native.contract().to_json_pretty().unwrap(),
        "{label}: canonical Contract bytes must not depend on the backend"
    );

    let memory_info = memory
        .source_info()
        .unwrap_or_else(|| panic!("{label}: provenance is on by default"));
    let native_info = native
        .source_info()
        .unwrap_or_else(|| panic!("{label}: provenance is on by default"));
    assert_eq!(
        memory_info.source_bundle_id(),
        native_info.source_bundle_id(),
        "{label}: source_bundle_id must not depend on the backend"
    );
    assert_eq!(
        serde_json::to_value(memory_info).unwrap(),
        serde_json::to_value(native_info).unwrap(),
        "{label}: every provenance collection must not depend on the backend"
    );
    assert_eq!(memory, native, "{label}: the whole artifact must agree");
    memory
}

/// Both backends must reject the same bundle with the same stable code and
/// phase. Returns the in-memory diagnostic so a caller can pin more.
#[track_caller]
fn assert_backends_reject(label: &str, entry: &str, sources: &[(&str, &str)]) -> Diagnostic {
    let memory = in_memory(entry, sources)
        .err()
        .unwrap_or_else(|| panic!("{label}: the in-memory backend must reject this bundle"));
    let native = materialized(entry, sources)
        .err()
        .unwrap_or_else(|| panic!("{label}: the materialized backend must reject this bundle"));
    let memory = memory.diagnostics.into_iter().next().unwrap();
    let native = native.diagnostics.into_iter().next().unwrap();
    assert_eq!(
        (memory.code.as_str(), &memory.phase),
        (native.code.as_str(), &native.phase),
        "{label}: the stable diagnostic code and phase must not depend on the backend"
    );
    assert_eq!(
        memory.resource_limit, native.resource_limit,
        "{label}: the resource triple must not depend on the backend"
    );
    memory
}

#[test]
fn plain_type_import_agrees() {
    let compilation = assert_backends_agree(
        "type import",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"types.did\";\nservice : { read: () -> (Item) query };",
            ),
            (
                "memory:/types.did",
                "type Item = record { id: nat64; label: text };",
            ),
        ],
    );
    let info = compilation.source_info().unwrap();
    assert_eq!(info.sources().len(), 2);
    assert_eq!(info.imports().len(), 1);
}

#[test]
fn service_import_and_type_import_together_agree() {
    // Actor inclusion is the property that separates the two import kinds:
    // the imported service's methods must appear in the merged actor, while
    // the imported types must not contribute methods.
    let compilation = assert_backends_agree(
        "service plus type import",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import service \"api.did\";\nimport \"types.did\";\nservice : { local: (Item) -> () };",
            ),
            (
                "memory:/api.did",
                "service : { imported: () -> (nat) query };",
            ),
            ("memory:/types.did", "type Item = record { id: nat };"),
        ],
    );
    let json = serde_json::to_string(compilation.contract()).unwrap();
    assert!(json.contains("imported"), "{json}");
    assert!(json.contains("local"), "{json}");
}

#[test]
fn diamond_and_repeated_imports_agree() {
    // `left` and `right` both import `shared`, and the entry imports `shared`
    // a second time directly. The bundle must contain one `shared` unit, and
    // both backends must agree on the result of collapsing it.
    let compilation = assert_backends_agree(
        "diamond import",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"left.did\";\nimport \"right.did\";\nimport \"shared.did\";\nservice : { read: (Left, Right, Shared) -> () };",
            ),
            (
                "memory:/left.did",
                "import \"shared.did\";\ntype Left = record { shared: Shared };",
            ),
            (
                "memory:/right.did",
                "import \"shared.did\";\ntype Right = variant { shared: Shared; none };",
            ),
            ("memory:/shared.did", "type Shared = nat;"),
        ],
    );
    let info = compilation.source_info().unwrap();
    assert_eq!(info.sources().len(), 4, "the diamond collapses to one unit");
    assert_eq!(info.imports().len(), 5, "every edge is still recorded");
}

#[test]
fn repeated_type_then_service_import_of_one_target_agrees() {
    // The same target reached by both a type import and a service import must
    // keep actor inclusion: `include_actor` is a logical OR over its edges,
    // and the merge order differs between the two backends, so this is the
    // case where an order-sensitive bug would show up.
    assert_backends_agree(
        "repeated type and service import",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"api.did\";\nimport service \"api.did\";\nservice : { local: (Exposed) -> () };",
            ),
            (
                "memory:/api.did",
                "type Exposed = nat;\nservice : { imported: () -> (Exposed) query };",
            ),
        ],
    );
}

#[test]
fn recursive_and_mutually_recursive_imports_agree() {
    assert_backends_agree(
        "recursion across sources",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"tree.did\";\nservice : { walk: (Node) -> (Branch) query };",
            ),
            (
                "memory:/tree.did",
                "type Node = record { children: vec Node; branch: opt Branch };\ntype Branch = variant { leaf: Node; empty };",
            ),
        ],
    );
}

#[test]
fn actorless_imported_bundle_agrees() {
    let compilation = assert_backends_agree(
        "actorless bundle",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"types.did\";\ntype Local = Item;",
            ),
            ("memory:/types.did", "type Item = record { id: nat };"),
        ],
    );
    assert!(
        compilation.contract().interface_id().is_none(),
        "an actorless Contract has no interface identity"
    );
}

#[test]
fn class_actor_with_imported_types_agrees() {
    assert_backends_agree(
        "service class",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"types.did\";\nservice : (Item) -> { read: () -> (Item) query };",
            ),
            ("memory:/types.did", "type Item = record { id: nat };"),
        ],
    );
}

#[test]
fn imported_service_without_an_actor_is_rejected_the_same_way() {
    let diagnostic = assert_backends_reject(
        "service import with no main service",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import service \"lib.did\";\nservice : {};",
            ),
            ("memory:/lib.did", "type T = nat;"),
        ],
    );
    assert_eq!(diagnostic.code, "did_type_check_error");
    assert_eq!(diagnostic.phase, Some(DiagnosticPhase::TypeCheck));
    // The in-memory backend knows which unit failed to merge without parsing
    // any message, so it scopes the diagnostic to that logical source — the
    // same location the materialized backend recovers afterwards.
    assert_eq!(
        diagnostic.span,
        Some(SourceSpan::source_only("memory:/lib.did"))
    );
    assert_eq!(
        diagnostic.message,
        "Imported service file \"memory:/lib.did\" has no main service"
    );
}

#[test]
fn imported_service_constructor_is_rejected_the_same_way() {
    let diagnostic = assert_backends_reject(
        "service import of a class",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import service \"lib.did\";\nservice : { local: () -> () };",
            ),
            (
                "memory:/lib.did",
                "service : (nat) -> { imported: () -> () };",
            ),
        ],
    );
    assert_eq!(diagnostic.code, "did_type_check_error");
    assert_eq!(
        diagnostic.span,
        Some(SourceSpan::source_only("memory:/lib.did")),
        "upstream names exactly one logical source, so it may be scoped"
    );
}

#[test]
fn unbound_imported_type_is_rejected_the_same_way() {
    let diagnostic = assert_backends_reject(
        "unbound imported type",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"types.did\";\nservice : { read: () -> (Missing) query };",
            ),
            ("memory:/types.did", "type Item = nat;"),
        ],
    );
    assert_eq!(diagnostic.code, "did_type_check_error");
    assert_eq!(diagnostic.phase, Some(DiagnosticPhase::TypeCheck));
}

#[test]
fn duplicate_declarations_across_sources_are_rejected_the_same_way() {
    let diagnostic = assert_backends_reject(
        "duplicate binding",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"left.did\";\nimport \"right.did\";\nservice : { read: () -> (Item) query };",
            ),
            ("memory:/left.did", "type Item = nat;"),
            ("memory:/right.did", "type Item = text;"),
        ],
    );
    assert_eq!(diagnostic.code, "did_type_check_error");
}

#[test]
fn a_non_service_actor_is_rejected_the_same_way() {
    let diagnostic = assert_backends_reject(
        "actor is not a service",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"types.did\";\nservice : Item;",
            ),
            ("memory:/types.did", "type Item = record { id: nat };"),
        ],
    );
    assert_eq!(diagnostic.code, "did_type_check_error");
    assert_eq!(
        diagnostic.span, None,
        "no single source owns this failure, so no location is invented"
    );
}

#[test]
fn parse_failures_stay_at_load_time_on_both_backends() {
    // Both backends parse every unit while loading it, so a syntactically
    // invalid import fails identically — with exact offsets against the
    // original text, before either checker runs.
    let diagnostic = assert_backends_reject(
        "invalid imported syntax",
        "memory:/entry.did",
        &[
            (
                "memory:/entry.did",
                "import \"types.did\";\nservice : { read: () -> (Item) query };",
            ),
            ("memory:/types.did", "type Broken = ;"),
        ],
    );
    assert_eq!(diagnostic.code, "did_parse_error");
    assert_eq!(diagnostic.phase, Some(DiagnosticPhase::Parse));
    let span = diagnostic.span.expect("parse errors keep exact offsets");
    assert_eq!(span.source_name.as_deref(), Some("memory:/types.did"));
    assert_eq!((span.start_byte, span.end_byte), (Some(14), Some(15)));
}

#[test]
fn no_diagnostic_from_either_backend_leaks_a_materialized_identity() {
    for (label, entry, sources) in [
        (
            "no main service",
            "memory:/entry.did",
            &[
                (
                    "memory:/entry.did",
                    "import service \"lib.did\";\nservice : {};",
                ),
                ("memory:/lib.did", "type T = nat;"),
            ][..],
        ),
        (
            "unbound type",
            "memory:/entry.did",
            &[
                (
                    "memory:/entry.did",
                    "import \"types.did\";\nservice : { read: () -> (Missing) query };",
                ),
                ("memory:/types.did", "type Item = nat;"),
            ][..],
        ),
    ] {
        for (backend, error) in [
            ("in-memory", in_memory(entry, sources)),
            ("materialized", materialized(entry, sources)),
        ] {
            let error = error
                .err()
                .unwrap_or_else(|| panic!("{label}/{backend} must fail"));
            let rendered = serde_json::to_string(&error.diagnostics).unwrap();
            for leak in ["0.did", "1.did", "candid-core-"] {
                assert!(
                    !rendered.contains(leak),
                    "{label}/{backend} leaked {leak:?}: {rendered}"
                );
            }
            let temp_dir = std::env::temp_dir().display().to_string();
            assert!(
                !rendered.contains(temp_dir.trim_end_matches('/')),
                "{label}/{backend} leaked the temp directory: {rendered}"
            );
        }
    }
}

#[test]
fn the_promoted_entry_point_and_the_native_file_path_agree_on_a_workspace_bundle() {
    // The one case that crosses both a real filesystem and the promoted
    // in-memory backend: `compile_did_file` reads the checked-in fixture
    // through `WorkspaceResolver` and the materialized checker, while
    // `compile_with_resolver` compiles the identical bytes from memory. Only
    // the logical source IDs differ (`workspace:/` versus `memory:/`), so the
    // Contract must match exactly and the provenance must match once the
    // scheme is accounted for.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/imports");
    let native = crate::compile_did_file(format!("{root}/root.did")).unwrap();
    let memory = crate::compile_with_resolver(
        "memory:/root.did",
        &resolver(&[
            (
                "memory:/root.did",
                &std::fs::read_to_string(format!("{root}/root.did")).unwrap(),
            ),
            (
                "memory:/types.did",
                &std::fs::read_to_string(format!("{root}/types.did")).unwrap(),
            ),
        ]),
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap();

    assert_eq!(
        native.contract().to_json_pretty().unwrap(),
        memory.contract().to_json_pretty().unwrap(),
        "the same source bytes must produce the same Contract from either entry point"
    );
    let native_sources: Vec<_> = native
        .source_info()
        .unwrap()
        .sources()
        .iter()
        .map(|source| source.name.replace("workspace:/", "memory:/"))
        .collect();
    let memory_sources: Vec<_> = memory
        .source_info()
        .unwrap()
        .sources()
        .iter()
        .map(|source| source.name.clone())
        .collect();
    assert_eq!(native_sources, memory_sources);
}
