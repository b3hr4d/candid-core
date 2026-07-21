//! Boundary, precedence, and determinism coverage for source-bundle identity
//! work accounting (`source_identity_work`).
//!
//! One identity pass serializes the `{sources, imports}` payload once to count
//! its bytes and once to materialize and hash it, so its exact cost is
//! `3 * serialized_len + domain_tag + 1`. A compilation performs one pass; a
//! presented-sidecar validation performs two (verifying the presented ID, then
//! re-emitting it during rederivation) on one budget. Every boundary below is
//! computed from that closed form, so a pass that stopped charging — or
//! charged twice — would move the boundary and fail these tests.

use candid_core::{
    compile_with_resolver, CancellationToken, Compilation, CompileOptions, Limits, MemoryResolver,
    RawContract, RawSourceInfo, RuntimeContext, SourceInfo,
};

const ROOT: &str = "import \"types.did\";\nservice : { read: (id: nat) -> (Item) query };";
const TYPES: &str = "type Item = record { id: nat; label: text };";
const DOMAIN: &str = "candid-core:source-bundle:v1";

/// Golden identity of the fixture bundle, pinned across releases.
///
/// Every other stability assertion in the repository compares two computations
/// from the same build, which a serialization change would move in lockstep.
/// This literal is the cross-release anchor: it moves only if the payload
/// shape, canonical ordering, domain tag, or hash construction changes — all
/// of which are frozen by ADR 0002 and issue #67's non-goals.
const GOLDEN_BUNDLE_ID: &str =
    "candid-core:source-bundle:v1:sha256:252f44cb868b8f6c3867e9e2fd5261c74163f9f652f29727048f5c986a55b3a4";

fn resolver() -> MemoryResolver {
    let mut resolver = MemoryResolver::new();
    resolver.insert("root.did", ROOT).unwrap();
    resolver.insert("types.did", TYPES).unwrap();
    resolver
}

fn bundle() -> Compilation {
    compile_with_resolver(
        "root.did",
        &resolver(),
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap()
}

fn raw_of(compilation: &Compilation) -> RawSourceInfo {
    serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
        .unwrap()
}

/// Exact `source_identity_work` cost of one identity pass over this sidecar.
fn identity_pass_work(info: &SourceInfo) -> usize {
    let sources = serde_json::to_string(info.sources()).unwrap();
    let imports = serde_json::to_string(info.imports()).unwrap();
    let serialized_len = r#"{"sources":,"imports":}"#.len() + sources.len() + imports.len();
    serialized_len * 3 + DOMAIN.len() + 1
}

fn accept(raw: RawSourceInfo, compilation: &Compilation, limits: Limits) {
    SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::new(limits),
    )
    .unwrap_or_else(|error| panic!("expected acceptance at the exact limit: {error:#?}"));
}

/// Returns `(resource, limit, observed)` from the first violation.
fn reject(raw: RawSourceInfo, compilation: &Compilation, limits: Limits) -> (String, usize, usize) {
    let error = SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::new(limits),
    )
    .expect_err("an exhausted identity budget must be rejected");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded", "{error:#?}");
    let info = violation
        .resource_limit
        .as_ref()
        .expect("resource limit failures must retain metadata");
    (info.resource.clone(), info.limit, info.observed)
}

#[test]
fn compile_charges_one_identity_pass_at_the_exact_boundary() {
    let baseline = bundle();
    let work = identity_pass_work(baseline.source_info().unwrap());

    let at_limit = compile_with_resolver(
        "root.did",
        &resolver(),
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_identity_work: work,
            ..Limits::default()
        }),
    )
    .unwrap_or_else(|error| panic!("the exact identity budget must compile: {error:#?}"));
    assert_eq!(
        at_limit.source_info().unwrap().source_bundle_id(),
        GOLDEN_BUNDLE_ID,
        "metering must not change the identity bytes",
    );
    assert_eq!(
        baseline.source_info().unwrap().source_bundle_id(),
        GOLDEN_BUNDLE_ID,
        "the unmetered baseline must match the pinned golden identity",
    );

    let error = compile_with_resolver(
        "root.did",
        &resolver(),
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_identity_work: work - 1,
            ..Limits::default()
        }),
    )
    .expect_err("one unit under the identity budget must fail during compilation");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_identity_work");
    assert_eq!(resource.limit, work - 1);
    assert_eq!(resource.observed, work);
}

