//! Boundary and adversarial coverage for bounded parse and serialization.
//!
//! Every trust-boundary parse entry point is exercised at exactly its
//! `max_input_bytes` limit (accepted) and one byte over it (rejected with
//! stable resource metadata). The one-over cases assert that the byte gate
//! fires *before* the document is decoded and before any structural limit is
//! consulted, so an oversized document is never materialized.
//!
//! These tests also pin two properties that have no coverage elsewhere: that
//! decode and validation share a single budget rather than each receiving a
//! fresh allowance, and that serialization consumes `max_canonicalization_work`
//! in addition to whichever structural limit gated construction.

use candid_core::{
    compile_did, Compilation, CompileOptions, Contract, ContractEnvelope, ContractJsonError,
    Limits, RuntimeContext,
};
// Only the two-source `Compilation` bundle needs the materializing resolver
// path; the Contract and envelope entry points are `compiler`-level.
#[cfg(feature = "filesystem-compiler")]
use candid_core::{compile_with_resolver, MemoryResolver, RawContract};

#[cfg(feature = "filesystem-compiler")]
const ROOT: &str = r#"import "types.did";
/// Root service documentation.
service : {
  /// Ping documentation.
  ping: (name: text, tag: nat) -> (item: Item) query;
};"#;

#[cfg(feature = "filesystem-compiler")]
const TYPES: &str = r#"/// Item documentation.
type Item = record {
  /// Identifier documentation.
  id: nat;
  /// Label documentation.
  label: text;
};"#;

#[cfg(feature = "filesystem-compiler")]
/// A two-source bundle that populates every provenance collection.
fn bundle() -> Compilation {
    let mut resolver = MemoryResolver::new();
    resolver.insert("root.did", ROOT).unwrap();
    resolver.insert("types.did", TYPES).unwrap();
    compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions {
            include_source_info: true,
        },
        &RuntimeContext::default(),
    )
    .expect("bundle must compile")
}

fn contract_json() -> String {
    compile_did("type Item = record { id: nat }; service : { ping: () -> () query };")
        .expect("must compile")
        .contract()
        .to_json_pretty()
        .expect("must serialize")
}

#[cfg(feature = "filesystem-compiler")]
fn compilation_json() -> String {
    serde_json::to_string_pretty(&bundle()).expect("must serialize")
}

fn envelope_json() -> String {
    let contract = compile_did("service : { ping: () -> () query };")
        .expect("must compile")
        .contract()
        .clone();
    serde_json::to_string_pretty(&ContractEnvelope::new(contract)).expect("must serialize")
}

fn limits_with_input_bytes(max_input_bytes: usize) -> Limits {
    Limits::default().with_max_input_bytes(max_input_bytes)
}

