//! `checkCompatible`: the live service must be a Candid subtype of the
//! written one, method by method.
//!
//! Three kinds of evidence, all over the same entry point the wasm artifact
//! wraps:
//!
//! - hand-written cases, each pinning its verdict and every diagnostic's
//!   code, method and path;
//! - a differential against upstream `candid`'s own subtype check
//!   (`candid::types::subtype::subtype_with_config`, the exact version
//!   `candid-core` pins), for every hand case and a seeded random campaign:
//!   per written method, the verdict must equal upstream's under the spec's
//!   rules (`OptReport::Silence`), and "no error and no warning" must equal
//!   upstream's verdict with the special opt rule refused
//!   (`OptReport::Error`). A disagreement fails the test unless it is listed,
//!   with its reason, in [`KNOWN_DIVERGENCES`] — and a listed one that no
//!   longer disagrees fails it too;
//! - the agreement file `crates/candid-core-ts/tests/goldens/compat/agreement.json`
//!   for the TypeScript decoder: each case's two Contracts and this check's
//!   per-method verdicts, which `ts/tests/compat-agreement.test.ts` replays
//!   through `wireSubtypeOfSchema` by decoding a live-typed service reference
//!   at the written service. Regenerate it with `UPDATE_GOLDENS=1 cargo test
//!   --test compatibility`, then review the diff.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use candid::types::internal::{Field, Function, Type, TypeInner};
use candid::types::subtype::{subtype_with_config, OptReport};
use candid::TypeEnv;
use candid_core::compile_did;
use candid_core_ts::{generate_module, TsNames, TsOptions};
use candid_core_wasm::{
    check_compatible, check_compatible_with, CheckOptions, MAX_CHECK_OUTPUT_BYTES, MAX_CHECK_STEPS,
    MAX_CHECK_TOTAL_STEPS, MAX_CHECK_TOTAL_WARNINGS, MAX_CHECK_WARNINGS,
};
use candid_parser::{check_prog, IDLProg};
use serde_json::{json, Value};

fn check(written: &str, live: &str) -> Value {
    let request = json!({ "written": { "source": written }, "live": { "source": live } });
    serde_json::from_str(&check_compatible(&request.to_string())).unwrap()
}

/// `(code, method, path)` of every diagnostic, in order.
fn summary(response: &Value) -> Vec<(String, String, Option<String>)> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["code"].as_str().unwrap().to_string(),
                item["method"].as_str().unwrap().to_string(),
                item["path"].as_str().map(str::to_string),
            )
        })
        .collect()
}

/// One hand-written case: the verdict and the diagnostics it must produce.
struct Case {
    name: &'static str,
    written: &'static str,
    live: &'static str,
    compatible: bool,
    /// `(code, method, path)`; an empty path stands for none.
    diagnostics: &'static [(&'static str, &'static str, &'static str)],
}

const LEDGER: &str = "type Account = record { owner : principal; subaccount : opt blob };
type TransferArg = record { to : Account; amount : nat; memo : opt blob };
type TransferError = variant { InsufficientFunds : record { balance : nat }; TooOld };
type TransferResult = variant { Ok : nat; Err : TransferError };
service : {
  balance_of : (Account) -> (nat) query;
  transfer : (TransferArg) -> (TransferResult);
}
";

const CASES: &[Case] = &[
    Case {
        name: "identical",
        written: LEDGER,
        live: LEDGER,
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_adds_a_method",
        written: "service : { a : () -> (nat) query }",
        live: "service : { a : () -> (nat) query; b : (text) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_adds_an_opt_result_field",
        written: "type R = record { x : nat }; service : { get : () -> (R) query }",
        live: "type R = record { x : nat; y : opt text }; service : { get : () -> (R) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_adds_a_required_result_field",
        written: "type R = record { x : nat }; service : { get : () -> (R) query }",
        live: "type R = record { x : nat; y : text }; service : { get : () -> (R) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_drops_a_required_result_field",
        written: "type R = record { x : nat; y : text }; service : { get : () -> (R) query }",
        live: "type R = record { x : nat }; service : { get : () -> (R) query }",
        compatible: false,
        diagnostics: &[("method_incompatible", "get", "$results[0].y")],
    },
    Case {
        name: "live_drops_an_opt_result_field",
        written: "type R = record { x : nat; y : opt text }; service : { get : () -> (R) query }",
        live: "type R = record { x : nat }; service : { get : () -> (R) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_requires_a_new_argument_field",
        written: "type A = record { x : nat }; service : { put : (A) -> () }",
        live: "type A = record { x : nat; y : text }; service : { put : (A) -> () }",
        compatible: false,
        diagnostics: &[("method_incompatible", "put", "$args[0].y")],
    },
    Case {
        name: "live_accepts_a_new_opt_argument_field",
        written: "type A = record { x : nat }; service : { put : (A) -> () }",
        live: "type A = record { x : nat; y : opt text }; service : { put : (A) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_adds_a_trailing_opt_argument",
        written: "service : { put : (nat) -> () }",
        live: "service : { put : (nat, opt text) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_adds_a_trailing_required_argument",
        written: "service : { put : (nat) -> () }",
        live: "service : { put : (nat, text) -> () }",
        compatible: false,
        diagnostics: &[("method_incompatible", "put", "$args[1]")],
    },
    Case {
        name: "live_returns_an_extra_value",
        written: "service : { get : () -> (nat) query }",
        live: "service : { get : () -> (nat, text) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_returns_one_value_fewer",
        written: "service : { get : () -> (nat, text) query }",
        live: "service : { get : () -> (nat) query }",
        compatible: false,
        diagnostics: &[("method_incompatible", "get", "$results[1]")],
    },
    Case {
        name: "live_adds_a_result_variant_arm",
        written: "type E = variant { A; B }; service : { get : () -> (E) query }",
        live: "type E = variant { A; B; C }; service : { get : () -> (E) query }",
        compatible: false,
        diagnostics: &[("method_incompatible", "get", "$results[0].C")],
    },
    Case {
        name: "live_accepts_a_new_argument_variant_arm",
        written: "type E = variant { A; B }; service : { put : (E) -> () }",
        live: "type E = variant { A; B; C }; service : { put : (E) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_widens_an_argument_from_nat_to_int",
        written: "service : { put : (nat) -> () }",
        live: "service : { put : (int) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "live_widens_a_result_from_nat_to_int",
        written: "service : { get : () -> (nat) query }",
        live: "service : { get : () -> (int) query }",
        compatible: false,
        diagnostics: &[("method_incompatible", "get", "$results[0]")],
    },
    Case {
        name: "live_narrows_a_result_from_int_to_nat",
        written: "service : { get : () -> (int) query }",
        live: "service : { get : () -> (nat) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "a_written_method_is_missing",
        written: "service : { a : () -> (); b : () -> () }",
        live: "service : { a : () -> () }",
        compatible: false,
        diagnostics: &[("method_missing", "b", "")],
    },
    Case {
        name: "a_method_changes_mode",
        written: "service : { get : () -> (nat) query }",
        live: "service : { get : () -> (nat) }",
        compatible: false,
        diagnostics: &[("mode_changed", "get", "")],
    },
    Case {
        name: "a_method_changes_mode_and_type",
        written: "service : { get : () -> (nat) query }",
        live: "service : { get : () -> (text) composite_query }",
        compatible: false,
        diagnostics: &[
            ("mode_changed", "get", ""),
            ("method_incompatible", "get", "$results[0]"),
        ],
    },
    Case {
        name: "the_special_opt_rule_is_a_warning",
        written: "type R = record { x : opt nat }; service : { get : () -> (R) query }",
        live: "type R = record { x : opt text }; service : { get : () -> (R) query }",
        compatible: true,
        diagnostics: &[("special_opt_rule", "get", "$results[0].x")],
    },
    Case {
        name: "a_value_lifted_into_an_opt_is_kept",
        written: "type R = record { x : opt nat }; service : { get : () -> (R) query }",
        live: "type R = record { x : nat }; service : { get : () -> (R) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "a_nested_mode_change_in_a_callback",
        written: "type Cb = func (nat) -> () query; service : { sub : (Cb) -> () }",
        live: "type Cb = func (nat) -> (); service : { sub : (Cb) -> () }",
        compatible: false,
        diagnostics: &[("method_incompatible", "sub", "$args[0]")],
    },
    Case {
        name: "a_nested_argument_change_in_a_callback",
        written: "type Cb = func (nat) -> (); service : { sub : (Cb) -> () }",
        live: "type Cb = func (int) -> (); service : { sub : (Cb) -> () }",
        compatible: false,
        diagnostics: &[("method_incompatible", "sub", "$args[0]::args[0]")],
    },
    Case {
        name: "a_nested_service_loses_a_method",
        written: "type S = service { ping : () -> () }; service : { peer : () -> (S) query }",
        live: "type S = service { pong : () -> () }; service : { peer : () -> (S) query }",
        compatible: false,
        diagnostics: &[("method_incompatible", "peer", "$results[0]::ping")],
    },
    Case {
        name: "recursive_types_with_the_same_shape",
        written: "type List = opt record { head : nat; tail : List }; service : { all : () -> (List) query }",
        live: "type Tree = opt record { head : nat; tail : Tree }; service : { all : () -> (Tree) query }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "recursive_types_with_a_deep_change",
        written: "type L = variant { nil; cons : record { nat; L } }; service : { all : () -> (L) query }",
        live: "type L = variant { nil; cons : record { int; L } }; service : { all : () -> (L) query }",
        compatible: false,
        diagnostics: &[("method_incompatible", "all", "$results[0].cons[0]")],
    },
    Case {
        name: "a_class_compares_its_service",
        written: "service : { get : () -> (nat) query }",
        live: "service : (nat) -> { get : () -> (nat) query; put : (nat) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "the_written_side_may_be_a_hand_written_subset",
        written: "type Account = record { owner : principal; subaccount : opt blob };
service : { balance_of : (Account) -> (nat) query }",
        live: LEDGER,
        compatible: true,
        diagnostics: &[],
    },
    Case {
        name: "reserved_accepts_anything_and_empty_is_accepted_anywhere",
        written: "service : { get : () -> (reserved) query; put : (empty) -> () }",
        live: "service : { get : () -> (record { a : text }) query; put : (nat) -> () }",
        compatible: true,
        diagnostics: &[],
    },
    // A warning is reported at every path that decodes as null, not only
    // the first: the two fields share one canonical pair.
    Case {
        name: "a_warning_at_every_field_of_one_type_pair",
        written: "service : { get : () -> (record { a : opt nat; b : opt nat }) }",
        live: "service : { get : () -> (record { a : opt text; b : opt text }) }",
        compatible: true,
        diagnostics: &[
            ("special_opt_rule", "get", "$results[0].a"),
            ("special_opt_rule", "get", "$results[0].b"),
        ],
    },
    // An alias whose `opt` content changed, used in several places: each use
    // is reported, those nested in a proven pair under each of its paths.
    Case {
        name: "a_changed_opt_alias_is_reported_at_each_use",
        written: "type Memo = opt blob;
type Entry = record { memo : Memo; n : nat };
service : {
  get : (Memo) -> (record { first : Entry; second : Entry; memo : Memo });
}",
        live: "type Memo = opt text;
type Entry = record { memo : Memo; n : nat };
service : {
  get : (Memo) -> (record { first : Entry; second : Entry; memo : Memo });
}",
        compatible: true,
        diagnostics: &[
            ("special_opt_rule", "get", "$args[0]"),
            ("special_opt_rule", "get", "$results[0].first.memo"),
            ("special_opt_rule", "get", "$results[0].memo"),
            ("special_opt_rule", "get", "$results[0].second.memo"),
        ],
    },
    // A failed opt probe takes back the warnings recorded inside it: only
    // the outer opt decodes as null, so only it is reported.
    Case {
        name: "a_failed_probe_takes_back_its_warnings",
        written: "service : { get : () -> (record { x : opt record { a : opt nat; b : nat } }) }",
        live: "service : { get : () -> (record { x : opt record { a : opt text; b : text } }) }",
        compatible: true,
        diagnostics: &[("special_opt_rule", "get", "$results[0].x")],
    },
    // A pair proven inside a failed probe (`P`, under `x`) is forgotten with
    // its warnings: walked again under `y`, where it recurses, it must not
    // re-report what the failed probe took back. Inside the recursion a
    // warning is reported up to where its path comes back round to `P`.
    Case {
        name: "a_pair_proven_in_a_failed_probe_is_forgotten",
        written: "type P = record { n : opt P; m : opt nat };
service : { get : () -> (record { x : opt record { p : P; bad : nat }; y : P }) }",
        live: "type P = record { n : opt P; m : opt text };
service : { get : () -> (record { x : opt record { p : P; bad : text }; y : P }) }",
        compatible: true,
        diagnostics: &[
            ("special_opt_rule", "get", "$results[0].x"),
            ("special_opt_rule", "get", "$results[0].y.m"),
        ],
    },
    // Mutually recursive types: `Q` is first proven inside `P`'s walk, cut
    // at `P` while `P` has not yet recorded `.y`. That proof is dropped when
    // `P`'s walk ends, so `Q` met again is walked again and `.back.y` is
    // reported under it. The order of the results does not change the set.
    Case {
        name: "a_type_proven_inside_an_enclosing_recursion_is_walked_again",
        written: "type P = record { x : Q; y : opt nat };
type Q = record { back : P };
service : { m : () -> (P, Q); n : () -> (Q, P) }",
        live: "type P = record { x : Q; y : opt text };
type Q = record { back : P };
service : { m : () -> (P, Q); n : () -> (Q, P) }",
        compatible: true,
        diagnostics: &[
            ("special_opt_rule", "m", "$results[0].y"),
            ("special_opt_rule", "m", "$results[1].back.y"),
            ("special_opt_rule", "n", "$results[0].back.y"),
            ("special_opt_rule", "n", "$results[1].y"),
        ],
    },
    // The same through record fields: which label comes first (`p` before
    // `q`, but `q` before `pp`) changes the order of the report, not what is
    // in it.
    Case {
        name: "the_paths_reported_do_not_depend_on_label_order",
        written: "type P = record { x : Q; y : opt nat };
type Q = record { back : P };
service : { a : () -> (record { p : P; q : Q }); b : () -> (record { pp : P; q : Q }) }",
        live: "type P = record { x : Q; y : opt text };
type Q = record { back : P };
service : { a : () -> (record { p : P; q : Q }); b : () -> (record { pp : P; q : Q }) }",
        compatible: true,
        diagnostics: &[
            ("special_opt_rule", "a", "$results[0].p.y"),
            ("special_opt_rule", "a", "$results[0].q.back.y"),
            ("special_opt_rule", "b", "$results[0].q.back.y"),
            ("special_opt_rule", "b", "$results[0].pp.y"),
        ],
    },
    // Upstream's unsound memo (its reference divergence, recorded below):
    // proving `c : opt W5 <: opt V` assumes (W5, V), proves (W7, E7) under it,
    // drops only (W5, V) when the arm `x` fails, and answers `v : W7 <: E7`
    // from the stale pair. `W7 <: E7` is false: W5's arm `x` is not in V.
    Case {
        name: "the_unsound_memo_case",
        written: "type V = variant { a : E7 };
type E7 = variant { c : V };
type E3 = variant { c : opt V; v : E7 };
service : { get : () -> (E3) }",
        live: "type W5 = variant { a : W7; x : text };
type W7 = variant { c : W5 };
type W3 = variant { c : opt W5; v : W7 };
service : { get : () -> (W3) }",
        compatible: false,
        diagnostics: &[
            ("method_incompatible", "get", "$results[0].v.c.x"),
            ("special_opt_rule", "get", "$results[0].c"),
        ],
    },
    // The same memo, met by the extended campaign (its seed 992) through
    // func and service references. `T0` reaches `T2` contravariantly
    // (`$results[0].b::f::results[0].d[*].d::args[0]`), where the written
    // `T2` must be a subtype of the live one; the live `T2` returns a
    // `variant { d : null }` the written one does not, so it is not. Upstream
    // answers that pair from one its failed opt probe left assumed, and
    // accepts.
    Case {
        name: "the_unsound_memo_through_references",
        written: "type T0 = func (T2, opt service { f : (bool, reserved) -> (bool, T1); g : (T1, empty) -> () }) -> (record { a : vec principal; b : func (T2) -> (int32, text) query; e : opt T2 });
type T1 = variant { b : func () -> (T1); d : vec variant { c : T2; d : T0 } };
type T2 = func (record { a : opt T1; b : nat64; c : T0; d : service { f : (T2) -> (T0, bool) } }) -> ();
service : {
  m0 : () -> (record { a : text; b : service { f : (empty, T0) -> (T1) }; e : vec principal }, T0);
  m1 : () -> () query;
}",
        live: "type T0 = func (T2, opt service { f : (bool, reserved) -> (bool, T1); g : (T1, empty) -> () }) -> (record { a : vec principal; b : func (T2) -> (int32, text) query; e : opt T2 });
type T1 = variant { b : func () -> (T1); d : vec variant { c : T2; d : T0 } };
type T2 = func (record { a : opt T1; b : nat64; c : T0; d : service { f : (T2) -> (T0, bool) } }) -> (variant { d : null });
service : {
  m0 : () -> (record { a : text; b : service { f : (empty, T0) -> (T1) }; e : vec principal }, T0);
}",
        compatible: false,
        diagnostics: &[
            (
                "method_incompatible",
                "m0",
                "$results[0].b::f::results[0].d[*].d::args[0]::results[0]",
            ),
            (
                "special_opt_rule",
                "m0",
                "$results[0].b::f::args[1]::args[0]::args[0].a",
            ),
            ("special_opt_rule", "m0", "$results[0].b::f::args[1]::args[1]"),
            (
                "special_opt_rule",
                "m0",
                "$results[0].b::f::args[1]::results[0].b::args[0]::args[0].a",
            ),
            ("special_opt_rule", "m0", "$results[0].b::f::args[1]::results[0].e"),
            ("method_missing", "m1", ""),
        ],
    },
];

