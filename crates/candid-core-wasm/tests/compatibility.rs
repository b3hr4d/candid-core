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
use candid_core_wasm::check_compatible;
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
];

/// Disagreements with upstream that were triaged by hand: `(case, method,
/// which verdict, reason)`. Every other disagreement fails the test.
const KNOWN_DIVERGENCES: &[(&str, &str, &str, &str)] = &[(
    "the_unsound_memo_case",
    "get",
    "lenient",
    "upstream candid's coinductive memo keeps a pair proven under an assumption a failed opt probe retracted (b3hr4d/candid-core#227, item 3); this check refuses, upstream accepts",
)];

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
    match rng.below(5) {
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
        _ => {
            let args = (0..rng.below(3))
                .map(|_| gen_ty(rng, decls, depth - 1))
                .collect();
            let rets = (0..rng.below(3))
                .map(|_| gen_ty(rng, decls, depth - 1))
                .collect();
            Ty::Func(args, rets, MODES[rng.below(2)])
        }
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
/// interface and a mutation of it, compared in both directions.
const RANDOM_CASES: u64 = 600;

/// Draw the campaign's cases: `(id, written, live)`. A draft either compiler
/// refuses is redrawn from the next seed.
fn random_cases() -> Vec<(String, String, String)> {
    let mut cases = Vec::new();
    let mut seed = 0;
    while (cases.len() as u64) < RANDOM_CASES {
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
    for (id, written, live) in random_cases() {
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