/// Returns `(resource, limit, observed)` from the first violation.
fn resource_metadata(error: ContractJsonError) -> (String, u64, u64) {
    let ContractJsonError::InvalidContract(error) = error else {
        panic!("oversized input must fail validation, not JSON syntax: {error:?}");
    };
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded", "{error:#?}");
    let info = violation
        .resource_limit
        .as_ref()
        .expect("resource limit failures must retain metadata");
    (info.resource.clone(), info.limit, info.observed)
}

#[test]
fn contract_json_is_accepted_at_the_limit_and_rejected_one_over() {
    let json = contract_json();
    let exact = json.len();

    Contract::from_json_with_limits(&json, &limits_with_input_bytes(exact))
        .expect("input of exactly max_input_bytes must be accepted");

    let error = Contract::from_json_with_limits(&json, &limits_with_input_bytes(exact - 1))
        .expect_err("input one byte over max_input_bytes must be rejected");
    assert_eq!(
        resource_metadata(error),
        ("input_bytes".to_string(), (exact - 1) as u64, exact as u64)
    );
}

#[test]
fn contract_slice_parsing_matches_the_string_entry_point() {
    let json = contract_json();
    let exact = json.len();

    let from_str = Contract::from_json_with_limits(&json, &limits_with_input_bytes(exact)).unwrap();
    let from_slice =
        Contract::from_slice_with_limits(json.as_bytes(), &limits_with_input_bytes(exact)).unwrap();
    assert_eq!(from_str.contract_id(), from_slice.contract_id());

    let error =
        Contract::from_slice_with_limits(json.as_bytes(), &limits_with_input_bytes(exact - 1))
            .expect_err("the byte entry point must enforce the same gate");
    assert_eq!(
        resource_metadata(error),
        ("input_bytes".to_string(), (exact - 1) as u64, exact as u64)
    );
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn compilation_json_is_accepted_at_the_limit_and_rejected_one_over() {
    let json = compilation_json();
    let exact = json.len();

    Compilation::from_json_with_limits(&json, &limits_with_input_bytes(exact))
        .expect("input of exactly max_input_bytes must be accepted");

    let error = Compilation::from_json_with_limits(&json, &limits_with_input_bytes(exact - 1))
        .expect_err("input one byte over max_input_bytes must be rejected");
    assert_eq!(
        resource_metadata(error),
        ("input_bytes".to_string(), (exact - 1) as u64, exact as u64)
    );
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn compilation_round_trips_through_its_bounded_parse_entry_point() {
    let original = bundle();
    let json = compilation_json();
    let parsed = Compilation::from_json_with_limits(&json, &Limits::default())
        .expect("a compilation this crate produced must parse back");

    assert_eq!(
        parsed.contract().contract_id(),
        original.contract().contract_id()
    );
    assert_eq!(
        parsed.source_info().map(|info| info.source_bundle_id()),
        original.source_info().map(|info| info.source_bundle_id()),
    );
}

#[test]
fn envelope_json_is_accepted_at_the_limit_and_rejected_one_over() {
    let json = envelope_json();
    let exact = json.len();

    ContractEnvelope::from_json_with_limits(&json, &limits_with_input_bytes(exact))
        .expect("input of exactly max_input_bytes must be accepted");

    let error = ContractEnvelope::from_json_with_limits(&json, &limits_with_input_bytes(exact - 1))
        .expect_err("input one byte over max_input_bytes must be rejected");
    assert_eq!(
        resource_metadata(error),
        ("input_bytes".to_string(), (exact - 1) as u64, exact as u64)
    );
}

#[test]
fn envelope_parsing_preserves_extensions_and_contract_identity() {
    let contract = compile_did("service : { ping: () -> () query };")
        .unwrap()
        .contract()
        .clone();
    let mut envelope = ContractEnvelope::new(contract.clone());
    envelope
        .insert_extension(
            "com.example.form/v1",
            serde_json::json!({ "widget": "button" }),
            &Limits::default(),
        )
        .unwrap();

    let json = envelope
        .to_json_pretty_with_limits(&Limits::default())
        .unwrap();
    let parsed = ContractEnvelope::from_json_with_limits(&json, &Limits::default()).unwrap();

    assert_eq!(parsed.contract().contract_id(), contract.contract_id());
    assert_eq!(parsed.extensions().len(), 1);
    assert_eq!(
        parsed.extensions()["com.example.form/v1"],
        serde_json::json!({ "widget": "button" })
    );
}

#[test]
fn oversized_input_is_rejected_before_structural_limits_are_consulted() {
    // The document exceeds `max_input_bytes` *and* `max_type_nodes`. The byte
    // gate runs before decoding, so `input_bytes` must be the reported
    // resource — structural limits are only reachable after materialization.
    let json = contract_json();
    let limits = Limits::default()
        .with_max_input_bytes(json.len() - 1)
        .with_max_type_nodes(1);

    let error = Contract::from_json_with_limits(&json, &limits)
        .expect_err("an oversized document must be rejected");
    let (resource, _, _) = resource_metadata(error);
    assert_eq!(
        resource, "input_bytes",
        "the byte gate must fire before any structural limit is consulted"
    );
}

/// Smallest `max_canonicalization_work` under which `parse` succeeds.
///
/// `canonicalization_work` is charged cumulatively on a single budget, so this
/// is a direct proxy for how many validation passes an entry point performs.
fn minimum_canonicalization_work(parse: impl Fn(&Limits) -> bool) -> usize {
    let ceiling = Limits::default().max_canonicalization_work();
    assert!(
        parse(&Limits::default()),
        "the probe must succeed at the default ceiling"
    );
    let (mut low, mut high) = (0usize, ceiling);
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = Limits::default().with_max_canonicalization_work(mid);
        if parse(&limits) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

#[test]
fn envelope_parsing_validates_the_nested_contract_exactly_once() {
    // Before bounded parsing, `RawContractEnvelope.contract` was typed
    // `Contract`, so serde validated and canonicalized the nested Contract on
    // its own default-limited budget and the envelope then revalidated it on a
    // second, independent one. Two passes, two full allowances.
    //
    // Retyping that field to `RawContract` collapses it to a single pass on
    // the envelope's own budget. Cumulative `canonicalization_work` makes that
    // directly observable: a second pass would roughly double the minimum.
    let contract_json = contract_json();
    let contract = Contract::from_json(&contract_json).unwrap();
    let envelope_json = ContractEnvelope::new(contract)
        .to_json_pretty_with_limits(&Limits::default())
        .unwrap();

    let contract_cost = minimum_canonicalization_work(|limits| {
        Contract::from_json_with_limits(&contract_json, limits).is_ok()
    });
    let envelope_cost = minimum_canonicalization_work(|limits| {
        ContractEnvelope::from_json_with_limits(&envelope_json, limits).is_ok()
    });

    assert!(
        contract_cost > 0,
        "the probe is meaningless if validation charges nothing"
    );
    // Measured: both cost 2256 exactly — the envelope adds no second pass.
    // Mutation-checked: reintroducing a second `validate_contract_with_budget`
    // call on the shared budget makes this fail.
    assert!(
        envelope_cost * 2 <= contract_cost * 3,
        "envelope parsing cost {envelope_cost} is disproportionate to the bare contract cost \
         {contract_cost}, which means the nested Contract is being validated twice"
    );
    assert!(
        envelope_cost >= contract_cost,
        "envelope parsing cost {envelope_cost} is below the bare contract cost {contract_cost}, \
         so the envelope is doing strictly less work than validating its own Contract"
    );

    // What this test does NOT pin, stated so nobody assumes otherwise: it
    // cannot detect the nested Contract escaping onto a *separate* budget.
    // `validate_extensions_with_budget` charges no `canonicalization_work`, so
    // one shared budget and two independent ones need the same minimum. Budget
    // sharing here is currently unobservable from outside the crate; the
    // single-pass property above is what is actually enforced.
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn every_bounded_entry_point_reports_the_same_input_bytes_metadata() {
    // One helper emits `input_bytes` for all three types. If any entry point
    // grew its own hand-rolled gate, its metadata would drift from the others.
    fn expect_one_over(name: &str, json: &str, error: ContractJsonError) {
        let exact = json.len();
        assert_eq!(
            resource_metadata(error),
            ("input_bytes".to_string(), (exact - 1) as u64, exact as u64),
            "{name} must report the observed length exactly, not a truncated or clamped value"
        );
    }

    let contract = contract_json();
    let under = limits_with_input_bytes(contract.len() - 1);
    expect_one_over(
        "Contract",
        &contract,
        Contract::from_json_with_limits(&contract, &under).expect_err("must reject"),
    );

    let compilation = compilation_json();
    let under = limits_with_input_bytes(compilation.len() - 1);
    expect_one_over(
        "Compilation",
        &compilation,
        Compilation::from_json_with_limits(&compilation, &under).expect_err("must reject"),
    );

    let envelope = envelope_json();
    let under = limits_with_input_bytes(envelope.len() - 1);
    expect_one_over(
        "ContractEnvelope",
        &envelope,
        ContractEnvelope::from_json_with_limits(&envelope, &under).expect_err("must reject"),
    );
}

#[test]
fn raised_limits_round_trip_and_pin_the_serialization_coupling() {
    // Build a Contract whose declaration name exceeds the default string
    // budget, then show that raising only the limit that gated *construction*
    // is not sufficient to *serialize* it: the rendered length is additionally
    // charged against `max_canonicalization_work`.
    let long_name = "A".repeat(2 * 1024 * 1024);
    let source = format!("type {long_name} = nat; service : {{}};");

    let construction_limits = Limits::default()
        .with_max_input_bytes(64 * 1024 * 1024)
        .with_max_source_bytes(8 * 1024 * 1024)
        .with_max_bundle_bytes(64 * 1024 * 1024)
        .with_max_string_bytes(8 * 1024 * 1024);
    let compilation = compile_did_with_limits(&source, &construction_limits);
    let contract = compilation.contract().clone();

    // Default limits cannot render it at all.
    let error = contract
        .to_json_pretty()
        .expect_err("the default string budget must reject a 2 MiB declaration name");
    assert_eq!(
        error.violations[0]
            .resource_limit
            .as_ref()
            .expect("metadata must survive")
            .resource,
        "string_bytes"
    );

    // Raising only the construction limit is still not enough.
    let error = contract
        .to_json_pretty_with_limits(&construction_limits)
        .expect_err("the rendered length is charged against max_canonicalization_work");
    assert_eq!(
        error.violations[0]
            .resource_limit
            .as_ref()
            .expect("metadata must survive")
            .resource,
        "canonicalization_work",
        "serialization consumes a limit construction never touched"
    );

    // Raising both succeeds, and the result parses back to the same identity.
    let serialization_limits = construction_limits
        .clone()
        .with_max_canonicalization_work(100_000_000);
    let json = contract
        .to_json_pretty_with_limits(&serialization_limits)
        .expect("raising both limits must render");
    let reparsed = Contract::from_json_with_limits(&json, &serialization_limits)
        .expect("a document rendered under raised limits must parse back under them");
    assert_eq!(reparsed.contract_id(), contract.contract_id());
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn compilation_serialization_charges_the_rendered_length_on_top_of_validation() {
    // Isolating the render charge needs an independent measurement of the
    // validation-only cost. Asserting merely that a starved limit fails is not
    // enough: validation charges `canonicalization_work` itself, so that test
    // still passes with the render charge deleted. Exact equality does pin it.
    let compilation = bundle();
    let rendered = compilation
        .to_json_pretty_with_limits(&Limits::default())
        .expect("default limits must render this bundle");

    // `to_json_pretty_with_context` validates via `validate_contract_with_budget`,
    // which is exactly what `Contract::validate_with_limits` runs.
    let validation_only = minimum_canonicalization_work(|limits| {
        compilation.contract().validate_with_limits(limits).is_ok()
    });
    let with_render = minimum_canonicalization_work(|limits| {
        compilation.to_json_pretty_with_limits(limits).is_ok()
    });

    assert_eq!(
        with_render,
        validation_only + rendered.len(),
        "serialization must charge exactly the rendered byte length on top of validation"
    );

    // A starved structural limit proves validation runs before rendering.
    let unvalidatable = Limits::default().with_max_type_nodes(1);
    let error = compilation
        .to_json_pretty_with_limits(&unvalidatable)
        .expect_err("serialization must validate before rendering");
    assert_eq!(error.violations[0].code, "resource_limit_exceeded");

    // The happy path round-trips through the bounded parser.
    let parsed = Compilation::from_json_with_limits(&rendered, &Limits::default()).unwrap();
    assert_eq!(
        parsed.contract().contract_id(),
        compilation.contract().contract_id()
    );
}

#[test]
fn envelope_serialization_charges_the_rendered_length_on_top_of_validation() {
    let contract = compile_did("service : { ping: () -> () query };")
        .unwrap()
        .contract()
        .clone();
    let envelope = ContractEnvelope::new(contract);
    let rendered = envelope
        .to_json_pretty_with_limits(&Limits::default())
        .expect("default limits must render this envelope");

    // `validate` runs the same two steps the serializer does, without the
    // render charge — so the difference is exactly the rendered length.
    let validation_only = minimum_canonicalization_work(|limits| envelope.validate(limits).is_ok());
    let with_render =
        minimum_canonicalization_work(|limits| envelope.to_json_pretty_with_limits(limits).is_ok());

    assert_eq!(
        with_render,
        validation_only + rendered.len(),
        "serialization must charge exactly the rendered byte length on top of validation"
    );

    let unvalidatable = Limits::default().with_max_type_nodes(1);
    let error = envelope
        .to_json_pretty_with_limits(&unvalidatable)
        .expect_err("serialization must validate before rendering");
    assert_eq!(error.violations[0].code, "resource_limit_exceeded");
}

#[test]
fn envelope_slice_parsing_matches_the_string_entry_point() {
    let json = envelope_json();
    let exact = json.len();

    ContractEnvelope::from_slice_with_limits(json.as_bytes(), &limits_with_input_bytes(exact))
        .expect("the byte entry point must accept at the exact limit");

    let error = ContractEnvelope::from_slice_with_limits(
        json.as_bytes(),
        &limits_with_input_bytes(exact - 1),
    )
    .expect_err("the byte entry point must enforce the same gate");
    assert_eq!(
        resource_metadata(error),
        ("input_bytes".to_string(), (exact - 1) as u64, exact as u64)
    );
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn compilation_slice_parsing_matches_the_string_entry_point() {
    let json = compilation_json();
    let exact = json.len();

    Compilation::from_slice_with_limits(json.as_bytes(), &limits_with_input_bytes(exact))
        .expect("the byte entry point must accept at the exact limit");

    let error =
        Compilation::from_slice_with_limits(json.as_bytes(), &limits_with_input_bytes(exact - 1))
            .expect_err("the byte entry point must enforce the same gate");
    assert_eq!(
        resource_metadata(error),
        ("input_bytes".to_string(), (exact - 1) as u64, exact as u64)
    );
}

#[test]
fn raised_max_input_bytes_accepts_a_document_larger_than_the_default() {
    let json = contract_json();
    let below_default = Limits::default().with_max_input_bytes(json.len() - 1);
    assert!(Contract::from_json_with_limits(&json, &below_default).is_err());

    let raised = Limits::default().with_max_input_bytes(json.len());
    Contract::from_json_with_limits(&json, &raised)
        .expect("a raised max_input_bytes must actually take effect");
}

#[test]
fn malformed_json_is_reported_as_syntax_not_as_a_resource_limit() {
    let error = Contract::from_json_with_limits("{ not json", &Limits::default())
        .expect_err("malformed input must be rejected");
    assert!(
        matches!(error, ContractJsonError::MalformedJson(_)),
        "syntax errors must not be laundered into resource errors: {error:?}"
    );

    let error = Compilation::from_json_with_limits("{ not json", &Limits::default())
        .expect_err("malformed input must be rejected");
    assert!(
        matches!(error, ContractJsonError::MalformedJson(_)),
        "{error:?}"
    );

    let error = ContractEnvelope::from_json_with_limits("{ not json", &Limits::default())
        .expect_err("malformed input must be rejected");
    assert!(
        matches!(error, ContractJsonError::MalformedJson(_)),
        "{error:?}"
    );
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn bounded_parsing_does_not_change_identity_bytes() {
    // The whole change is additive to the parse path; nothing may perturb the
    // bytes that identity is computed over.
    let compilation = bundle();
    let json = compilation_json();
    let parsed = Compilation::from_json_with_limits(&json, &Limits::default()).unwrap();

    assert_eq!(
        parsed.contract().contract_id(),
        compilation.contract().contract_id()
    );
    assert_eq!(
        parsed.contract().interface_id(),
        compilation.contract().interface_id()
    );
    assert_eq!(
        parsed.source_info().map(|info| info.source_bundle_id()),
        compilation
            .source_info()
            .map(|info| info.source_bundle_id()),
    );

    let raw = RawContract::from(compilation.contract());
    let rebuilt = Contract::try_from_raw_with_context(raw, &RuntimeContext::default()).unwrap();
    assert_eq!(
        rebuilt.contract_id(),
        compilation.contract().contract_id(),
        "the raw DTO path must yield byte-identical identities"
    );
}

fn compile_did_with_limits(source: &str, limits: &Limits) -> Compilation {
    compile_did_with_context_helper(source, &RuntimeContext::new(limits.clone()))
}

fn compile_did_with_context_helper(source: &str, context: &RuntimeContext) -> Compilation {
    candid_core::compile_did_with_context(source, CompileOptions::default(), context)
        .expect("source must compile under the supplied limits")
}