/// Disagreements with upstream that were triaged by hand: `(case, method,
/// which verdict, reason)`. Every other disagreement fails the test.
const KNOWN_DIVERGENCES: &[(&str, &str, &str, &str)] = &[
    (
        "the_unsound_memo_case",
        "get",
        "lenient",
        "upstream candid's coinductive memo keeps a pair proven under an assumption a failed opt probe retracted (b3hr4d/candid-core#227, item 3); this check refuses, upstream accepts",
    ),
    (
        "the_unsound_memo_through_references",
        "m0",
        "lenient",
        "the same memo (b3hr4d/candid-core#227, item 3), through func and service references; this check refuses, upstream accepts",
    ),
];

#[test]
fn hand_cases_pin_verdicts_codes_and_paths() {
    for case in CASES {
        let response = check(case.written, case.live);
        assert_eq!(response["ok"], json!(true), "{}: {response}", case.name);
        assert_eq!(
            response["compatible"],
            json!(case.compatible),
            "{}: {response}",
            case.name
        );
        let expected: Vec<(String, String, Option<String>)> = case
            .diagnostics
            .iter()
            .map(|(code, method, path)| {
                (
                    code.to_string(),
                    method.to_string(),
                    (!path.is_empty()).then(|| path.to_string()),
                )
            })
            .collect();
        assert_eq!(summary(&response), expected, "{}: {response}", case.name);
        for item in response["diagnostics"].as_array().unwrap() {
            let severity = if item["code"] == "special_opt_rule" {
                "warning"
            } else {
                "error"
            };
            assert_eq!(item["severity"], severity, "{}: {item}", case.name);
            assert!(item["message"].as_str().is_some_and(|m| !m.is_empty()));
        }
    }
}

#[test]
fn identities_tell_changed_from_unchanged() {
    let written = "service : { get : () -> (nat) query }";
    let same = check(written, written);
    assert_eq!(
        same["live"]["interface_id"],
        same["written"]["interface_id"]
    );
    let changed = check(
        written,
        "service : { get : () -> (nat) query; put : () -> () }",
    );
    assert_eq!(changed["compatible"], json!(true));
    assert_ne!(
        changed["live"]["interface_id"],
        changed["written"]["interface_id"]
    );
    assert_eq!(
        same["live"]["interface_id"],
        changed["written"]["interface_id"]
    );
    for side in ["written", "live"] {
        for key in ["contract_id", "interface_id"] {
            assert!(changed[side][key]
                .as_str()
                .is_some_and(|id| id.starts_with("candid-core:")));
        }
    }
}

#[test]
fn bundles_and_failures() {
    let request = json!({
        "written": { "entry": "app.did", "files": {
            "app.did": "import \"types.did\";\nservice : { get : () -> (Item) query }",
            "types.did": "type Item = record { id : nat };",
        } },
        "live": { "source": "service : { get : () -> (record { id : nat; name : opt text }) query }" },
    });
    let response: Value = serde_json::from_str(&check_compatible(&request.to_string())).unwrap();
    assert_eq!(response["compatible"], json!(true), "{response}");

    // A side that does not compile names itself; the diagnostics are the
    // compiler's, verbatim.
    let response = check("service : { get : () -> (Missing) }", "service : {}");
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["input"], json!("written"));
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("did_type_check_error")
    );
    let response = check("service : {}", "service : { oops ");
    assert_eq!(response["input"], json!("live"));
    assert_eq!(response["diagnostics"][0]["code"], json!("did_parse_error"));

    // No service on either side: nothing to compare.
    for (written, live, side) in [
        ("type A = nat;", "service : {}", "written"),
        ("service : {}", "type A = nat;", "live"),
    ] {
        let response = check(written, live);
        assert_eq!(response["ok"], json!(false));
        assert_eq!(response["input"], json!(side));
        assert_eq!(response["diagnostics"][0]["code"], json!("no_service"));
    }

    // Malformed requests fail closed with invalid_request and no `input`.
    for request in [
        "[]".to_string(),
        "{".to_string(),
        json!({ "written": { "source": "service : {}" } }).to_string(),
        json!({ "written": { "source": "service : {}" }, "live": "service : {}" }).to_string(),
        json!({ "written": { "source": "service : {}" }, "live": { "source": "service : {}" }, "extra": 1 }).to_string(),
        json!({ "written": { "source": "service : {}", "methods": [] }, "live": { "source": "service : {}" } }).to_string(),
    ] {
        let response: Value = serde_json::from_str(&check_compatible(&request)).unwrap();
        assert_eq!(response["ok"], json!(false), "{request}");
        assert_eq!(response["diagnostics"][0]["code"], json!("invalid_request"), "{request}");
        assert!(response.get("input").is_none(), "{request}");
    }
}

#[test]
fn responses_are_deterministic() {
    for case in CASES {
        let request =
            json!({ "written": { "source": case.written }, "live": { "source": case.live } })
                .to_string();
        assert_eq!(
            check_compatible(&request),
            check_compatible(&request),
            "{}",
            case.name
        );
    }
}

