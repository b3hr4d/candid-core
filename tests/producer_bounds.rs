//! Bounds and identity boundary for untrusted producer metadata.
//!
//! Producer metadata is caller-supplied provenance. Two properties matter and
//! are pinned here: its aggregate bytes are bounded like every other untrusted
//! string budget, and it stays *outside* authenticated Contract identity so that
//! rewriting it cannot forge — or is forced to break — a signed `contract_id`.

use candid_core::{compile_did, Contract, Limits, ProducerInfo, RawContract, RuntimeContext};

fn producer_bytes(producer: &ProducerInfo) -> usize {
    producer.name.len()
        + producer.version.len()
        + producer.candid_version.len()
        + producer.candid_parser_version.len()
}

fn raw_contract() -> RawContract {
    RawContract::from(
        compile_did("service : { ping: () -> () };")
            .unwrap()
            .contract(),
    )
}

#[test]
fn producer_bytes_accepted_at_the_limit_and_rejected_one_over() {
    // "One over" is framed from the input's side: the same producer is accepted
    // when the limit equals its byte count and rejected when the limit is set
    // one byte lower, i.e. the input is one byte over its configured limit.
    let raw = raw_contract();
    let baseline = producer_bytes(&raw.producer);
    assert!(baseline > 0);

    Contract::try_from_raw_with_context(
        raw.clone(),
        &RuntimeContext::new(Limits {
            max_producer_bytes: baseline,
            ..Limits::default()
        }),
    )
    .unwrap_or_else(|error| {
        panic!("producer bytes exactly at the limit must validate: {error:#?}")
    });

    let error = Contract::try_from_raw_with_context(
        raw,
        &RuntimeContext::new(Limits {
            max_producer_bytes: baseline - 1,
            ..Limits::default()
        }),
    )
    .expect_err("producer bytes one over the limit must be rejected");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    let info = violation.resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "producer_bytes");
    assert_eq!(info.limit, baseline - 1);
    assert_eq!(info.observed, baseline);
}

#[test]
fn a_single_oversized_producer_field_is_rejected_before_identity_checks() {
    // The aggregate bound caps every individual field, so one giant version
    // string fails on `producer_bytes` rather than being validated and stored.
    let mut raw = raw_contract();
    raw.producer.version = "9".repeat(1_000_000);
    let observed = producer_bytes(&raw.producer);

    let error = Contract::try_from_raw_with_context(raw, &RuntimeContext::default())
        .expect_err("a megabyte-long producer field must be rejected");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    let info = violation.resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "producer_bytes");
    assert_eq!(info.limit, Limits::default().max_producer_bytes);
    assert_eq!(info.observed, observed);
}

#[test]
fn oversized_producer_fails_deterministically_across_runs() {
    let mut raw = raw_contract();
    raw.producer.name = "z".repeat(8192);

    let first = Contract::try_from_raw_with_context(raw.clone(), &RuntimeContext::default())
        .expect_err("oversized producer must be rejected");
    let second = Contract::try_from_raw_with_context(raw, &RuntimeContext::default())
        .expect_err("oversized producer must be rejected");
    assert_eq!(first, second);
}

#[test]
fn producer_stays_outside_authenticated_identity() {
    // The load-bearing compatibility boundary: producer is part of the canonical
    // wire bytes but never part of the identity hash. Two Contracts that differ
    // only in producer must therefore share both identities while remaining
    // byte-different on the wire. Binding producer into identity would change
    // every existing `contract_id`; this test would catch that regression.
    let base = raw_contract();
    let mut forked = base.clone();
    forked.producer.name = format!("{}-fork", forked.producer.name);
    forked.producer.version = format!("{}-1", forked.producer.version);

    let original = Contract::build_raw(base, &Limits::default()).unwrap();
    let rebranded = Contract::build_raw(forked, &Limits::default()).unwrap();

    assert_ne!(
        original.producer(),
        rebranded.producer(),
        "the two Contracts must genuinely differ in producer"
    );
    assert_eq!(
        original.contract_id(),
        rebranded.contract_id(),
        "producer must not influence the contract identity"
    );
    assert_eq!(
        original.interface_id(),
        rebranded.interface_id(),
        "producer must not influence the interface identity"
    );
    assert_ne!(
        serde_json::to_string(&original).unwrap(),
        serde_json::to_string(&rebranded).unwrap(),
        "producer is still part of the canonical wire bytes"
    );
}