#[test]
fn validation_charges_exactly_two_identity_passes_at_the_exact_boundary() {
    // Pass one verifies the presented `source_bundle_id`; pass two re-emits it
    // while rederiving the bundle on the same budget. Acceptance at exactly
    // twice the per-pass cost proves both passes are charged once each — a
    // third hidden pass would exceed the budget, and a skipped one would let
    // the one-under case succeed.
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let per_pass = identity_pass_work(compilation.source_info().unwrap());
    let exact = per_pass * 2;

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_source_identity_work: exact,
            ..Limits::default()
        },
    );
    assert_eq!(
        reject(
            raw,
            &compilation,
            Limits {
                max_source_identity_work: exact - 1,
                ..Limits::default()
            },
        ),
        ("source_identity_work".to_string(), exact - 1, exact)
    );
}

#[test]
fn compilation_sidecar_parse_charges_the_same_two_passes() {
    // `Compilation::try_from_raw` remaps and then validates the sidecar on one
    // budget; its identity accounting must match the direct validation path.
    let compilation = bundle();
    let per_pass = identity_pass_work(compilation.source_info().unwrap());
    let exact = per_pass * 2;

    Compilation::try_from_raw_with_context(
        RawContract::from(compilation.contract()),
        Some(raw_of(&compilation)),
        &RuntimeContext::new(Limits {
            max_source_identity_work: exact,
            ..Limits::default()
        }),
    )
    .unwrap_or_else(|error| panic!("expected acceptance at the exact limit: {error:#?}"));

    let error = Compilation::try_from_raw_with_context(
        RawContract::from(compilation.contract()),
        Some(raw_of(&compilation)),
        &RuntimeContext::new(Limits {
            max_source_identity_work: exact - 1,
            ..Limits::default()
        }),
    )
    .expect_err("one unit under the identity budget must be rejected");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    let info = violation.resource_limit.as_ref().unwrap();
    assert_eq!(
        (info.resource.as_str(), info.limit, info.observed),
        ("source_identity_work", exact - 1, exact)
    );
}

#[test]
fn repeated_validation_accounts_identity_work_deterministically() {
    // Every operation gets a fresh budget, so repeating the same validation
    // must neither accumulate work across calls nor drift in the observed
    // value it reports.
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let exact = identity_pass_work(compilation.source_info().unwrap()) * 2;

    for _ in 0..2 {
        accept(
            raw.clone(),
            &compilation,
            Limits {
                max_source_identity_work: exact,
                ..Limits::default()
            },
        );
    }
    let starved = Limits {
        max_source_identity_work: exact - 1,
        ..Limits::default()
    };
    let first = reject(raw.clone(), &compilation, starved.clone());
    let second = reject(raw, &compilation, starved);
    assert_eq!(first, second);
    assert_eq!(
        first,
        ("source_identity_work".to_string(), exact - 1, exact)
    );
}

#[test]
fn identity_counting_pass_interrupts_mid_serialization() {
    // A limit below the serialized payload length must fail while the
    // streaming counting pass is still running: the observed value stays at
    // most the serialized length, far below the full `3 * len` reservation.
    // Cancellation and deadlines share the same per-chunk checkpoint, so this
    // pins the supported mid-pass granularity.
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let per_pass = identity_pass_work(compilation.source_info().unwrap());
    let serialized_len = (per_pass - DOMAIN.len() - 1) / 3;

    let (resource, limit, observed) = reject(
        raw,
        &compilation,
        Limits {
            max_source_identity_work: serialized_len - 1,
            ..Limits::default()
        },
    );
    assert_eq!(resource, "source_identity_work");
    assert_eq!(limit, serialized_len - 1);
    assert!(
        observed <= serialized_len,
        "exhaustion must be observed during the counting pass, \
         not after the whole reservation: observed {observed} of {per_pass}",
    );
}