/// Cycles of different lengths on the two sides make the walk's path long:
/// past the depth bound the method fails closed, it does not overflow.
#[test]
fn the_walk_is_bounded() {
    // A ring of records with one marked node, so canonicalization cannot
    // fold it shorter: every node of `W` is distinct, and so is every node
    // of `L`. Every pair of the two rings holds (the marker is `reserved`,
    // which a record may lack), so the walk visits all of them on one path.
    fn ring(prefix: &str, marker: &str, length: usize) -> String {
        let mut source = String::new();
        for i in 0..length {
            let mark = if i == 0 {
                format!("; {marker} : reserved")
            } else {
                String::new()
            };
            source.push_str(&format!(
                "type {prefix}{i} = record {{ n : {prefix}{}{mark} }};\n",
                (i + 1) % length
            ));
        }
        source.push_str(&format!("service : {{ get : () -> ({prefix}0) query }}\n"));
        source
    }
    // Two coprime ring lengths: the pairs repeat only after 19 × 23 = 437
    // steps, past the 384 bound.
    let response = check(&ring("W", "k", 19), &ring("L", "j", 23));
    assert_eq!(response["compatible"], json!(false), "{response}");
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("resource_limit_exceeded")
    );
    assert_eq!(
        response["diagnostics"][0]["resource_limit"],
        json!({ "resource": "check_depth", "limit": 384, "observed": 385 })
    );
    // Rings whose product stays under the bound are decided.
    let response = check(&ring("W", "k", 13), &ring("L", "j", 17));
    assert_eq!(response["compatible"], json!(true), "{response}");
    // A walk to the bound fits a 2 MiB stack even unoptimized.
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| check(&ring("W", "k", 19), &ring("L", "j", 23)))
        .unwrap();
    assert_eq!(handle.join().unwrap()["compatible"], json!(false));
}

/// A method's diagnostics do not depend on which other methods the written
/// service declares: `b` is reported at the same path whether or not `a`,
/// checked first and walking the same types, is there.
#[test]
fn a_methods_diagnostics_do_not_depend_on_the_other_methods() {
    let source = |leaf: &str, methods: &str| {
        format!(
            "type X = record {{ r : R; bad : {leaf} }};\ntype R = record {{ f1 : X; f2 : {leaf} }};\nservice : {{ {methods} }}\n"
        )
    };
    let both = "a : () -> (X); b : () -> (R)";
    let only_b = "b : () -> (R)";
    let paths = |written: &str| -> Vec<(String, String, Option<String>)> {
        summary(&check(written, &source("text", both)))
            .into_iter()
            .filter(|(_, method, _)| method == "b")
            .collect()
    };
    let alone = paths(&source("nat", only_b));
    assert_eq!(
        alone,
        [(
            "method_incompatible".to_string(),
            "b".to_string(),
            Some("$results[0].f1.bad".to_string())
        )]
    );
    assert_eq!(paths(&source("nat", both)), alone);
}

/// Re-reporting a proven pair's warnings under each of its paths is bounded:
/// a shared type graph that doubles the paths at every level reports the
/// first 1,000 and says the rest were dropped. The verdict, which is
/// complete, stands.
#[test]
fn warnings_are_bounded() {
    let doubling = |content: &str, levels: usize| {
        let mut source = format!("type D0 = opt {content};\n");
        for level in 1..=levels {
            source.push_str(&format!(
                "type D{level} = record {{ l : D{p}; r : D{p} }};\n",
                p = level - 1
            ));
        }
        source.push_str(&format!("service : {{ get : () -> (D{levels}) }}\n"));
        source
    };
    // 2^9 = 512 paths decode as null: each reported.
    let response = check(&doubling("nat", 9), &doubling("text", 9));
    assert_eq!(response["compatible"], json!(true), "{response}");
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 512);
    let distinct: BTreeSet<&str> = items
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert_eq!(distinct.len(), 512, "each path once");
    // 2^10 = 1024 is past the bound of 1000: the first 1000 are reported,
    // then one warning that the rest were not. The method stays compatible.
    let response = check(&doubling("nat", 10), &doubling("text", 10));
    assert_eq!(response["compatible"], json!(true), "{response}");
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 1001);
    let distinct: BTreeSet<&str> = items[..1000]
        .iter()
        .map(|item| {
            assert_eq!(item["code"], json!("special_opt_rule"));
            item["path"].as_str().unwrap()
        })
        .collect();
    assert_eq!(distinct.len(), 1000, "each path once");
    assert_eq!(
        items[1000],
        json!({
            "code": "resource_limit_exceeded",
            "severity": "warning",
            "method": "get",
            "message": "this method has more special_opt_rule warnings than its check_warnings bound of 1000; the rest are not reported, and the verdict stands",
            "resource_limit": { "resource": "check_warnings", "limit": 1000, "observed": 1001 },
        })
    );
    // A failed probe that reached the bound takes back the truncation with
    // its warnings: only the outer opt decodes as null.
    let inside = |content: &str, leaf: &str| {
        doubling(content, 10).replace(
            "service : { get : () -> (D10) }",
            &format!("service : {{ get : () -> (record {{ x : opt record {{ d : D10; bad : {leaf} }} }}) }}"),
        )
    };
    let response = check(&inside("nat", "nat"), &inside("text", "text"));
    assert_eq!(response["compatible"], json!(true), "{response}");
    assert_eq!(
        summary(&response),
        [(
            "special_opt_rule".to_string(),
            "get".to_string(),
            Some("$results[0].x".to_string())
        )]
    );
}

/// The work bound fails a method closed, and each method starts again from
/// nothing spent: `f` and `g` cost eight steps each (the result examined and
/// its record pair visited, then each of the three fields examined and its
/// pair visited).
#[test]
fn the_work_is_bounded_per_method() {
    assert_eq!(CheckOptions::default().step_limit, MAX_CHECK_STEPS);
    let source = "type R = record { a : nat; b : text; c : bool };
service : { f : () -> (R); g : () -> (R) }";
    let request =
        json!({ "written": { "source": source }, "live": { "source": source } }).to_string();
    let run = |step_limit: usize| -> Value {
        let options = CheckOptions {
            step_limit,
            ..CheckOptions::default()
        };
        serde_json::from_str(&check_compatible_with(&request, options)).unwrap()
    };
    let response = run(8);
    assert_eq!(response["compatible"], json!(true), "{response}");
    assert_eq!(response["diagnostics"], json!([]));
    let response = run(7);
    assert_eq!(response["compatible"], json!(false), "{response}");
    let bound = |method: &str| {
        json!({
            "code": "resource_limit_exceeded",
            "severity": "error",
            "method": method,
            "message": "the check of this method stopped at its check_steps bound of 7",
            "resource_limit": { "resource": "check_steps", "limit": 7, "observed": 8 },
        })
    };
    assert_eq!(response["diagnostics"], json!([bound("f"), bound("g")]));
}

/// `checkCompatible` under `options`.
fn check_with(written: &str, live: &str, options: CheckOptions) -> Value {
    let request = json!({ "written": { "source": written }, "live": { "source": live } });
    serde_json::from_str(&check_compatible_with(&request.to_string(), options)).unwrap()
}

/// The fail-closed diagnostic a method gets once the whole check has
/// reached an aggregate bound.
fn stopped(method: &str, resource: &str, limit: usize, observed: usize) -> Value {
    json!({
        "code": "resource_limit_exceeded",
        "severity": "error",
        "method": method,
        "message": format!("the check of the whole service reached its {resource} bound of {limit}; this method fails closed, and nothing else is reported for it"),
        "resource_limit": { "resource": resource, "limit": limit, "observed": observed },
    })
}

/// Codex's example: a 10-level shared two-field graph, 1,024 paths of which
/// decode as null, reused by `methods` methods named in name order.
fn shared_doubling(content: &str, methods: usize) -> String {
    let mut source = format!("type D0 = opt {content};\n");
    for level in 1..=10 {
        source.push_str(&format!(
            "type D{level} = record {{ l : D{p}; r : D{p} }};\n",
            p = level - 1
        ));
    }
    let list: Vec<String> = (0..methods)
        .map(|index| format!("m{index:05} : () -> (D10);"))
        .collect();
    source.push_str(&format!("service : {{ {} }}\n", list.join(" ")));
    source
}

/// The whole check's work is bounded: the per-method bound alone would let a
/// service multiply it by its method count. Each of `f`, `g`, `h` and `i`
/// costs nine steps of the total, one for the method and eight for its
/// walk; the method that takes the total past the bound, and every method
/// after it, fail closed.
#[test]
fn the_work_is_bounded_across_the_check() {
    assert_eq!(MAX_CHECK_TOTAL_STEPS, 10 * MAX_CHECK_STEPS);
    assert_eq!(
        CheckOptions::default().total_step_limit,
        MAX_CHECK_TOTAL_STEPS
    );
    let source = "type R = record { a : nat; b : text; c : bool };
service : { f : () -> (R); g : () -> (R); h : () -> (R); i : () -> (R) }";
    let run = |total_step_limit: usize| {
        let options = CheckOptions {
            total_step_limit,
            ..CheckOptions::default()
        };
        check_with(source, source, options)
    };
    // Exactly at the bound: 36 steps, all decided.
    let response = run(36);
    assert_eq!(response["compatible"], json!(true), "{response}");
    assert_eq!(response["diagnostics"], json!([]));
    // One step under: `i` reaches it at its last step.
    let response = run(35);
    assert_eq!(response["compatible"], json!(false), "{response}");
    assert_eq!(
        response["diagnostics"],
        json!([stopped("i", "check_total_steps", 35, 36)])
    );
    // `h` reaches it at the third step of its walk; `i` is not walked.
    let response = run(21);
    assert_eq!(
        response["diagnostics"],
        json!([
            stopped("h", "check_total_steps", 21, 22),
            stopped("i", "check_total_steps", 21, 22),
        ])
    );

    // Methods that reach their own bound first: each result probes an `opt`
    // whose content walks a 12-field, 100-level graph and then fails, so the
    // probe forgets the pairs it proved and the next result walks them
    // again, 2,409 steps a result. Under a bound of 20,000 a method, nine
    // methods fail closed at their own bound; the tenth takes the total past
    // ten methods' worth, and it and every method after it are not decided.
    // `a_wide_record_shared_by_many_methods_reaches_the_total_step_bound`
    // reaches the shipped total bound.
    let forgetting = |leaf: &str, methods: usize| {
        let mut source = String::from("type D0 = nat;\n");
        for level in 1..=100 {
            let fields: Vec<String> = (0..12)
                .map(|field| format!("f{field} : D{p}", p = level - 1))
                .collect();
            source.push_str(&format!(
                "type D{level} = record {{ {} }};\n",
                fields.join("; ")
            ));
        }
        let mut results = Vec::new();
        for result in 0..10 {
            source.push_str(&format!(
                "type E{result} = record {{ g : D100; b{result} : {leaf} }};\n"
            ));
            results.push(format!("opt E{result}"));
        }
        let list: Vec<String> = (0..methods)
            .map(|index| format!("m{index:05} : () -> ({});", results.join(", ")))
            .collect();
        source.push_str(&format!("service : {{ {} }}\n", list.join(" ")));
        source
    };
    let (written, live) = (forgetting("nat", 12), forgetting("text", 12));
    let options = |step_limit: usize, total_step_limit: usize| CheckOptions {
        step_limit,
        total_step_limit,
        ..CheckOptions::default()
    };
    // Unbounded, each method is decided: ten warnings, one per result.
    let response = check_with(&written, &live, options(usize::MAX, usize::MAX));
    assert_eq!(response["compatible"], json!(true), "{response}");
    assert_eq!(response["diagnostics"].as_array().unwrap().len(), 120);
    // A method costs 24,090 steps of its own: decided at that bound, not
    // one under.
    let (one_written, one_live) = (forgetting("nat", 1), forgetting("text", 1));
    let response = check_with(&one_written, &one_live, options(24_090, usize::MAX));
    assert_eq!(response["compatible"], json!(true), "{response}");
    let response = check_with(&one_written, &one_live, options(24_089, usize::MAX));
    assert_eq!(
        response["diagnostics"][0]["resource_limit"],
        json!({ "resource": "check_steps", "limit": 24_089, "observed": 24_090 })
    );
    let response = check_with(&written, &live, options(20_000, 200_000));
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 12, "{response}");
    for item in &items[..9] {
        assert_eq!(item["resource_limit"]["resource"], json!("check_steps"));
    }
    let observed = items[9]["resource_limit"]["observed"].as_u64().unwrap() as usize;
    assert!(observed > 200_000, "{observed}");
    for (index, item) in items[9..].iter().enumerate() {
        assert_eq!(
            *item,
            stopped(
                &format!("m{:05}", 9 + index),
                "check_total_steps",
                200_000,
                observed
            )
        );
    }
}

