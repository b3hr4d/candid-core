//! Boundary and adversarial coverage for HostValue depth and size accounting.
//!
//! `HostValue` is the crate's one recursive value type, and every operation on
//! it — decoding, constructing, cloning, comparing, formatting, dropping, and
//! serializing — walks one stack frame per level. This file pins the two bounds
//! that keep those walks finite:
//!
//! * `value_nesting` bounds lexical JSON container nesting *before*
//!   `serde_json`'s recursive decoder runs, so a hostile document is rejected
//!   by a budget check rather than by exhausting the stack.
//! * `value_depth` and `value_elements` bound semantic nesting and node count,
//!   enforced identically whether a value arrives by decoding or is assembled
//!   through the public constructors.
//!
//! Each bound is exercised at exactly its limit (accepted) and one step over
//! (rejected with stable `{resource, limit, observed}` metadata).
//!
//! What this file does NOT cover: it does not prove that decoding a document at
//! exactly `max_value_nesting` is safe on an arbitrarily small stack. That
//! decode still recurses, at a per-level cost that depends on the build
//! profile; `Limits::max_value_nesting` carries the measured figures and is the
//! single place they are stated. The small-stack guarantee this crate makes is
//! the one asserted in `tests/deep_nesting.rs`: input nested *past* the limit is
//! rejected without recursing at all.

use candid_core::{HostFieldValue, HostValue, HostValueJsonError, Limits};