#[test]
fn default_budget_covers_worst_case_default_valid_bundles() {
    // The default must accept every bundle that is valid under the default
    // byte and count limits, including JSON serialization overhead. A byte
    // expands to at most six (`\uXXXX`) when escaped, and every identity
    // string is bounded before hashing: by `source_string_bytes` on the
    // presented-sidecar path, and per resolved name / import spelling by
    // `source_id_bytes` on the compile path.
    let defaults = Limits::default();
    const MAX_ESCAPE: usize = 6;
    let domain_overhead = DOMAIN.len() + 1;
    let shell = r#"{"sources":[],"imports":[]}"#.len();
    // Per-entry JSON overhead (longest `kind` variant) plus a separating comma.
    let source_overhead = r#"{"name":"","source":""}"#.len() + 1;
    let import_overhead = r#"{"from":"","import":"","to":"","kind":"service"}"#.len() + 1;
    let structural = shell
        + defaults.max_sources * source_overhead
        + defaults.max_import_edges * import_overhead;

    // Presented-sidecar validation: two identity passes on one budget.
    let validation_pass =
        MAX_ESCAPE * (defaults.max_bundle_bytes + defaults.max_string_bytes) + structural;
    let validation_work = 2 * (3 * validation_pass + domain_overhead);
    assert!(
        validation_work <= defaults.max_source_identity_work,
        "two default-valid validation passes ({validation_work}) must fit the default budget",
    );

    // Compilation: one identity pass; names and import spellings are each
    // bounded by `max_source_id_bytes` rather than the aggregate string limit.
    let compile_strings = defaults.max_sources * defaults.max_source_id_bytes
        + 3 * defaults.max_import_edges * defaults.max_source_id_bytes;
    let compile_pass = MAX_ESCAPE * (defaults.max_bundle_bytes + compile_strings) + structural;
    let compile_work = 3 * compile_pass + domain_overhead;
    assert!(
        compile_work <= defaults.max_source_identity_work,
        "a default-valid compile pass ({compile_work}) must fit the default budget",
    );
}

#[test]
fn zero_identity_budget_fails_closed_with_stable_metadata() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let (resource, limit, observed) = reject(
        raw,
        &compilation,
        Limits {
            max_source_identity_work: 0,
            ..Limits::default()
        },
    );
    assert_eq!(resource, "source_identity_work");
    assert_eq!(limit, 0);
    assert!(observed > 0, "the first serialized chunk must be charged");

    let error = compile_with_resolver(
        "root.did",
        &resolver(),
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_identity_work: 0,
            ..Limits::default()
        }),
    )
    .expect_err("a zero identity budget must fail compilation");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    assert_eq!(
        diagnostic.resource_limit.as_ref().unwrap().resource,
        "source_identity_work"
    );
}

