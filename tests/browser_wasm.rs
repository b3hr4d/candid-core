//! Imported-bundle compilation, executed in a real browser.
//!
//! Every case below runs twice in CI, from this one file:
//!
//! * on `wasm32-unknown-unknown` each case is a `wasm_bindgen_test` configured
//!   with `run_in_browser`, executed in headless Chrome by the `browser` job;
//! * on every other target the *identical* assertion bodies run under the
//!   ordinary test harness, in every native job.
//!
//! Sharing one body is the parity mechanism. A pinned identity that drifted on
//! one side but not the other could not be expressed here: there is only one
//! literal, and both targets compare against it.
//!
//! What this proves that a `cargo check --target wasm32-unknown-unknown` does
//! not: that the promoted `compile_with_resolver` actually *runs* in a browser
//! — parsing, merging, type checking, lowering, canonicalizing, and hashing —
//! and that its failure modes stay structured there, including the deadline
//! boundary on a target with no clock.
//!
//! Scope is deliberately narrow (issue #21): the resolver supplies immutable
//! source data synchronously. Nothing here fetches, resolves asynchronously,
//! consults a registry, or generates bindings.

#![cfg(feature = "compiler")]

use candid_core::{
    compile_did, compile_with_resolver, CancellationToken, CompileError, CompileOptions,
    DiagnosticPhase, Limits, MemoryResolver, RawSourceInfo, RuntimeContext, SourceId, SourceInfo,
};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_browser);

/// One case body, two harnesses: a browser test on bare WASM and an ordinary
/// test everywhere else.
macro_rules! browser_case {
    ($(#[$attribute:meta])* fn $name:ident() $body:block) => {
        $(#[$attribute])*
        #[cfg_attr(
            all(target_arch = "wasm32", target_os = "unknown"),
            wasm_bindgen_test::wasm_bindgen_test
        )]
        #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
        fn $name() $body
    };
}

// ---------------------------------------------------------------------------
// The bundle under test, and its pinned semantic identities.
// ---------------------------------------------------------------------------

/// A four-source bundle that exercises both import kinds at once:
///
/// * `types.did` is a plain type import, reached from two different sources —
///   the diamond, which must collapse to a single loaded unit;
/// * `api.did` is an `import service`, whose methods must join the entry
///   actor;
/// * `left.did` is an ordinary transitive type import.
const BUNDLE: &[(&str, &str)] = &[
    (
        "memory:/entry.did",
        concat!(
            "import \"types.did\";\n",
            "import service \"api.did\";\n",
            "import \"left.did\";\n",
            "/// Entry service documentation.\n",
            "service : {\n",
            "  /// Local documentation.\n",
            "  local: (item: Item) -> (leaf: Leaf) query;\n",
            "};\n"
        ),
    ),
    (
        "memory:/types.did",
        "/// Item documentation.\ntype Item = record { id: nat64; label: text };\n",
    ),
    (
        "memory:/api.did",
        "service : { imported: () -> (nat) query };\n",
    ),
    (
        "memory:/left.did",
        "import \"types.did\";\ntype Leaf = record { item: Item };\n",
    ),
];

/// Pinned identities of [`BUNDLE`].
///
/// These are the semantic-output-compatibility assertion for issue #21: the
/// promoted in-memory backend must produce exactly the Contract the
/// materialized backend produced for the same logical input. `src/compile/
/// differential.rs` proves the two backends agree natively; these literals
/// carry that agreement across releases and into the browser.
const BUNDLE_CONTRACT_ID: &str =
    "candid-core:contract:v1:sha256:3ffcbc083e3d254570bf3240db14b9312f217282a674ec9518c6b8b0fee42482";
const BUNDLE_INTERFACE_ID: &str =
    "candid-core:interface:v1:sha256:5e632ffd738139f60fe8755bcd5e99f077eae2b4f32b2bff0e60b8eff1096091";
const BUNDLE_SOURCE_BUNDLE_ID: &str =
    "candid-core:source-bundle:v1:sha256:cb3eebea62cdb70e945171c72d2ccb48f29c19885a4b4add278a92e49243181b";