/// The warnings the whole check reports are bounded. Codex's example: each
/// method of a shared 10-level graph reports its first 1,000 warnings; ten
/// methods reach the 10,000 bound exactly and are reported, an eleventh
/// would pass it, so it and every method after it fail closed. Before this
/// bound, 100 such methods turned a 5.3 KB request into a 27.8 MB response.
#[test]
fn warnings_are_bounded_across_the_check() {
    assert_eq!(MAX_CHECK_TOTAL_WARNINGS, 10 * MAX_CHECK_WARNINGS);
    assert_eq!(
        CheckOptions::default().total_warning_limit,
        MAX_CHECK_TOTAL_WARNINGS
    );
    let run = |methods: usize| {
        let request = json!({
            "written": { "source": shared_doubling("nat", methods) },
            "live": { "source": shared_doubling("text", methods) },
        })
        .to_string();
        let response = check_compatible(&request);
        (
            request.len(),
            response.len(),
            serde_json::from_str::<Value>(&response).unwrap(),
        )
    };
    let (_, at_bound_bytes, at_bound) = run(10);
    assert_eq!(at_bound["compatible"], json!(true));
    let items = at_bound["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 10 * 1001);
    assert_eq!(
        items
            .iter()
            .filter(|item| item["code"] == "special_opt_rule")
            .count(),
        MAX_CHECK_TOTAL_WARNINGS
    );

    let mut sizes = Vec::new();
    for methods in [11, 100, 3000] {
        let (request_bytes, response_bytes, response) = run(methods);
        assert_eq!(response["compatible"], json!(false));
        let items = response["diagnostics"].as_array().unwrap();
        assert_eq!(items.len(), 10 * 1001 + methods - 10, "{methods}");
        // The first ten methods are reported as they are at the bound.
        assert_eq!(
            items[..10 * 1001],
            at_bound["diagnostics"].as_array().unwrap()[..]
        );
        for (index, item) in items[10 * 1001..].iter().enumerate() {
            assert_eq!(
                *item,
                stopped(
                    &format!("m{:05}", 10 + index),
                    "check_total_warnings",
                    MAX_CHECK_TOTAL_WARNINGS,
                    11_000
                )
            );
        }
        // Past the bound the response grows only by one fixed-size
        // diagnostic per method: linearly with the request.
        let per_method = (response_bytes - at_bound_bytes) / (methods - 10);
        assert!(per_method < 500, "{methods}: {per_method} bytes per method");
        sizes.push((methods, request_bytes, response_bytes));
    }
    // The sizes the CLI page rounds: 100 methods, a 5.3 KB request, a 2.8 MB
    // response (27.8 MB before this bound, measured on the earlier code, not
    // pinned here); 3,000 methods, 4.0 MB; and 397 bytes for each method
    // left past the bound, whose name has six characters.
    assert_eq!((sizes[2].2 - sizes[1].2) / (3000 - 100), 397);
    assert_eq!((sizes[2].2 - sizes[1].2) % (3000 - 100), 0);
    assert_eq!(
        sizes,
        [
            (11, 1_363, 2_784_821),
            (100, 5_279, 2_820_154),
            (3000, 132_879, 3_971_454),
        ]
    );
}

/// The diagnostic text the whole check reports is bounded in bytes, so a
/// long name repeated along paths cannot multiply the response: the
/// `method`, `path` and `message` strings of each method's diagnostics
/// count, and the method that would pass the bound, and every method after
/// it, fail closed.
#[test]
fn the_output_is_bounded_across_the_check() {
    assert_eq!(MAX_CHECK_OUTPUT_BYTES, 4 * 1024 * 1024);
    assert_eq!(
        CheckOptions::default().output_byte_limit,
        MAX_CHECK_OUTPUT_BYTES
    );
    let source = |leaf: &str, name: &str, methods: usize| {
        let list: Vec<String> = (0..methods)
            .map(|index| format!("m{index:05} : () -> (R);"))
            .collect();
        format!(
            "type R = record {{ {name} : {leaf} }};\nservice : {{ {} }}\n",
            list.join(" ")
        )
    };
    // Each method fails at `$results[0].x`: its text is the method name, the
    // path and the message.
    let written = source("nat", "x", 3);
    let live = source("text", "x", 3);
    let unbounded = check(&written, &live);
    let items = unbounded["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let text = |item: &Value| -> usize {
        ["method", "path", "message"]
            .iter()
            .map(|key| item[*key].as_str().map_or(0, str::len))
            .sum()
    };
    let each = text(&items[0]);
    assert_eq!(
        each,
        "m00000".len() + "$results[0].x".len() + "text is not a subtype of nat".len()
    );
    let run = |output_byte_limit: usize| {
        let options = CheckOptions {
            output_byte_limit,
            ..CheckOptions::default()
        };
        check_with(&written, &live, options)
    };
    // Exactly at the bound: all three reported.
    assert_eq!(run(3 * each), unbounded);
    // One byte under: the third is not.
    let response = run(3 * each - 1);
    assert_eq!(response["diagnostics"].as_array().unwrap()[..2], items[..2]);
    assert_eq!(
        response["diagnostics"][2],
        stopped("m00002", "check_output_bytes", 3 * each - 1, 3 * each)
    );

    // The bytes are counted as the JSON response writes them, escapes
    // included: a method name of five bytes, with a tab, a quote and a
    // control character, takes twelve.
    let escaped = |leaf: &str| {
        format!(
            "type R = record {{ x : {leaf} }};\nservice : {{ \"t\\tq\\\"\\u{{1}}\" : () -> (R); \"z\" : () -> (R) }}\n"
        )
    };
    let unbounded = check(&escaped("nat"), &escaped("text"));
    let items = unbounded["diagnostics"].as_array().unwrap();
    assert_eq!(items[0]["method"], json!("t\tq\"\u{1}"));
    let written_text = |item: &Value| -> usize {
        ["method", "path", "message"]
            .iter()
            .map(|key| serde_json::to_string(&item[*key]).unwrap().len() - 2)
            .sum()
    };
    let name = "t\\tq\\\"\\u0001".len();
    assert_eq!(name, 12);
    assert_eq!(written_text(&items[0]), name + text(&items[1]) - 1);
    let both = written_text(&items[0]) + written_text(&items[1]);
    let run = |output_byte_limit: usize| {
        let options = CheckOptions {
            output_byte_limit,
            ..CheckOptions::default()
        };
        check_with(&escaped("nat"), &escaped("text"), options)
    };
    assert_eq!(run(both), unbounded);
    assert_eq!(
        run(both - 1)["diagnostics"][1],
        stopped("z", "check_output_bytes", both - 1, both)
    );

    // At the shipped bound: a 100,000-byte field name on every path. 41
    // methods' text fits in 4 MiB; the 42nd and the rest fail closed.
    let name = "x".repeat(100_000);
    let response = check(&source("nat", &name, 50), &source("text", &name, 50));
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 50);
    let each = "m00000".len() + 1 + "$results[0].".len() + name.len() - 1
        + "text is not a subtype of nat".len();
    let fits = MAX_CHECK_OUTPUT_BYTES / each;
    assert_eq!(fits, 41);
    for item in &items[..fits] {
        assert_eq!(item["code"], json!("method_incompatible"));
    }
    for (index, item) in items[fits..].iter().enumerate() {
        assert_eq!(
            *item,
            stopped(
                &format!("m{:05}", fits + index),
                "check_output_bytes",
                MAX_CHECK_OUTPUT_BYTES,
                (fits + 1) * each
            )
        );
    }
}

/// Measuring a method's text stops at the first string that takes it past
/// the output bound, so `observed` is at most the bound plus that string,
/// however much more text the method has. Measuring it all would cost time
/// in the length of every name times the paths and reports it is on, which
/// the step bounds do not count: a reviewer's 208 KB request of this shape
/// took 70 s to measure 1,000 warnings' paths of long names.
#[test]
fn measuring_the_output_stops_at_the_bound() {
    // A recursive written record whose two fields have long names, spelled
    // once, against a live chain of distinct records labelled by the names'
    // ids: every warning path holds about 30 long-named segments, and the
    // method reports 1,000 of them, about 30 MB of text.
    let long = 1_000;
    let (a, b) = ("a".repeat(long), "b".repeat(long));
    let (id_a, id_b) = (candid::idl_hash(&a), candid::idl_hash(&b));
    let written =
        format!("type W = opt record {{ {a} : W; {b} : W }};\nservice : {{ f : () -> (W) }}\n");
    let mut live = String::from("type D0 = text;\n");
    for level in 1..=10 {
        live.push_str(&format!(
            "type D{level} = opt record {{ {id_a} : D{p}; {id_b} : D{p} }};\n",
            p = level - 1
        ));
    }
    live.push_str("type C0 = D10;\n");
    for level in 1..=20 {
        live.push_str(&format!(
            "type C{level} = record {{ {id_a} : C{p} }};\n",
            p = level - 1
        ));
    }
    live.push_str("service : { f : () -> (C20) }\n");
    // The largest string is one segment: `.` and a name.
    let segment = 1 + long;
    for limit in [10_000, MAX_CHECK_OUTPUT_BYTES] {
        let options = CheckOptions {
            output_byte_limit: limit,
            ..CheckOptions::default()
        };
        let response = check_with(&written, &live, options);
        let observed = response["diagnostics"][0]["resource_limit"]["observed"]
            .as_u64()
            .unwrap() as usize;
        assert!(
            observed > limit && observed <= limit + segment,
            "{observed}"
        );
        assert_eq!(
            response["diagnostics"],
            json!([stopped("f", "check_output_bytes", limit, observed)])
        );
    }

    // A 100,000-byte method name, on each of 1,001 reports (Codex's 1,000
    // warnings and the warning-bound notice), about 100 MB of text: the
    // name is measured until the count passes the bound, and not after.
    let name = "m".repeat(100_000);
    let source = |content: &str| shared_doubling(content, 1).replace("m00000", &name);
    let response = check(&source("nat"), &source("text"));
    let observed = response["diagnostics"][0]["resource_limit"]["observed"]
        .as_u64()
        .unwrap() as usize;
    assert!(
        observed > MAX_CHECK_OUTPUT_BYTES && observed <= MAX_CHECK_OUTPUT_BYTES + name.len(),
        "{observed}"
    );
    assert_eq!(
        response["diagnostics"],
        json!([stopped(
            &name,
            "check_output_bytes",
            MAX_CHECK_OUTPUT_BYTES,
            observed
        )])
    );
}