#[test]
fn preexisting_errors_keep_precedence_over_identity_work() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let starved = |limits: Limits| Limits {
        max_source_identity_work: 1,
        ..limits
    };

    // A header mismatch keeps its error: the header is validated before the
    // preflight and every identity charge.
    let mut mismatched = raw.clone();
    mismatched.contract_id.push('0');
    let error = SourceInfo::try_from_raw_with_context(
        mismatched,
        compilation.contract(),
        &RuntimeContext::new(starved(Limits::default())),
    )
    .expect_err("a mismatched contract_id must be rejected");
    assert_eq!(error.violations[0].code, "source_contract_id_mismatch");

    // A collection over its count limit keeps that resource: the preflight
    // count checks run before any identity work is charged.
    let (resource, limit, observed) = reject(
        raw.clone(),
        &compilation,
        starved(Limits {
            max_sources: 1,
            ..Limits::default()
        }),
    );
    assert_eq!(
        (resource.as_str(), limit, observed),
        ("sources", 1, raw.sources.len())
    );

    // An oversized bundle keeps its byte-limit error: the preflight runs
    // before any identity work is charged.
    let bundle_bytes: usize = raw.sources.iter().map(|source| source.source.len()).sum();
    let (resource, _, observed) = reject(
        raw.clone(),
        &compilation,
        starved(Limits {
            max_bundle_bytes: bundle_bytes - 1,
            ..Limits::default()
        }),
    );
    assert_eq!(resource, "bundle_bytes");
    assert_eq!(observed, bundle_bytes);

    // A malformed logical source ID is rejected before the bundle is hashed.
    let mut malformed = raw.clone();
    malformed.sources[0].name = "memory:/bad\\name.did".to_string();
    let error = SourceInfo::try_from_raw_with_context(
        malformed,
        compilation.contract(),
        &RuntimeContext::new(starved(Limits::default())),
    )
    .expect_err("a malformed source ID must be rejected");
    assert_eq!(error.violations[0].code, "invalid_source_id");

    // A non-canonical bundle keeps its ordering error: canonicality is checked
    // before the hash pass charges anything.
    let mut unsorted = raw;
    unsorted.sources.reverse();
    let error = SourceInfo::try_from_raw_with_context(
        unsorted,
        compilation.contract(),
        &RuntimeContext::new(starved(Limits::default())),
    )
    .expect_err("a non-canonical bundle must be rejected");
    assert_eq!(error.violations[0].code, "non_canonical_source_bundle");
}

#[test]
fn tampered_bundle_id_is_rejected_whenever_the_hash_can_complete() {
    // With enough budget for the verification pass, a tampered ID keeps its
    // established mismatch error. One unit under it, the deterministic
    // resource failure precedes the mismatch — the mismatch is only
    // observable after hashing, which is exactly the work being metered.
    let compilation = bundle();
    let per_pass = identity_pass_work(compilation.source_info().unwrap());
    let mut tampered = raw_of(&compilation);
    tampered.source_bundle_id.push('0');

    let error = SourceInfo::try_from_raw_with_context(
        tampered.clone(),
        compilation.contract(),
        &RuntimeContext::new(Limits {
            max_source_identity_work: per_pass,
            ..Limits::default()
        }),
    )
    .expect_err("a tampered source_bundle_id must be rejected");
    assert_eq!(error.violations[0].code, "source_bundle_id_mismatch");

    assert_eq!(
        reject(
            tampered,
            &compilation,
            Limits {
                max_source_identity_work: per_pass - 1,
                ..Limits::default()
            },
        ),
        ("source_identity_work".to_string(), per_pass - 1, per_pass)
    );
}

#[test]
fn identity_validation_observes_cancellation_and_deadlines() {
    // This pins the operation surface: a cancelled or expired context aborts
    // sidecar validation with the stable codes. The identity-stage boundary
    // itself — including cancellation between serializer chunks mid-pass — is
    // pinned deterministically by the unit tests in `src/source.rs` and
    // `src/canonical.rs`, which drive the identity seam directly.
    let compilation = bundle();
    let raw = raw_of(&compilation);

    let token = CancellationToken::new();
    token.cancel();
    let context = RuntimeContext::new(Limits::default()).with_cancellation(token);
    let error =
        SourceInfo::try_from_raw_with_context(raw.clone(), compilation.contract(), &context)
            .expect_err("cancellation must abort sidecar validation");
    assert_eq!(error.violations[0].code, "operation_cancelled");

    // `Some(1)` elapsed in 1970; deriving "now minus one" from a second
    // wall-clock read can race a backwards clock step.
    let error = SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::new(Limits {
            deadline_unix_ms: Some(1),
            ..Limits::default()
        }),
    )
    .expect_err("an elapsed deadline must abort sidecar validation");
    assert_eq!(error.violations[0].code, "operation_deadline_exceeded");
}