/// Every logical source of [`BUNDLE`], in the deterministic order the sidecar
/// records them — sorted by logical ID, not by load order, so the identity is
/// independent of traversal.
const BUNDLE_SOURCES: &[&str] = &[
    "memory:/api.did",
    "memory:/entry.did",
    "memory:/left.did",
    "memory:/types.did",
];

/// Every import edge of [`BUNDLE`] as `(from, spelling, to)`, in the same
/// deterministic sorted order. `types.did` appears twice — once per importer —
/// while the loaded unit list above holds it once, which is exactly what
/// collapsing the diamond means.
const BUNDLE_IMPORTS: &[(&str, &str, &str)] = &[
    ("memory:/entry.did", "api.did", "memory:/api.did"),
    ("memory:/entry.did", "left.did", "memory:/left.did"),
    ("memory:/entry.did", "types.did", "memory:/types.did"),
    ("memory:/left.did", "types.did", "memory:/types.did"),
];

const SELF_CONTAINED: &str =
    "type Item = record { id: nat }; service : { ping: (Item) -> () query };";

fn resolver(sources: &[(&str, &str)]) -> MemoryResolver {
    let mut resolver = MemoryResolver::new();
    for (id, source) in sources {
        resolver
            .insert(*id, *source)
            .expect("every fixture source ID is canonical");
    }
    resolver
}

fn compile_bundle(context: &RuntimeContext) -> Result<candid_core::Compilation, CompileError> {
    compile_with_resolver(
        "memory:/entry.did",
        &resolver(BUNDLE),
        CompileOptions::default(),
        context,
    )
}