/// Once an aggregate bound is reached, every method not yet decided fails
/// closed with that one diagnostic: not its `method_missing`, not its
/// `mode_changed`, not its own verdict.
#[test]
fn an_exhausted_check_fails_closed_for_every_method_left() {
    let written = "type R = record { a : nat; b : text; c : bool };
service : { a : () -> (R); b : () -> (R); c : () -> (R); d : () -> (R) query; e : () -> (R) }";
    let live = "type R = record { a : nat; b : text; c : bool };
type S = record { a : nat; b : text };
service : { a : () -> (R); b : () -> (R); d : () -> (R); e : () -> (S) }";
    let summary_with = |total_step_limit: usize| {
        let options = CheckOptions {
            total_step_limit,
            ..CheckOptions::default()
        };
        check_with(written, live, options)
    };
    // Below the bound, each method's own verdict.
    let response = summary_with(MAX_CHECK_TOTAL_STEPS);
    assert_eq!(
        summary(&response),
        [
            ("method_missing".to_string(), "c".to_string(), None),
            ("mode_changed".to_string(), "d".to_string(), None),
            (
                "method_incompatible".to_string(),
                "e".to_string(),
                Some("$results[0].c".to_string())
            ),
        ]
    );
    // `a` costs nine steps; `b` reaches a bound of 12 at the third step of
    // its walk, and `c`, `d` and `e` are not decided.
    let response = summary_with(12);
    assert_eq!(response["compatible"], json!(false), "{response}");
    assert_eq!(
        response["diagnostics"],
        json!(["b", "c", "d", "e"]
            .iter()
            .map(|method| stopped(method, "check_total_steps", 12, 13))
            .collect::<Vec<_>>())
    );
}