/// `depth` nested `opt` wrappers around a `null`.
///
/// This is `depth` JSON objects for the options plus one for the `null`, so the
/// document's lexical nesting is `depth + 1` while its semantic value depth is
/// `depth`. The two units differing is exactly why they are separate limits.
fn nested_opt_json(depth: usize) -> String {
    let mut json = String::new();
    for _ in 0..depth {
        json.push_str(r#"{"kind":"opt","value":"#);
    }
    json.push_str(r#"{"kind":"null"}"#);
    for _ in 0..depth {
        json.push('}');
    }
    json
}

/// `depth` nested `opt` wrappers built through the public constructors.
fn nested_opt_value(depth: usize, limits: &Limits) -> Result<HostValue, HostValueJsonError> {
    let mut value = HostValue::null();
    for _ in 0..depth {
        value = HostValue::opt(Some(value), limits)?;
    }
    Ok(value)
}

/// Asserts the `{resource, limit, observed}` triple and returns the reported
/// path.
///
/// The path is returned rather than asserted because it legitimately differs by
/// route: decoding reports the position within the document, while a
/// constructor has no document to point into and reports `$`.
#[track_caller]
fn assert_value_limit(
    error: &HostValueJsonError,
    expected_resource: &str,
    expected_limit: usize,
    expected_observed: usize,
) -> String {
    match error {
        HostValueJsonError::ValueLimit {
            resource,
            limit,
            observed,
            path,
        } => {
            assert_eq!(*resource, expected_resource, "resource");
            assert_eq!(*limit, expected_limit, "limit");
            assert_eq!(*observed, expected_observed, "observed");
            path.clone()
        }
        other => panic!("expected a resource limit, found {other:?}"),
    }
}

// --- decode side (issue #58) ------------------------------------------------

#[test]
fn value_nesting_accepts_exact_limit_and_rejects_one_over() {
    let limits = Limits {
        max_value_nesting: 16,
        ..Limits::default()
    };

    // 15 options plus the terminating `null` object is exactly 16 containers.
    HostValue::from_json_with_limits(&nested_opt_json(15), &limits).unwrap();

    let error = HostValue::from_json_with_limits(&nested_opt_json(16), &limits).unwrap_err();
    assert_value_limit(&error, "value_nesting", 16, 17);
}

#[test]
fn value_nesting_is_rejected_before_the_serde_recursion_ceiling() {
    // `serde_json` applies a fixed 128-frame ceiling that this crate keeps in
    // place. A document past that ceiling must still be reported as this
    // crate's `value_nesting` resource with intact metadata, not as serde's
    // `Malformed("recursion limit exceeded")` string, because the crate-owned
    // check runs first.
    let error =
        HostValue::from_json_with_limits(&nested_opt_json(4_000), &Limits::default()).unwrap_err();
    assert_value_limit(&error, "value_nesting", 64, 65);
}

#[test]
fn value_nesting_ignores_braces_inside_strings() {
    // The scan tracks string and escape state. A text value made entirely of
    // structural characters — including an escaped quote and an escaped
    // backslash immediately before a quote — carries no nesting at all.
    let payload = format!(
        r#"{{"kind":"text","value":"{}\"{}\\"}}"#,
        "{[".repeat(200),
        "]}".repeat(200)
    );
    let value = HostValue::from_json_with_limits(&payload, &Limits::default()).unwrap();
    let rendered = serde_json::to_value(&value).unwrap();
    assert_eq!(rendered["kind"], "text");
}

#[test]
fn decoded_value_depth_accepts_exact_limit_and_rejects_one_over() {
    // The post-decode semantic depth check has never had coverage: at default
    // limits it cannot fire, because `max_value_depth` is 256 while nesting is
    // capped far below that. Lowering `max_value_depth` reaches it.
    let limits = Limits {
        max_value_depth: 5,
        ..Limits::default()
    };

    HostValue::from_json_with_limits(&nested_opt_json(5), &limits).unwrap();

    let error = HostValue::from_json_with_limits(&nested_opt_json(6), &limits).unwrap_err();
    assert_value_limit(&error, "value_depth", 5, 6);
}

// --- construction side (issue #59) ------------------------------------------

#[test]
fn constructed_value_depth_accepts_exact_limit_and_rejects_one_over() {
    let limits = Limits {
        max_value_depth: 5,
        ..Limits::default()
    };

    nested_opt_value(5, &limits).unwrap();

    let error = nested_opt_value(6, &limits).unwrap_err();
    assert_value_limit(&error, "value_depth", 5, 6);
}

#[test]
fn constructed_value_elements_accepts_exact_limit_and_rejects_one_over() {
    let limits = Limits {
        max_value_elements: 8,
        ..Limits::default()
    };

    // The vector node counts as one element alongside its children.
    let leaves = || (0..7).map(|_| HostValue::null()).collect::<Vec<_>>();
    HostValue::vector(leaves(), &limits).unwrap();

    let mut oversized = leaves();
    oversized.push(HostValue::null());
    let error = HostValue::vector(oversized, &limits).unwrap_err();
    assert_value_limit(&error, "value_elements", 8, 9);
}

#[test]
fn every_container_constructor_enforces_the_depth_bound() {
    let limits = Limits {
        max_value_depth: 1,
        ..Limits::default()
    };
    let deep = || HostValue::opt(Some(HostValue::null()), &limits).unwrap();

    // Each container is at depth 1 on its own, and rejected once it encloses
    // another container.
    assert_value_limit(
        &HostValue::opt(Some(deep()), &limits).unwrap_err(),
        "value_depth",
        1,
        2,
    );
    assert_value_limit(
        &HostValue::vector(vec![deep()], &limits).unwrap_err(),
        "value_depth",
        1,
        2,
    );
    assert_value_limit(
        &HostValue::record(vec![HostFieldValue::new(1, deep())], &limits).unwrap_err(),
        "value_depth",
        1,
        2,
    );
    assert_value_limit(
        &HostValue::variant(1, deep(), &limits).unwrap_err(),
        "value_depth",
        1,
        2,
    );
}

#[test]
fn host_field_value_cannot_carry_an_unbounded_value_into_a_record() {
    // `HostFieldValue::new` is infallible by design, so the bound has to be
    // enforced where the field joins a record. A field is not a way around it.
    //
    // One policy throughout: the value is built legitimately at exactly the
    // limit, and wrapping it in a record adds the level that breaches it. The
    // field is what carries the already-at-limit value across, so if `record`
    // trusted it instead of remeasuring, this would succeed at depth 5.
    let limits = Limits {
        max_value_depth: 4,
        ..Limits::default()
    };
    let field = HostFieldValue::new(7, nested_opt_value(4, &limits).unwrap());
    assert_eq!(field.id(), 7);

    let error = HostValue::record(vec![field], &limits).unwrap_err();
    assert_value_limit(&error, "value_depth", 4, 5);
}

// --- the two sides agree ----------------------------------------------------

#[test]
fn decoding_and_constructing_report_the_same_depth_for_the_same_value() {
    // A value rejected for depth must report one `observed` figure, whichever
    // route produced it. The decode path counts container edges as it recurses
    // and the constructors read a cached extent; if those units ever drift, the
    // public metadata becomes route-dependent.
    let limits = Limits {
        max_value_depth: 9,
        ..Limits::default()
    };

    let decoded = HostValue::from_json_with_limits(&nested_opt_json(10), &limits).unwrap_err();
    let constructed = nested_opt_value(10, &limits).unwrap_err();

    let decoded_path = assert_value_limit(&decoded, "value_depth", 9, 10);
    let constructed_path = assert_value_limit(&constructed, "value_depth", 9, 10);

    // Only the triple is shared. The paths deliberately differ: decoding knows
    // where in the document the limit was breached, and a constructor does not.
    assert_eq!(decoded_path, format!("${}", ".value".repeat(10)));
    assert_eq!(constructed_path, "$");
}

#[test]
fn a_value_at_the_limit_round_trips_through_every_recursive_operation() {
    // `Drop`, `Clone`, `PartialEq`, `Debug`, and `Serialize` each recurse once
    // per level. This asserts they agree on a value built at the bound; it is
    // the depth bound itself, asserted above, that keeps them finite.
    //
    // This test is NOT load-bearing for the fix: depth 32 behaved identically
    // before it, so it passes against unpatched source and proves nothing about
    // the bound. It is kept as plain regression cover for the hand-written
    // `Serialize` impl and the derived `PartialEq` that the cached extent
    // introduced — neither of which existed to be broken previously.
    let limits = Limits {
        max_value_depth: 32,
        ..Limits::default()
    };
    let value = nested_opt_value(32, &limits).unwrap();

    let cloned = value.clone();
    assert_eq!(value, cloned);
    assert!(!format!("{value:?}").is_empty());

    let json = serde_json::to_string(&value).unwrap();
    let decoded = HostValue::from_json_with_limits(&json, &limits).unwrap();
    assert_eq!(value, decoded);
    drop(cloned);
}