/// A diagnostic must never mention a materialized file name, a temporary
/// directory, or a native path — least of all on a target that has none.
fn assert_no_filesystem_leakage(error: &CompileError) {
    let rendered = serde_json::to_string(&error.diagnostics).expect("diagnostics must serialize");
    for leak in [
        "\"0.did\"",
        "\"1.did\"",
        "\"2.did\"",
        "\"3.did\"",
        "candid-core-",
        "/tmp",
        "/var/folders",
        "\\\\",
        "C:",
    ] {
        assert!(
            !rendered.contains(leak),
            "diagnostic leaked {leak:?}: {rendered}"
        );
    }
    for diagnostic in &error.diagnostics {
        if let Some(name) = diagnostic
            .span
            .as_ref()
            .and_then(|span| span.source_name.as_deref())
        {
            assert!(
                BUNDLE_SOURCES.contains(&name) || name.starts_with("memory:/"),
                "a span may only name a logical source: {name:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cases.
// ---------------------------------------------------------------------------

browser_case! {
    /// The pre-#21 browser baseline: a self-contained source, no resolver.
    fn compiles_a_self_contained_did() {
        let compilation = compile_did(SELF_CONTAINED).expect("a self-contained source must compile");
        assert!(compilation
            .contract()
            .contract_id()
            .starts_with("candid-core:contract:v1:sha256:"));
        assert_eq!(compilation.source_info().unwrap().sources().len(), 1);
        // An import in an inline source is still a caller error, and the
        // guidance points at the entry point that accepts a bundle.
        let error = compile_did("import \"types.did\"; service : {};")
            .expect_err("an inline source cannot resolve imports");
        assert_eq!(error.diagnostics[0].code, "did_import_requires_file");
        assert!(error.diagnostics[0]
            .message
            .contains("compile_with_resolver"));
    }
}

browser_case! {
    /// What issue #21 exists for: an imported multi-file bundle, with both
    /// import kinds and a diamond, compiled with no filesystem at all.
    fn compiles_an_imported_multi_file_bundle() {
        let compilation = compile_bundle(&RuntimeContext::default())
            .expect("an imported bundle must compile without a filesystem");

        assert_eq!(compilation.contract().contract_id(), BUNDLE_CONTRACT_ID);
        assert_eq!(
            compilation.contract().interface_id(),
            Some(BUNDLE_INTERFACE_ID)
        );

        // The service import contributed its method to the merged actor; the
        // type imports contributed types and no methods.
        let methods: Vec<&str> = match compilation.contract().actor() {
            Some(candid_core::Actor::Service { service }) => {
                match &compilation.contract().types()[*service as usize] {
                    candid_core::TypeNode::Service { methods } => {
                        methods.iter().map(|method| method.name.as_str()).collect()
                    }
                    other => panic!("the actor must select a service node: {other:?}"),
                }
            }
            other => panic!("the bundle declares a service actor: {other:?}"),
        };
        assert!(methods.contains(&"local"), "{methods:?}");
        assert!(methods.contains(&"imported"), "{methods:?}");
        assert_eq!(methods.len(), 2, "{methods:?}");
    }
}

browser_case! {
    /// Provenance is the other half of the semantic output: the exact logical
    /// sources, the exact import edges, and the pinned bundle identity.
    fn imported_bundle_provenance_names_logical_sources_only() {
        let compilation = compile_bundle(&RuntimeContext::default()).expect("must compile");
        let info = compilation.source_info().expect("provenance is on by default");

        assert_eq!(info.source_bundle_id(), BUNDLE_SOURCE_BUNDLE_ID);
        assert_eq!(info.contract_id(), BUNDLE_CONTRACT_ID);

        let sources: Vec<&str> = info
            .sources()
            .iter()
            .map(|source| source.name.as_str())
            .collect();
        assert_eq!(sources, BUNDLE_SOURCES, "the diamond must load once");

        let imports: Vec<(&str, &str, &str)> = info
            .imports()
            .iter()
            .map(|import| {
                (
                    import.from.as_str(),
                    import.import.as_str(),
                    import.to.as_str(),
                )
            })
            .collect();
        assert_eq!(imports, BUNDLE_IMPORTS, "every edge is recorded once");

        // The sidecar authenticates by rederiving its own bundle, which is the
        // same in-memory backend running a second time. It must round-trip.
        let raw: RawSourceInfo =
            serde_json::from_value(serde_json::to_value(info).expect("must serialize"))
                .expect("must decode");
        let authenticated = SourceInfo::try_from_raw(raw, compilation.contract(), &Limits::default())
            .expect("compiler-emitted provenance must authenticate");
        assert_eq!(&authenticated, info);
    }
}

browser_case! {
    /// An invalid imported service: stable code, stable phase, the logical
    /// source that failed, and nothing that looks like a file.
    fn invalid_imported_service_is_structured_and_leaks_nothing() {
        let error = compile_with_resolver(
            "memory:/entry.did",
            &resolver(&[
                ("memory:/entry.did", "import service \"lib.did\";\nservice : {};\n"),
                ("memory:/lib.did", "type T = nat;\n"),
            ]),
            CompileOptions::default(),
            &RuntimeContext::default(),
        )
        .expect_err("an imported service with no main service must be rejected");

        let diagnostic = &error.diagnostics[0];
        assert_eq!(diagnostic.code, "did_type_check_error");
        assert_eq!(diagnostic.phase, Some(DiagnosticPhase::TypeCheck));
        let span = diagnostic.span.as_ref().expect("the failing source is known");
        assert_eq!(span.source_name.as_deref(), Some("memory:/lib.did"));
        assert_eq!(span.start_byte, None, "no offset may be invented");
        assert_no_filesystem_leakage(&error);
    }
}

browser_case! {
    /// A type-check failure inside an imported bundle, and a parse failure in
    /// an imported source. Both keep their phase and their logical source.
    fn imported_type_and_parse_failures_keep_their_phase_and_source() {
        let type_error = compile_with_resolver(
            "memory:/entry.did",
            &resolver(&[
                (
                    "memory:/entry.did",
                    "import \"types.did\";\nservice : { read: () -> (Missing) query };\n",
                ),
                ("memory:/types.did", "type Item = nat;\n"),
            ]),
            CompileOptions::default(),
            &RuntimeContext::default(),
        )
        .expect_err("an unbound imported type must be rejected");
        assert_eq!(type_error.diagnostics[0].code, "did_type_check_error");
        assert_eq!(
            type_error.diagnostics[0].phase,
            Some(DiagnosticPhase::TypeCheck)
        );
        assert_no_filesystem_leakage(&type_error);

        let parse_error = compile_with_resolver(
            "memory:/entry.did",
            &resolver(&[
                (
                    "memory:/entry.did",
                    "import \"types.did\";\nservice : { read: () -> (Item) query };\n",
                ),
                ("memory:/types.did", "type Broken = ;\n"),
            ]),
            CompileOptions::default(),
            &RuntimeContext::default(),
        )
        .expect_err("an unparseable imported source must be rejected");
        let diagnostic = &parse_error.diagnostics[0];
        assert_eq!(diagnostic.code, "did_parse_error");
        assert_eq!(diagnostic.phase, Some(DiagnosticPhase::Parse));
        // Parse offsets index the original text the resolver supplied, so
        // unlike the merge report they are exact and are published.
        let span = diagnostic.span.as_ref().expect("parse errors carry a span");
        assert_eq!(span.source_name.as_deref(), Some("memory:/types.did"));
        assert_eq!((span.start_byte, span.end_byte), (Some(14), Some(15)));
        assert_no_filesystem_leakage(&parse_error);
    }
}

browser_case! {
    /// Resolver failure, resource exhaustion, and cancellation: the three ways
    /// a host stops this work, all structured, none of them a panic.
    fn resolver_resource_and_cancellation_failures_stay_structured() {
        let missing = compile_with_resolver(
            "memory:/entry.did",
            &resolver(&[(
                "memory:/entry.did",
                "import \"absent.did\";\nservice : {};\n",
            )]),
            CompileOptions::default(),
            &RuntimeContext::default(),
        )
        .expect_err("an unresolvable import must be rejected");
        assert_eq!(missing.diagnostics[0].code, "did_source_not_found");
        assert_eq!(missing.diagnostics[0].phase, Some(DiagnosticPhase::Load));
        assert_no_filesystem_leakage(&missing);

        let exhausted = compile_bundle(&RuntimeContext::new(
            Limits::default().with_max_sources(BUNDLE.len() - 1),
        ))
        .expect_err("a bundle over the source count must be rejected");
        let diagnostic = &exhausted.diagnostics[0];
        assert_eq!(diagnostic.code, "resource_limit_exceeded");
        let resource = diagnostic
            .resource_limit
            .as_ref()
            .expect("the resource triple must survive");
        assert_eq!(resource.resource, "sources");
        assert_eq!(resource.limit, (BUNDLE.len() - 1) as u64);
        assert_eq!(resource.observed, BUNDLE.len() as u64);

        let token = CancellationToken::new();
        token.cancel();
        let cancelled = compile_bundle(
            &RuntimeContext::new(Limits::default()).with_cancellation(token),
        )
        .expect_err("a cancelled operation must not produce an artifact");
        assert_eq!(cancelled.diagnostics[0].code, "operation_cancelled");
    }
}

/// Compile [`SELF_CONTAINED`] inline under `limits`, so the deadline case can
/// check the entry point that takes no resolver as well as the one that does.
fn compile_inline(limits: &Limits) -> Result<candid_core::Compilation, CompileError> {
    candid_core::compile_did_with_context(
        SELF_CONTAINED,
        CompileOptions::default(),
        &RuntimeContext::new(limits.clone()),
    )
}

browser_case! {
    /// The deadline boundary, which is target-specific by design.
    ///
    /// Bare WASM has no clock. An explicit deadline cannot be measured there,
    /// so it fails closed through the ordinary `operation_deadline_exceeded`
    /// result rather than calling an unsupported `std::time` function and
    /// aborting the module. Natively the same deadline is measured: an elapsed
    /// one fails and a future one lets the work finish. Both sides agree that
    /// no deadline is unbounded and that neither ever panics.
    ///
    /// No case here hard-codes a calendar date. A literal "far future"
    /// timestamp is a test that expires: it silently becomes an elapsed
    /// deadline on some later date and starts asserting the opposite of what
    /// it was written for. The native branch derives its future deadline from
    /// the clock this very run reads, and the bare-WASM branch needs no clock
    /// at all because on that target the value cannot matter.
    fn explicit_deadlines_fail_closed_without_a_clock() {
        // Shared on every target: an unset deadline bounds nothing.
        compile_bundle(&RuntimeContext::new(Limits::default()))
            .expect("no deadline must not bound anything");
        compile_inline(&Limits::default()).expect("nor for the inline entry point");
        assert!(!Limits::default().deadline_exceeded());

        // Shared on every target, and clock-free: `1` is one millisecond after
        // the Unix epoch, which is in the past for any real native clock and
        // is simply "an explicit deadline" on a target with none. Both
        // compilation entry points must fail closed, so no path reaches a
        // clock call on bare WASM.
        let elapsed = Limits::default().with_deadline_unix_ms(Some(1));
        let error = compile_bundle(&RuntimeContext::new(elapsed.clone()))
            .expect_err("an elapsed deadline must fail closed");
        assert_eq!(
            error.diagnostics[0].code, "operation_deadline_exceeded",
            "{:#?}",
            error.diagnostics
        );
        assert_eq!(error.diagnostics[0].phase, Some(DiagnosticPhase::Load));
        let inline = compile_inline(&elapsed).expect_err("inline must fail closed too");
        assert_eq!(inline.diagnostics[0].code, "operation_deadline_exceeded");
        assert!(
            elapsed.deadline_exceeded(),
            "the public predicate must agree with the budget"
        );

        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            // No clock: *every* explicit deadline fails closed, whatever its
            // value and whatever the date, and none of them panics. Nothing
            // here can expire, because nothing here is compared to a clock.
            for deadline_unix_ms in [0, 1, 2_000_000_000_000, u64::MAX] {
                let limits = Limits::default().with_deadline_unix_ms(Some(deadline_unix_ms));
                let error = compile_bundle(&RuntimeContext::new(limits.clone()))
                    .unwrap_err();
                assert_eq!(
                    error.diagnostics[0].code, "operation_deadline_exceeded",
                    "deadline {deadline_unix_ms} must fail closed: {:#?}",
                    error.diagnostics
                );
                assert_eq!(error.diagnostics[0].phase, Some(DiagnosticPhase::Load));
                assert_eq!(
                    compile_inline(&limits).unwrap_err().diagnostics[0].code,
                    "operation_deadline_exceeded"
                );
                assert!(
                    limits.deadline_exceeded(),
                    "deadline {deadline_unix_ms} must read as elapsed"
                );
            }
        }

        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            // Measured natively, so a genuinely future deadline is built from
            // the current time rather than written down. Ten minutes is long
            // enough that compiling a four-source bundle cannot reach it and
            // short enough to stay far inside every platform's monotonic
            // range.
            const TEN_MINUTES_MS: u64 = 10 * 60 * 1_000;
            let now_unix_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|since_epoch| u64::try_from(since_epoch.as_millis()).ok())
                .expect("a native host's clock must be representable since the Unix epoch");
            let future = Limits::default().with_deadline_unix_ms(Some(
                now_unix_ms
                    .checked_add(TEN_MINUTES_MS)
                    .expect("ten minutes past now must be representable"),
            ));

            compile_bundle(&RuntimeContext::new(future.clone()))
                .expect("a deadline ten minutes out is measurable and must not fire");
            compile_inline(&future).expect("nor for the inline entry point");
            assert!(!future.deadline_exceeded());
        }
    }
}

browser_case! {
    /// The filesystem is genuinely absent, not merely unused: a `memory:/`
    /// bundle compiles while a logical ID from another scheme is rejected by
    /// `MemoryResolver` itself rather than reaching a host path.
    fn no_native_path_is_ever_consulted() {
        assert!(SourceId::parse("memory:/entry.did").is_ok());
        let mut resolver = resolver(BUNDLE);
        let error = resolver
            .insert("workspace:/entry.did", "service : {};")
            .expect_err("MemoryResolver only owns memory:/ sources");
        assert_eq!(error.code, "did_source_scheme_mismatch");
    }
}