/// The fewest steps of the whole check under which `written` against `live`
/// reaches no `check_total_steps` bound, after checking that one step fewer
/// fails the last method closed at exactly that many.
fn total_steps_needed(written: &str, live: &str) -> usize {
    let run = |total_step_limit: usize| {
        let options = CheckOptions {
            total_step_limit,
            ..CheckOptions::default()
        };
        check_with(written, live, options)
    };
    let reached = |response: &Value| {
        response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_limit"]["resource"] == "check_total_steps")
    };
    // Reaching the bound is monotone in it: search for the edge.
    let mut high = 1;
    while reached(&run(high)) {
        high *= 2;
    }
    let mut low = 0;
    while low < high {
        let middle = (low + high) / 2;
        if reached(&run(middle)) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let needed = low;
    assert!(needed > 0);
    let short = run(needed - 1);
    let last = short["diagnostics"].as_array().unwrap().last().unwrap();
    assert_eq!(
        last["resource_limit"],
        json!({ "resource": "check_total_steps", "limit": needed - 1, "observed": needed })
    );
    needed
}

/// A step is a unit of work actually done, so the step bounds bound the
/// work: deciding a method (one step of the whole check's bound), visiting a
/// pair however it is answered (expanded, from a proof, at a pair in
/// progress, or from a failed pair, which also costs the segments of the
/// failure path it copies), recording a pair's failure (the segments of its
/// path), examining a field, arm, method or value, passing
/// over one the other side lacks, re-examining a proven pair's warning (the
/// pairs below it, at least one), and copying a warning's path (its
/// segments). Each case pins the total; dropping any one charge changes one
/// of them.
#[test]
fn every_unit_of_work_is_a_step() {
    let both = |source: &str| total_steps_needed(source, source);
    // A method alone: one step, so the bound caps the method count too.
    assert_eq!(both("service : { f : () -> () }"), 1);
    assert_eq!(
        both("service : { a : () -> (); b : () -> (); c : () -> () }"),
        3
    );
    // The method; the result examined and its record pair visited; three
    // fields examined and their pairs visited.
    let record = "type R = record { a : nat; b : text; c : bool };
";
    assert_eq!(
        both(&format!("{record}service : {{ f : () -> (R) }}")),
        1 + 2 + 6
    );
    // The second `R` is answered from the first one's proof: still a visit.
    assert_eq!(
        both(&format!("{record}service : {{ f : () -> (R, R) }}")),
        1 + 8 + 2
    );
    // `cons : L` meets `L` while its walk is in progress: still a visit. The
    // result and its pair, then two arms and their pairs.
    assert_eq!(
        both(
            "type L = variant { nil; cons : L };
service : { f : () -> (L) }"
        ),
        1 + 2 + 4
    );
    // Fields, arms and methods the other side lacks: `a` is passed over
    // before `b` is found (ids 97 < 98), `c` after it is not looked at. The
    // method, the result and its pair, `b` examined, `a` passed over, `b`'s
    // pair visited.
    let wide = "service : { f : () -> (record { a : nat; b : nat; c : nat }) }";
    let narrow = "service : { f : () -> (record { b : nat }) }";
    assert_eq!(total_steps_needed(narrow, wide), 1 + 2 + 3);
    let wide = "service : { f : () -> (variant { a; b; c }) }";
    let narrow = "service : { f : () -> (variant { b }) }";
    assert_eq!(total_steps_needed(wide, narrow), 1 + 2 + 3);
    let wide = "service : { f : () -> (service { a : () -> (); b : () -> (); c : () -> () }) }";
    let narrow = "service : { f : () -> (service { b : () -> () }) }";
    assert_eq!(total_steps_needed(narrow, wide), 1 + 2 + 3);
    // An absent field the written side may lack is examined.
    assert_eq!(
        total_steps_needed(
            "service : { f : () -> (record { a : nat; o : opt nat }) }",
            "service : { f : () -> (record { a : nat }) }"
        ),
        1 + 2 + 2 + 1
    );
    // A failed pair answers its second meeting, copying its one-segment
    // path, and every pair that fails pays for the path it records.
    // `$results[0]`: the result and its opt pair, the content's pair visited
    // in the probe, `v` examined and its pair visited (it fails, recording
    // no segment), `X`'s failure recorded (one segment), and the warning's
    // one-segment path copied. `$results[1]`: the result and its opt pair,
    // the record content's pair, `y` examined, `X`'s pair visited and
    // answered from its failure (one segment), the record's failure
    // recorded (two segments), and the warning.
    let failed = |leaf: &str| {
        format!(
            "type X = record {{ v : {leaf} }};
service : {{ f : () -> (opt X, opt record {{ y : X }}) }}"
        )
    };
    assert_eq!(
        total_steps_needed(&failed("nat"), &failed("text")),
        1 + (2 + 1 + 2 + 1 + 1) + (2 + 1 + 1 + 1 + 1 + 2 + 1)
    );
    // A proven pair's warning re-examined (one pair below it) and re-reported
    // under the second result (a two-segment path). `$results[0]`: the
    // result and its pair, `o` examined and its opt pair visited, the probe's
    // pair, and the warning's two-segment path. `$results[1]`: the result and
    // its pair, the warning re-examined and its path copied.
    let repeated = |leaf: &str| {
        format!(
            "type O = record {{ o : opt {leaf} }};
service : {{ f : () -> (O, O) }}"
        )
    };
    assert_eq!(
        total_steps_needed(&repeated("nat"), &repeated("text")),
        1 + (2 + 2 + 1 + 2) + (2 + 1 + 2)
    );
}

/// One wide record shared by many methods: each method walks every field
/// again, from empty state, and pays for it. At the shipped bound, 1,000
/// fields cost 2,003 steps a method, so the 4,993rd method takes the total
/// past ten million and it and every method after it fail closed. Before
/// each visit and field was a step, a method like these cost two.
#[test]
fn a_wide_record_shared_by_many_methods_reaches_the_total_step_bound() {
    let fields: Vec<String> = (0..1000).map(|index| format!("f{index} : nat")).collect();
    let list: Vec<String> = (0..5000)
        .map(|index| format!("m{index:05} : () -> (R);"))
        .collect();
    let source = format!(
        "type R = record {{ {} }};
service : {{ {} }}
",
        fields.join("; "),
        list.join(" ")
    );
    let per_method = 1 + 2 + 2 * 1000;
    let decided = MAX_CHECK_TOTAL_STEPS / per_method;
    assert_eq!(decided, 4992);
    let response = check(&source, &source);
    assert_eq!(response["compatible"], json!(false));
    let items = response["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 5000 - decided);
    for (index, item) in items.iter().enumerate() {
        assert_eq!(
            *item,
            stopped(
                &format!("m{:05}", decided + index),
                "check_total_steps",
                MAX_CHECK_TOTAL_STEPS,
                MAX_CHECK_TOTAL_STEPS + 1
            )
        );
    }
}

/// `written` against `live` needs exactly `steps` steps of the whole check:
/// under that bound no method fails closed, and one step under it the last
/// method does, at exactly `steps`.
fn assert_total_steps(written: &str, live: &str, steps: usize) {
    let run = |total_step_limit: usize| {
        let options = CheckOptions {
            total_step_limit,
            ..CheckOptions::default()
        };
        check_with(written, live, options)
    };
    let enough = run(steps);
    assert_eq!(enough["ok"], json!(true), "{enough}");
    assert!(
        enough["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["resource_limit"]["resource"] != "check_total_steps"),
        "{enough}"
    );
    let short = run(steps - 1);
    let last = short["diagnostics"].as_array().unwrap().last().unwrap();
    assert_eq!(
        last["resource_limit"],
        json!({ "resource": "check_total_steps", "limit": steps - 1, "observed": steps })
    );
}

/// Two method names that share an `idl_hash`, so every pair of strings that
/// differ only by putting one for the other after a common prefix does too.
const COLLIDING: [&str; 2] = ["jhwlzguu", "jsyrjsvk"];

/// Pairing the methods of two services reads no name: it compares their
/// ranks, integers computed once per check. So a method of a nested service
/// costs the same steps whether its name is one byte or half a megabyte
/// long, and whether or not its id collides with another method's, which
/// orders the two by name; and the verdicts agree with upstream's. Before
/// ranks, each pairing compared the two names in full, for one step. (Two
/// colliding names share a 100,000-byte prefix, not a longer one: the
/// compiler's canonicalization work, which charges every method name's bytes
/// on each round of its refinement, refuses two of 500,000.)
///
/// It pins step counts and verdicts, not time. Were the names compared
/// again, every count here would stay the same and the check would only be
/// slower, which no test here measures: the test that fails then is the unit
/// test `methods_are_paired_by_rank_and_never_by_name`, in `src/compat.rs`.
#[test]
fn a_long_or_colliding_method_name_costs_what_a_short_one_does() {
    let nested = |written: &[String], live: &[String]| {
        let source = |names: &[String]| {
            let methods: Vec<String> = names
                .iter()
                .map(|name| format!("{name} : () -> ()"))
                .collect();
            format!(
                "type S = service {{ {} }};\nservice : {{ f : () -> (S, S); g : (S) -> () }}\n",
                methods.join("; ")
            )
        };
        (source(written), source(live))
    };
    let named = |prefix: &str, suffixes: &[&str]| -> Vec<String> {
        suffixes
            .iter()
            .map(|suffix| format!("{prefix}{suffix}"))
            .collect()
    };
    // One method, the same on both sides. `f`: the method, then per
    // result the value and its pair, and the first time the service's
    // method examined and its func pair visited; the second result is
    // answered from the proof. `g`: the method, the value and its pair,
    // the service's method and its func pair.
    for name in ["a".to_string(), "a".repeat(500_000)] {
        let one = [name];
        let (written, live) = nested(&one, &one);
        assert_total_steps(&written, &live, (1 + 4 + 2) + (1 + 4));
        assert_eq!(check(&written, &live)["diagnostics"], json!([]));
    }
    let long = "p".repeat(100_000);
    for prefix in ["", long.as_str()] {
        // Two methods of one id, ordered by name: each examined and its
        // func pair visited.
        let both = named(prefix, &COLLIDING);
        let (written, live) = nested(&both, &both);
        assert_total_steps(&written, &live, (1 + 6 + 2) + (1 + 6));
        assert_eq!(check(&written, &live)["diagnostics"], json!([]));
        // The live service has both, the written one the second: `f`
        // passes over the first to find the second, and `g` finds the
        // first missing from the written service and fails there,
        // recording a one-segment failure.
        let second = named(prefix, &COLLIDING[1..]);
        let (written, live) = nested(&second, &both);
        assert_total_steps(&written, &live, (1 + 5 + 2) + (1 + 3 + 1));
        let response = check(&written, &live);
        assert_eq!(
            summary(&response),
            [(
                "method_incompatible".to_string(),
                "g".to_string(),
                Some(format!("$args[0]::{prefix}{}", COLLIDING[0]))
            )]
        );
        assert_eq!(disagreements(&written, &live, &response), []);
    }
}

/// A long-named method of a service type, met again and again: each of
/// many records holds the service type and a field that fails, so each
/// probe of one proves the service pair and then forgets it, and each of
/// several methods walks them all. Each pairing of the long-named method
/// is one step, as it is with a one-byte name, and every method is decided.
/// Before ranks, each pairing compared the two names in full: with 3,000
/// records and 300 methods, a 500,000-byte name took a native release
/// build 39 to 61 s, where a one-byte name took 1.9 to 2.0 s.
///
/// Like the test above, it pins step counts and diagnostics, not time:
/// comparing the names again would leave both as they are, so it is
/// `methods_are_paired_by_rank_and_never_by_name` that fails then.
#[test]
fn a_long_named_method_met_again_and_again_is_decided() {
    let (records, methods) = (100, 10);
    let shape = |name: &str, leaf: &str, opt: &str| {
        let mut source = format!("type S = service {{ {name} : () -> () }};\n");
        let mut fields = Vec::new();
        for index in 0..records {
            source.push_str(&format!(
                "type T{index} = record {{ 0 : S; {} : {leaf} }};\n",
                index + 1
            ));
            fields.push(format!("{index} : {opt} T{index}"));
        }
        fields.push(format!("{} : {leaf}", records + 10));
        source.push_str(&format!("type R = record {{ {} }};\n", fields.join("; ")));
        let list: Vec<String> = (0..methods)
            .map(|index| format!("m{index} : () -> ({opt} R);"))
            .collect();
        source.push_str(&format!("service : {{ {} }}\n", list.join(" ")));
        source
    };
    let short = (shape("a", "nat", "opt"), shape("a", "text", ""));
    let name = "a".repeat(500_000);
    let long = (shape(&name, "nat", "opt"), shape(&name, "text", ""));
    let response = check(&long.0, &long.1);
    assert_eq!(response["compatible"], json!(true), "{response}");
    assert_eq!(
        summary(&response),
        (0..methods)
            .map(|index| (
                "special_opt_rule".to_string(),
                format!("m{index}"),
                Some("$results[0]".to_string())
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        response["diagnostics"],
        check(&short.0, &short.1)["diagnostics"]
    );
    let steps = total_steps_needed(&short.0, &short.1);
    assert_total_steps(&long.0, &long.1, steps);
}

/// Below the aggregate bounds a method is reported exactly as it is when the
/// written service declares it alone, at each bound included; one step, one
/// warning or one byte under what the check needs, the method that would
/// pass it is not, so a method depends on the others only once a bound is
/// reached.
#[test]
fn below_the_aggregate_bounds_a_method_is_reported_as_it_is_alone() {
    let written_types = "type R = record { a : nat; b : text; c : bool };
type O = record { o : opt nat; p : opt nat };
type X = record { x : nat };\n";
    let live_types = "type R = record { a : nat; b : text; c : bool };
type O = record { o : opt text; p : opt nat };
type X = record { x : text };\n";
    let methods = [
        ("f", "f : () -> (R);", "f : () -> (R);"),
        ("g", "g : () -> (O);", "g : () -> (O);"),
        ("h", "h : (nat) -> (R) query;", "h : (nat) -> (R);"),
        ("k", "k : () -> (X);", "k : () -> (X);"),
    ];
    let service =
        |types: &str, list: &[&str]| format!("{types}service : {{ {} }}\n", list.join(" "));
    let written = service(written_types, &methods.map(|method| method.1));
    let live = service(live_types, &methods.map(|method| method.2));
    let run = |options: CheckOptions| check_with(&written, &live, options);
    let steps = |total_step_limit: usize| CheckOptions {
        total_step_limit,
        ..CheckOptions::default()
    };
    // Each method alone, against the full live service.
    let alone: Vec<Value> = methods
        .iter()
        .flat_map(|(_, method, _)| {
            check(&service(written_types, &[method]), &live)["diagnostics"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect();
    assert_eq!(
        summary(&json!({ "diagnostics": alone })),
        [
            (
                "special_opt_rule".to_string(),
                "g".to_string(),
                Some("$results[0].o".to_string())
            ),
            ("mode_changed".to_string(), "h".to_string(), None),
            (
                "method_incompatible".to_string(),
                "k".to_string(),
                Some("$results[0].x".to_string())
            ),
        ]
    );
    assert_eq!(run(CheckOptions::default())["diagnostics"], json!(alone));

    // The work bound: the fewest steps the whole check needs.
    let needed = (1..1000)
        .find(|limit| {
            run(steps(*limit))["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["resource_limit"]["resource"] != "check_total_steps")
        })
        .unwrap();
    assert_eq!(run(steps(needed))["diagnostics"], json!(alone));
    let response = run(steps(needed - 1));
    assert_eq!(
        response["diagnostics"].as_array().unwrap().last().unwrap(),
        &stopped("k", "check_total_steps", needed - 1, needed)
    );

    // The warning bound: `g` reports the one warning.
    let warnings = |total_warning_limit: usize| CheckOptions {
        total_warning_limit,
        ..CheckOptions::default()
    };
    assert_eq!(run(warnings(1))["diagnostics"], json!(alone));
    assert_eq!(
        run(warnings(0))["diagnostics"],
        json!(["g", "h", "k"]
            .iter()
            .map(|method| stopped(method, "check_total_warnings", 0, 1))
            .collect::<Vec<_>>())
    );

    // The output bound: the text of every diagnostic.
    let text: usize = alone
        .iter()
        .flat_map(|item| {
            ["method", "path", "message"].map(|key| item[key].as_str().map_or(0, str::len))
        })
        .sum();
    let output = |output_byte_limit: usize| CheckOptions {
        output_byte_limit,
        ..CheckOptions::default()
    };
    assert_eq!(run(output(text))["diagnostics"], json!(alone));
    let response = run(output(text - 1));
    assert_eq!(
        response["diagnostics"].as_array().unwrap().last().unwrap(),
        &stopped("k", "check_output_bytes", text - 1, text)
    );

    // The warning bound: each of ten methods sharing Codex's graph reports
    // what it reports alone.
    let together = check(&shared_doubling("nat", 10), &shared_doubling("text", 10));
    let one = check(&shared_doubling("nat", 1), &shared_doubling("text", 10));
    let items = together["diagnostics"].as_array().unwrap();
    for index in 0..10 {
        let name = format!("m{index:05}");
        let mine: Vec<Value> = items
            .iter()
            .filter(|item| item["method"] == json!(name))
            .map(|item| {
                let mut item = item.clone();
                item["method"] = json!("m00000");
                item
            })
            .collect();
        assert_eq!(json!(mine), one["diagnostics"], "{name}");
    }
}

/// Hold one case's response to a walk that walks every pair again instead
/// of re-reporting a proven pair's warnings: `Some(whether it warned)`, or
/// `None` when that walk, exponential in general, did not finish within its
/// own bound.
fn assert_equals_the_rewalking_walk(id: &str, written: &str, live: &str) -> Option<bool> {
    let reference = CheckOptions {
        step_limit: 2_000_000,
        // Bounded per method only, as it was before the aggregate bound.
        total_step_limit: usize::MAX,
        memo: false,
        ..CheckOptions::default()
    };
    let request =
        json!({ "written": { "source": written }, "live": { "source": live } }).to_string();
    let expected: Value =
        serde_json::from_str(&check_compatible_with(&request, reference)).unwrap();
    let items = expected["diagnostics"].as_array().unwrap();
    if items
        .iter()
        .any(|item| item["code"] == "resource_limit_exceeded")
    {
        return None;
    }
    let actual = check(written, live);
    assert_eq!(actual, expected, "{id}\n{written}\n{live}");
    Some(items.iter().any(|item| item["code"] == "special_opt_rule"))
}

/// The warnings re-reported from proven pairs are exactly those a walk that
/// walks every pair again reports: every path along which no pair of types
/// repeats. The whole response must be equal, errors included. A case the
/// re-walking walk cannot finish is left out, and few may be.
#[test]
fn re_reported_warnings_equal_a_walk_that_walks_every_pair_again() {
    let mut cases: Vec<(String, String, String)> = CASES
        .iter()
        .map(|case| {
            (
                case.name.to_string(),
                case.written.to_string(),
                case.live.to_string(),
            )
        })
        .collect();
    cases.extend(random_cases());
    let (mut compared, mut warned, mut left_out) = (0, 0, 0);
    for (id, written, live) in &cases {
        match assert_equals_the_rewalking_walk(id, written, live) {
            Some(warns) => {
                compared += 1;
                warned += usize::from(warns);
            }
            None => left_out += 1,
        }
    }
    eprintln!("{compared} compared ({warned} with warnings), {left_out} left out");
    assert!(
        left_out * 20 <= cases.len(),
        "{left_out} of {} left out",
        cases.len()
    );
    assert!(warned >= 50, "{warned} cases with warnings");
}

/// A type at the compiler's nesting bound is decided, not refused: the walk's
/// bound sits above every depth the compiler accepts.
#[test]
fn a_type_at_the_compilers_nesting_bound_is_decided() {
    let deep = |leaf: &str| {
        format!(
            "type Deep = {}{leaf};\nservice : {{ get : () -> (Deep) query }}\n",
            "vec ".repeat(254)
        )
    };
    // 254 vecs inside a method result of the actor is Candid depth 256, the
    // compiler's bound: one vec more is refused.
    let over = format!(
        "type Deep = {}nat;\nservice : {{ get : () -> (Deep) query }}\n",
        "vec ".repeat(255)
    );
    assert_eq!(
        check(&over, &over)["diagnostics"][0]["resource_limit"]["resource"],
        json!("type_depth")
    );
    let response = check(&deep("nat"), &deep("nat"));
    assert_eq!(response["compatible"], json!(true), "{response}");
    assert_eq!(response["diagnostics"], json!([]));
    let response = check(&deep("nat"), &deep("int"));
    assert_eq!(response["compatible"], json!(false), "{response}");
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("method_incompatible")
    );
    assert_eq!(
        response["diagnostics"][0]["path"],
        json!(format!("$results[0]{}", "[*]".repeat(254)))
    );
}

// ---------------------------------------------------------------------------
// The differential against upstream `candid`.
// ---------------------------------------------------------------------------

/// Rename every type variable so two sources can share one environment.
fn renamed(ty: &Type, prefix: &str) -> Type {
    let inner = match ty.as_ref() {
        TypeInner::Var(name) => TypeInner::Var(format!("{prefix}{name}")),
        TypeInner::Opt(inner) => TypeInner::Opt(renamed(inner, prefix)),
        TypeInner::Vec(inner) => TypeInner::Vec(renamed(inner, prefix)),
        TypeInner::Record(fields) => TypeInner::Record(renamed_fields(fields, prefix)),
        TypeInner::Variant(fields) => TypeInner::Variant(renamed_fields(fields, prefix)),
        TypeInner::Func(function) => TypeInner::Func(Function {
            modes: function.modes.clone(),
            args: function.args.iter().map(|t| renamed(t, prefix)).collect(),
            rets: function.rets.iter().map(|t| renamed(t, prefix)).collect(),
        }),
        TypeInner::Service(methods) => TypeInner::Service(
            methods
                .iter()
                .map(|(name, t)| (name.clone(), renamed(t, prefix)))
                .collect(),
        ),
        TypeInner::Class(args, service) => TypeInner::Class(
            args.iter().map(|t| renamed(t, prefix)).collect(),
            renamed(service, prefix),
        ),
        other => other.clone(),
    };
    inner.into()
}

fn renamed_fields(fields: &[Field], prefix: &str) -> Vec<Field> {
    fields
        .iter()
        .map(|field| Field {
            id: field.id.clone(),
            ty: renamed(&field.ty, prefix),
        })
        .collect()
}

/// Parse and check one source with upstream, adding its declarations to
/// `env` under `prefix`; returns its actor's methods.
fn upstream_side(env: &mut TypeEnv, source: &str, prefix: &str) -> BTreeMap<String, Type> {
    let program: IDLProg = source.parse().unwrap();
    let mut own = TypeEnv::new();
    let actor = check_prog(&mut own, &program).unwrap().expect("a service");
    for (name, ty) in own.0.iter() {
        env.0.insert(format!("{prefix}{name}"), renamed(ty, prefix));
    }
    let mut service = renamed(&actor, prefix);
    loop {
        service = match service.as_ref() {
            TypeInner::Class(_, inner) => inner.clone(),
            TypeInner::Var(name) => env.0[name].clone(),
            TypeInner::Service(methods) => return methods.iter().cloned().collect(),
            other => panic!("not a service: {other:?}"),
        };
    }
}

/// Upstream's verdicts per written method: `(spec rules, special opt rule
/// refused)`; `None` for a method the live service lacks.
fn upstream(written: &str, live: &str) -> BTreeMap<String, Option<(bool, bool)>> {
    let mut env = TypeEnv::new();
    let written = upstream_side(&mut env, written, "W_");
    let live = upstream_side(&mut env, live, "L_");
    written
        .iter()
        .map(|(name, written_ty)| {
            let verdict = live.get(name).map(|live_ty| {
                let lenient = subtype_with_config(
                    OptReport::Silence,
                    &mut Default::default(),
                    &env,
                    live_ty,
                    written_ty,
                )
                .is_ok();
                let strict = subtype_with_config(
                    OptReport::Error,
                    &mut Default::default(),
                    &env,
                    live_ty,
                    written_ty,
                )
                .is_ok();
                (lenient, strict)
            });
            (name.clone(), verdict)
        })
        .collect()
}

/// This check's verdicts in the same shape as [`upstream`].
fn ours(response: &Value, methods: &[String]) -> BTreeMap<String, Option<(bool, bool)>> {
    methods
        .iter()
        .map(|name| {
            let items: Vec<&Value> = response["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["method"] == json!(name))
                .collect();
            if items.iter().any(|item| item["code"] == "method_missing") {
                return (name.clone(), None);
            }
            let errors = items.iter().any(|item| item["severity"] == "error");
            let warnings = items.iter().any(|item| item["severity"] == "warning");
            (name.clone(), Some((!errors, !errors && !warnings)))
        })
        .collect()
}

/// Compare one case with upstream; returns the disagreements as `(method,
/// verdict kind)`.
fn disagreements(written: &str, live: &str, response: &Value) -> Vec<(String, &'static str)> {
    let reference = upstream(written, live);
    let methods: Vec<String> = reference.keys().cloned().collect();
    let mine = ours(response, &methods);
    let mut found = Vec::new();
    for name in &methods {
        match (&reference[name], &mine[name]) {
            (None, None) => {}
            (Some((lenient, strict)), Some((our_lenient, our_strict))) => {
                if lenient != our_lenient {
                    found.push((name.clone(), "lenient"));
                }
                if strict != our_strict {
                    found.push((name.clone(), "strict"));
                }
            }
            _ => found.push((name.clone(), "presence")),
        }
    }
    // The service-level verdict, too.
    let service_ok = reference
        .values()
        .all(|v| v.is_some_and(|(lenient, _)| lenient));
    if service_ok != (response["compatible"] == json!(true))
        && !found.iter().any(|(_, kind)| *kind == "lenient")
    {
        found.push(("*".to_string(), "service"));
    }
    found
}

fn assert_triaged(case: &str, found: Vec<(String, &'static str)>, observed: &mut BTreeSet<String>) {
    for (method, kind) in found {
        let known = KNOWN_DIVERGENCES
            .iter()
            .any(|(c, m, k, _)| *c == case && *m == method && *k == kind);
        assert!(
            known,
            "{case}: method {method} disagrees with upstream on the {kind} verdict and is not a triaged divergence"
        );
        observed.insert(format!("{case}/{method}/{kind}"));
    }
}

#[test]
fn hand_cases_agree_with_upstream_except_the_triaged_divergences() {
    let mut observed = BTreeSet::new();
    for case in CASES {
        let response = check(case.written, case.live);
        assert_triaged(
            case.name,
            disagreements(case.written, case.live, &response),
            &mut observed,
        );
    }
    for (case, method, kind, _) in KNOWN_DIVERGENCES {
        if CASES.iter().any(|c| c.name == *case) {
            assert!(
                observed.contains(&format!("{case}/{method}/{kind}")),
                "{case}/{method}/{kind} is listed as a divergence but now agrees with upstream: remove it"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The seeded random campaign.
// ---------------------------------------------------------------------------

/// SplitMix64: small, seedable, and the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

#[derive(Debug, Clone)]
enum Ty {
    Prim(&'static str),
    Opt(Box<Ty>),
    Vec(Box<Ty>),
    Record(Vec<(&'static str, Ty)>),
    Variant(Vec<(&'static str, Ty)>),
    Ref(usize),
    Func(Vec<Ty>, Vec<Ty>, &'static str),
    /// A service type: its methods, each a `Func`.
    Service(Vec<(&'static str, Ty)>),
}

const SERVICE_METHODS: &[&str] = &["f", "g"];

/// A func member of a service type: a query or an update, never oneway, so
/// any results it is given stay valid.
fn gen_func(rng: &mut Rng, decls: usize, depth: usize) -> Ty {
    let args = (0..rng.below(3))
        .map(|_| gen_ty(rng, decls, depth))
        .collect();
    let rets = (0..rng.below(3))
        .map(|_| gen_ty(rng, decls, depth))
        .collect();
    Ty::Func(args, rets, MODES[rng.below(2)])
}

const PRIMS: &[&str] = &[
    "nat",
    "int",
    "nat8",
    "nat64",
    "int32",
    "text",
    "bool",
    "null",
    "reserved",
    "empty",
    "principal",
    "float64",
];
const LABELS: &[&str] = &["a", "b", "c", "d", "e"];
const MODES: &[&str] = &["", " query", " oneway", " composite_query"];

#[derive(Debug, Clone)]
struct Iface {
    decls: Vec<Ty>,
    methods: Vec<(&'static str, Vec<Ty>, Vec<Ty>, &'static str)>,
}

fn gen_ty(rng: &mut Rng, decls: usize, depth: usize) -> Ty {
    let leaf = depth == 0 || rng.chance(35);
    if leaf {
        return if decls > 0 && rng.chance(40) {
            Ty::Ref(rng.below(decls))
        } else {
            Ty::Prim(PRIMS[rng.below(PRIMS.len())])
        };
    }
    match rng.below(6) {
        0 => Ty::Opt(Box::new(gen_ty(rng, decls, depth - 1))),
        1 => Ty::Vec(Box::new(gen_ty(rng, decls, depth - 1))),
        2 | 3 => {
            let mut fields = Vec::new();
            for label in LABELS {
                if rng.chance(45) {
                    fields.push((*label, gen_ty(rng, decls, depth - 1)));
                }
            }
            if depth.is_multiple_of(2) {
                Ty::Record(fields)
            } else {
                Ty::Variant(fields)
            }
        }
        4 => gen_func(rng, decls, depth - 1),
        _ => Ty::Service(
            SERVICE_METHODS
                .iter()
                .take(1 + rng.below(SERVICE_METHODS.len()))
                .map(|name| (*name, gen_func(rng, decls, depth - 1)))
                .collect(),
        ),
    }
}

fn gen_iface(rng: &mut Rng) -> Iface {
    let count = rng.below(4);
    let decls = (0..count)
        .map(|_| {
            // A declaration is always a constructor, never a bare name, so no
            // two declarations can only name each other.
            match gen_ty(rng, count, 3) {
                Ty::Ref(_) | Ty::Prim(_) => Ty::Record(vec![("a", gen_ty(rng, count, 2))]),
                other => other,
            }
        })
        .collect();
    let methods = ["m0", "m1", "m2"]
        .into_iter()
        .take(1 + rng.below(3))
        .map(|name| {
            let args = (0..rng.below(3)).map(|_| gen_ty(rng, count, 2)).collect();
            let rets = (0..rng.below(3)).map(|_| gen_ty(rng, count, 2)).collect();
            (name, args, rets, MODES[rng.below(MODES.len())])
        })
        .collect();
    Iface { decls, methods }
}

/// Apply one random edit somewhere in a type; returns whether one applied.
fn mutate_ty(ty: &mut Ty, rng: &mut Rng, decls: usize) -> bool {
    let here = rng.chance(30);
    match ty {
        Ty::Prim(name) => {
            *name = ["nat", "int", "text", "null", "reserved", "empty", "nat8"][rng.below(7)];
            true
        }
        Ty::Opt(inner) if !here => mutate_ty(inner, rng, decls),
        Ty::Opt(inner) => {
            *ty = (**inner).clone();
            true
        }
        Ty::Vec(inner) if !here => mutate_ty(inner, rng, decls),
        Ty::Record(fields) | Ty::Variant(fields) if !here && !fields.is_empty() => {
            let at = rng.below(fields.len());
            mutate_ty(&mut fields[at].1, rng, decls)
        }
        Ty::Record(fields) | Ty::Variant(fields) => {
            if !fields.is_empty() && rng.chance(50) {
                fields.remove(rng.below(fields.len()));
            } else {
                let free: Vec<&'static str> = LABELS
                    .iter()
                    .copied()
                    .filter(|label| fields.iter().all(|(used, _)| used != label))
                    .collect();
                if free.is_empty() {
                    return false;
                }
                let label = free[rng.below(free.len())];
                let added = if rng.chance(50) {
                    Ty::Opt(Box::new(gen_ty(rng, decls, 1)))
                } else {
                    gen_ty(rng, decls, 1)
                };
                fields.push((label, added));
            }
            true
        }
        Ty::Func(args, rets, mode) => {
            match rng.below(4) {
                0 => *mode = MODES[rng.below(2)],
                1 if !args.is_empty() => {
                    let at = rng.below(args.len());
                    return mutate_ty(&mut args[at], rng, decls);
                }
                2 if !rets.is_empty() => {
                    let at = rng.below(rets.len());
                    return mutate_ty(&mut rets[at], rng, decls);
                }
                _ => rets.push(gen_ty(rng, decls, 1)),
            }
            true
        }
        Ty::Service(methods) if !here => {
            let at = rng.below(methods.len());
            mutate_ty(&mut methods[at].1, rng, decls)
        }
        Ty::Service(methods) => {
            if methods.len() > 1 && rng.chance(50) {
                methods.remove(rng.below(methods.len()));
            } else if let Some(name) = SERVICE_METHODS
                .iter()
                .find(|name| methods.iter().all(|(used, _)| used != *name))
            {
                methods.push((name, gen_func(rng, decls, 1)));
            } else {
                return false;
            }
            true
        }
        Ty::Ref(_) => {
            *ty = Ty::Opt(Box::new(ty.clone()));
            true
        }
        other => {
            *other = Ty::Opt(Box::new(other.clone()));
            true
        }
    }
}

fn mutate(iface: &Iface, rng: &mut Rng) -> Iface {
    let mut next = iface.clone();
    for _ in 0..1 + rng.below(3) {
        match rng.below(10) {
            0 => {
                if next.methods.len() > 1 {
                    let at = rng.below(next.methods.len());
                    next.methods.remove(at);
                }
            }
            1 => {
                if !next.methods.iter().any(|m| m.0 == "extra") {
                    next.methods.push(("extra", Vec::new(), Vec::new(), ""));
                }
            }
            2 => {
                let at = rng.below(next.methods.len());
                next.methods[at].3 = MODES[rng.below(MODES.len())];
            }
            3 | 4 if !next.decls.is_empty() => {
                let at = rng.below(next.decls.len());
                let count = next.decls.len();
                mutate_ty(&mut next.decls[at], rng, count);
            }
            _ => {
                let at = rng.below(next.methods.len());
                let count = next.decls.len();
                let method = &mut next.methods[at];
                let list = if rng.chance(50) {
                    &mut method.1
                } else {
                    &mut method.2
                };
                if list.is_empty() || rng.chance(20) {
                    list.push(if rng.chance(50) {
                        Ty::Opt(Box::new(gen_ty(rng, count, 1)))
                    } else {
                        gen_ty(rng, count, 1)
                    });
                } else {
                    let at = rng.below(list.len());
                    mutate_ty(&mut list[at], rng, count);
                }
            }
        }
    }
    next
}

fn print_ty(ty: &Ty) -> String {
    match ty {
        Ty::Prim(name) => name.to_string(),
        Ty::Opt(inner) => format!("opt {}", print_ty(inner)),
        Ty::Vec(inner) => format!("vec {}", print_ty(inner)),
        Ty::Record(fields) => format!(
            "record {{ {} }}",
            fields
                .iter()
                .map(|(label, ty)| format!("{label} : {}", print_ty(ty)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        Ty::Variant(fields) => format!(
            "variant {{ {} }}",
            fields
                .iter()
                .map(|(label, ty)| format!("{label} : {}", print_ty(ty)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        Ty::Ref(index) => format!("T{index}"),
        Ty::Func(args, rets, mode) => format!("func {}", print_signature(args, rets, mode)),
        Ty::Service(methods) => format!(
            "service {{ {} }}",
            methods
                .iter()
                .map(|(name, ty)| match ty {
                    Ty::Func(args, rets, mode) => {
                        format!("{name} : {}", print_signature(args, rets, mode))
                    }
                    other => format!("{name} : {}", print_ty(other)),
                })
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

fn print_signature(args: &[Ty], rets: &[Ty], mode: &str) -> String {
    let list = |types: &[Ty]| types.iter().map(print_ty).collect::<Vec<_>>().join(", ");
    format!("({}) -> ({}){mode}", list(args), list(rets))
}

fn print_iface(iface: &Iface) -> String {
    let mut source = String::new();
    for (index, ty) in iface.decls.iter().enumerate() {
        source.push_str(&format!("type T{index} = {};\n", print_ty(ty)));
    }
    source.push_str("service : {\n");
    for (name, args, rets, mode) in &iface.methods {
        source.push_str(&format!(
            "  {name} : {};\n",
            print_signature(args, rets, mode)
        ));
    }
    source.push_str("}\n");
    source
}

/// The committed campaign: this many cases from seed 1, each a random
/// interface and a mutation of it, compared in both directions. It is
/// bounded: further instances of upstream's unsound memo exist past it, which
/// [`an_extended_campaign_disagrees_only_in_the_memos_direction`] looks for.
const RANDOM_CASES: u64 = 600;

fn random_cases() -> Vec<(String, String, String)> {
    random_cases_up_to(RANDOM_CASES)
}

/// Draw `count` cases: `(id, written, live)`. A draft either compiler
/// refuses is redrawn from the next seed.
fn random_cases_up_to(count: u64) -> Vec<(String, String, String)> {
    let mut cases = Vec::new();
    let mut seed = 0;
    while (cases.len() as u64) < count {
        seed += 1;
        let mut rng = Rng(seed);
        let base = gen_iface(&mut rng);
        let edited = mutate(&base, &mut rng);
        let (base, edited) = (print_iface(&base), print_iface(&edited));
        if compile_did(&base).is_err() || compile_did(&edited).is_err() {
            continue;
        }
        if upstream_accepts(&base) && upstream_accepts(&edited) {
            let (written, live) = if seed.is_multiple_of(2) {
                (base, edited)
            } else {
                (edited, base)
            };
            cases.push((format!("r{seed}"), written, live));
        }
    }
    cases
}

fn upstream_accepts(source: &str) -> bool {
    let Ok(program) = source.parse::<IDLProg>() else {
        return false;
    };
    check_prog(&mut TypeEnv::new(), &program).is_ok()
}

#[test]
fn a_seeded_random_campaign_agrees_with_upstream() {
    let mut observed = BTreeSet::new();
    let mut verdicts = BTreeMap::<&str, usize>::new();
    let mut with_services = 0;
    for (id, written, live) in random_cases() {
        if written.contains("service {") || live.contains("service {") {
            with_services += 1;
        }
        let response = check(&written, &live);
        assert_eq!(
            response["ok"],
            json!(true),
            "{id}: {response}\n{written}\n{live}"
        );
        *verdicts
            .entry(if response["compatible"] == json!(true) {
                "compatible"
            } else {
                "incompatible"
            })
            .or_default() += 1;
        let found = disagreements(&written, &live, &response);
        if !found.is_empty() {
            eprintln!("{id}\n--- written\n{written}--- live\n{live}{response}");
        }
        assert_triaged(&id, found, &mut observed);
    }
    // The campaign must exercise both verdicts substantially, or it proves
    // little about either.
    assert!(verdicts["compatible"] >= 100, "{verdicts:?}");
    assert!(verdicts["incompatible"] >= 100, "{verdicts:?}");
    // Service-typed values, whose methods are contravariant in their own
    // arguments, are drawn too.
    assert!(
        with_services >= 100,
        "{with_services} cases with a service type"
    );
}

// ---------------------------------------------------------------------------
// The agreement file for the TypeScript decoder.
// ---------------------------------------------------------------------------

/// The canonical Contract document, or `None` when the generator would
/// leave something out (the TypeScript loader then drops it too, and a
/// per-method comparison would be over different services).
fn contract_document(source: &str) -> Option<Value> {
    let compilation = compile_did(source).unwrap();
    let names = TsNames::from_source_info(compilation.source_info().unwrap());
    let generated = generate_module(compilation.contract(), &names, &TsOptions::default()).ok()?;
    if !generated.omitted.is_empty() {
        return None;
    }
    Some(serde_json::from_str(&compilation.contract().to_json_pretty().unwrap()).unwrap())
}

/// How many random cases the agreement file carries, after the hand cases.
const AGREEMENT_RANDOM_CASES: usize = 150;

#[test]
fn the_agreement_file_is_current() {
    let mut entries = Vec::new();
    let hand = CASES.iter().map(|case| {
        (
            case.name.to_string(),
            case.written.to_string(),
            case.live.to_string(),
        )
    });
    let random = random_cases().into_iter().take(AGREEMENT_RANDOM_CASES);
    for (name, written, live) in hand.chain(random) {
        let (Some(written_contract), Some(live_contract)) =
            (contract_document(&written), contract_document(&live))
        else {
            continue;
        };
        let response = check(&written, &live);
        let methods: Vec<String> = upstream(&written, &live).keys().cloned().collect();
        let verdicts: serde_json::Map<String, Value> = ours(&response, &methods)
            .into_iter()
            .map(|(name, verdict)| (name, json!(verdict.is_some_and(|(lenient, _)| lenient))))
            .collect();
        entries.push(json!({
            "name": name,
            "compatible": response["compatible"],
            "methods": verdicts,
            "written": written_contract,
            "live": live_contract,
        }));
    }
    let mut text = String::from("{\n");
    text.push_str("\"about\": \"Generated by crates/candid-core-wasm/tests/compatibility.rs: each case's written and live Contracts and checkCompatible's verdict per written method (true: the live method is a subtype of the written one under the spec's rules). ts/tests/compat-agreement.test.ts holds the decoder's subtype check to every verdict. Regenerate with UPDATE_GOLDENS=1 cargo test --test compatibility in crates/candid-core-wasm.\",\n");
    text.push_str("\"cases\": [\n");
    for (index, entry) in entries.iter().enumerate() {
        text.push_str(&serde_json::to_string(entry).unwrap());
        text.push_str(if index + 1 < entries.len() {
            ",\n"
        } else {
            "\n"
        });
    }
    text.push_str("]\n}\n");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../candid-core-ts/tests/goldens/compat/agreement.json");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == text,
        "{} is stale: regenerate with UPDATE_GOLDENS=1 cargo test --test compatibility and review the diff",
        path.display()
    );
    assert!(entries.len() >= 120, "only {} cases", entries.len());
}

/// A longer campaign, run on demand:
/// `COMPAT_CAMPAIGN_CASES=15000 cargo test --test compatibility -- --ignored`.
/// Past the committed seeds, upstream's unsound memo is met again; each
/// disagreement must be in its direction (upstream accepts under the spec's
/// rules but refuses under strict opt reporting, and this check refuses), so
/// that none can be a case this check accepts and upstream refuses. Every
/// case must also equal the re-walking walk, as in
/// `re_reported_warnings_equal_a_walk_that_walks_every_pair_again`.
#[test]
#[ignore = "an on-demand campaign; the committed ones are a_seeded_random_campaign_agrees_with_upstream and re_reported_warnings_equal_a_walk_that_walks_every_pair_again"]
fn an_extended_campaign_disagrees_only_in_the_memos_direction() {
    let count = std::env::var("COMPAT_CAMPAIGN_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5_000);
    let mut memo = Vec::new();
    let (mut rewalked, mut warned) = (0, 0);
    for (id, written, live) in random_cases_up_to(count) {
        if let Some(warns) = assert_equals_the_rewalking_walk(&id, &written, &live) {
            rewalked += 1;
            warned += usize::from(warns);
        }
        let response = check(&written, &live);
        let found = disagreements(&written, &live, &response);
        if found.is_empty() {
            continue;
        }
        let reference = upstream(&written, &live);
        for (method, kind) in &found {
            let upstream_verdict = reference.get(method).copied().flatten();
            assert!(
                *kind == "lenient" && upstream_verdict == Some((true, false)),
                "{id}: method {method} disagrees on the {kind} verdict outside the memo's direction\n--- written\n{written}--- live\n{live}{response}"
            );
            memo.push(format!("{id}/{method}"));
        }
    }
    eprintln!(
        "{count} cases; {} disagreements, all in the memo's direction: {memo:?}; {rewalked} equal to the re-walking walk ({warned} with warnings)",
        memo.len()
    );
}
