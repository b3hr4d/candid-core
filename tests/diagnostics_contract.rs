//! Pins the unified diagnostic item contract from issue #19.
//!
//! One serializable item algebra backs compile diagnostics, Contract
//! violations, and HostValue violations. These tests assert the exact
//! serialized shapes: the legacy key sets stay byte-compatible, new optional
//! fields (`path`, source-scoped spans, `related`) appear only when their data
//! exists, materialized file identities never leak, and resource metadata
//! survives every conversion chain.

use candid_core::{
    compile_did, compile_did_with_context, compile_with_resolver, validate_host_value,
    CompileOptions, Contract, ContractJsonError, Diagnostic, DiagnosticPhase, HostValue, Limits,
    MemoryResolver, RuntimeContext, Severity, SourceSpan,
};
use serde_json::json;

fn compile_error_json(source: &str) -> serde_json::Value {
    let error = compile_did(source).expect_err("source must fail to compile");
    serde_json::to_value(&error.diagnostics).expect("diagnostics serialize")
}

#[test]
fn compile_diagnostics_keep_the_legacy_serialized_shape_exactly() {
    // The pre-#19 shape: code/phase/severity/message always present, exact
    // span with original offsets, expected-token notes, and none of the new
    // optional keys. Byte-for-byte via full-value equality.
    assert_eq!(
        compile_error_json("service : { broken: (nat) -> ( };"),
        json!([{
            "code": "did_parse_error",
            "phase": "parse",
            "severity": "error",
            "message": "Candid parser error: Unexpected token at bytes 31..32",
            "span": {
                "source_name": "memory:/inline.did",
                "start_byte": 31,
                "end_byte": 32,
            },
            "notes": [
                "Expects one of \")\", \"blob\", \"func\", \"id\", \"null\", \"opt\", \"principal\",\n\"record\", \"service\", \"text\", \"variant\", \"vec\"",
            ],
        }])
    );
}

#[test]
fn contract_violations_keep_the_legacy_serialized_shape_exactly() {
    let fixture = include_str!("fixtures/conformance/empty_actor.contract.json");
    let mut json: serde_json::Value = serde_json::from_str(fixture).unwrap();
    json["identities"]["contract"] = json!("not-a-valid-id");
    let error = match Contract::from_json(&json.to_string()) {
        Err(ContractJsonError::InvalidContract(error)) => error,
        other => panic!("expected an invalid-contract error, got {other:?}"),
    };
    // The pre-#19 violation shape: code/path/message only — no phase,
    // severity, span, notes, or related keys.
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([{
            "code": "invalid_contract_id_format",
            "path": "$.identities.contract",
            "message": "contract identity must use candid-core:contract:v1:sha256:<64 lowercase hex>",
        }])
    );
}

#[test]
fn host_value_violations_keep_the_legacy_serialized_shape_exactly() {
    let compilation = compile_did("type Deep = opt Deep; service : {};").unwrap();
    let contract = compilation.contract();
    let deep = contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == "Deep")
        .unwrap()
        .ty;
    let selector = contract.bind_type(deep).unwrap();

    let text = HostValue::text("hello");
    let error = validate_host_value(contract, &selector, &text, &Limits::default()).unwrap_err();
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([{
            "code": "host_value_kind_mismatch",
            "path": "$",
            "message": "expected opt, found text",
        }])
    );
}

#[test]
fn host_value_resource_chain_preserves_the_exact_triple_and_path() {
    let compilation = compile_did("type Deep = opt Deep; service : {};").unwrap();
    let contract = compilation.contract();
    let deep = contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == "Deep")
        .unwrap()
        .ty;
    let selector = contract.bind_type(deep).unwrap();

    let mut value = HostValue::null();
    for _ in 0..40 {
        value = HostValue::opt(Some(value), &Limits::default()).unwrap();
    }
    let error = validate_host_value(
        contract,
        &selector,
        &value,
        &Limits {
            max_value_depth: 3,
            ..Limits::default()
        },
    )
    .unwrap_err();
    // The {resource, limit, observed} triple survives untouched, and the
    // violation keeps its real value path rather than collapsing to "$".
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([{
            "code": "resource_limit_exceeded",
            "path": "$.value.value.value.value",
            "message": "value depth exceeds limit 3",
            "resource_limit": { "resource": "value_depth", "limit": 3, "observed": 4 },
        }])
    );
}

#[test]
fn compile_lowering_converts_structured_violations_without_stringification() {
    // Exhaust the canonicalization budget inside lowering. Before #19 this
    // surfaced as one "contract_lowering_error" whose message was the count-only
    // Display of the structured error; now every violation converts
    // item-by-item with its code, structured path, and resource triple.
    let context = RuntimeContext::new(Limits {
        max_canonicalization_work: 1,
        ..Limits::default()
    });
    let error =
        compile_did_with_context("service : {};", CompileOptions::default(), &context).unwrap_err();
    assert_eq!(
        serde_json::to_value(&error.diagnostics).unwrap(),
        json!([{
            "code": "resource_limit_exceeded",
            "phase": "lower",
            "severity": "error",
            "path": "$",
            "message": "$: resource canonicalization_work exceeded limit 1; observed 2",
            "resource_limit": { "resource": "canonicalization_work", "limit": 1, "observed": 2 },
        }])
    );
}

#[test]
fn imported_type_check_failures_name_logical_sources_only() {
    // A service import without a main service is only discovered by
    // `check_file` over the materialized bundle, where the importing text
    // spells the target as a numeric "N.did" inside a private temp directory.
    // The diagnostic must speak in logical source IDs, with a source-scoped
    // span whose offsets are absent (pretty-printed offsets are not original
    // offsets).
    let resolver = MemoryResolver::new()
        .with_source(
            "memory:/entry.did",
            "import service \"lib.did\";\nservice : {};\n",
        )
        .unwrap()
        .with_source("memory:/lib.did", "type T = nat;\n")
        .unwrap();
    let error = compile_with_resolver(
        "memory:/entry.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap_err();
    assert_eq!(
        serde_json::to_value(&error.diagnostics).unwrap(),
        json!([{
            "code": "did_type_check_error",
            "phase": "type_check",
            "severity": "error",
            "message": "Imported service file \"memory:/lib.did\" has no main service",
            "span": { "source_name": "memory:/lib.did" },
        }])
    );

    // Belt and braces against every known leak vector: the numeric
    // materialized name, the temp-directory prefix, and fabricated offsets.
    let rendered = serde_json::to_string(&error.diagnostics).unwrap();
    assert!(
        !rendered.contains("1.did"),
        "numeric name leaked: {rendered}"
    );
    assert!(
        !rendered.contains("candid-core-"),
        "materialized temp directory leaked: {rendered}"
    );
    let temp_dir = std::env::temp_dir().display().to_string();
    assert!(
        !rendered.contains(temp_dir.trim_end_matches('/')),
        "temp directory leaked: {rendered}"
    );
    assert!(
        !rendered.contains("start_byte"),
        "rewritten offsets must not be published: {rendered}"
    );
}

#[test]
fn lowering_structure_validation_converts_resource_failures_item_by_item() {
    // The sibling of the canonicalization chain: exhaust a limit that only
    // `validate_structure` observes (`type_nodes`), so the conversion at the
    // structure-validation seam is exercised, not just the canonicalize seam.
    let context = RuntimeContext::new(Limits {
        max_type_nodes: 1,
        ..Limits::default()
    });
    let error = compile_did_with_context(
        "type Pair = record { first: nat; second: text }; service : {};",
        CompileOptions::default(),
        &context,
    )
    .unwrap_err();
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    assert_eq!(diagnostic.phase, Some(DiagnosticPhase::Lower));
    assert_eq!(diagnostic.severity, Some(Severity::Error));
    assert_eq!(diagnostic.path.as_deref(), Some("$"));
    let info = diagnostic
        .resource_limit
        .as_ref()
        .expect("the resource triple must survive the lowering conversion");
    assert_eq!(info.resource, "type_nodes");
    assert_eq!(info.limit, 1);
    assert!(info.observed > 1);
    // The message is the structured violation's rendering — not a Display
    // flattening of the whole collection.
    assert_eq!(
        diagnostic.message,
        format!(
            "$: resource type_nodes exceeded limit 1; observed {}",
            info.observed
        )
    );
}

#[test]
fn quoted_user_labels_survive_the_materialized_boundary_untouched() {
    // `"0.did"` is a legal Candid text field label, and upstream renders it
    // raw-quoted inside type errors. The materialized-identity mapping must
    // not rewrite it into a source ID or fabricate a span from it.
    let resolver = MemoryResolver::new()
        .with_source(
            "memory:/entry.did",
            "type R = record { \"0.did\" : nat };\nservice : R;\n",
        )
        .unwrap();
    let error = compile_with_resolver(
        "memory:/entry.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap_err();
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "did_type_check_error");
    assert!(
        diagnostic.message.contains("\"0.did\""),
        "the user's own label must stay verbatim: {}",
        diagnostic.message
    );
    assert!(
        !diagnostic.message.contains("entry.did"),
        "no source ID may be spliced into user text: {}",
        diagnostic.message
    );
    assert_eq!(diagnostic.span, None, "no span may be fabricated");
}

#[test]
fn related_locations_serialize_and_round_trip_exactly() {
    let item = Diagnostic::compiler("did_parse_error", DiagnosticPhase::Parse, "m")
        .with_span(SourceSpan::exact(
            Some("memory:/entry.did".to_string()),
            3,
            4,
        ))
        .with_related(vec![
            candid_core::RelatedLocation {
                message: "first related".to_string(),
                span: Some(SourceSpan::exact(
                    Some("memory:/entry.did".to_string()),
                    8,
                    9,
                )),
            },
            candid_core::RelatedLocation {
                message: "second related".to_string(),
                span: Some(SourceSpan::source_only("memory:/lib.did")),
            },
        ])
        .with_notes(vec!["note".to_string()]);
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        json!({
            "code": "did_parse_error",
            "phase": "parse",
            "severity": "error",
            "message": "m",
            "span": { "source_name": "memory:/entry.did", "start_byte": 3, "end_byte": 4 },
            "related": [
                {
                    "message": "first related",
                    "span": { "source_name": "memory:/entry.did", "start_byte": 8, "end_byte": 9 },
                },
                {
                    "message": "second related",
                    "span": { "source_name": "memory:/lib.did" },
                },
            ],
            "notes": ["note"],
        })
    );
    let serialized = serde_json::to_string(&item).unwrap();
    let round_tripped: Diagnostic = serde_json::from_str(&serialized).unwrap();
    assert_eq!(round_tripped, item);
    assert_eq!(serde_json::to_string(&round_tripped).unwrap(), serialized);
}

#[test]
fn half_and_empty_spans_are_rejected_on_deserialization() {
    for invalid in [
        r#"{"code":"c","message":"m","span":{"start_byte":5}}"#,
        r#"{"code":"c","message":"m","span":{"end_byte":5}}"#,
        r#"{"code":"c","message":"m","span":{"source_name":"s","start_byte":5}}"#,
        r#"{"code":"c","message":"m","span":{}}"#,
    ] {
        assert!(
            serde_json::from_str::<Diagnostic>(invalid).is_err(),
            "must reject: {invalid}"
        );
    }
    // Both legal forms still decode.
    for valid in [
        r#"{"code":"c","message":"m","span":{"start_byte":5,"end_byte":6}}"#,
        r#"{"code":"c","message":"m","span":{"source_name":"s"}}"#,
    ] {
        assert!(
            serde_json::from_str::<Diagnostic>(valid).is_ok(),
            "must accept: {valid}"
        );
    }
}

#[test]
fn resolver_route_parse_errors_keep_exact_original_offsets() {
    let resolver = MemoryResolver::new()
        .with_source("memory:/entry.did", "import \"lib.did\";\nservice : {};\n")
        .unwrap()
        .with_source("memory:/lib.did", "type Broken = ;\n")
        .unwrap();
    let error = compile_with_resolver(
        "memory:/entry.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap_err();
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "did_parse_error");
    let span = diagnostic.span.as_ref().expect("parse errors carry a span");
    assert_eq!(span.source_name.as_deref(), Some("memory:/lib.did"));
    // "type Broken = ;" — the unexpected token is the ";" at byte 14.
    assert_eq!(span.start_byte, Some(14));
    assert_eq!(span.end_byte, Some(15));
}

#[test]
fn provenance_rederivation_preserves_resource_metadata() {
    // Chain: compile → embed source bundle → revalidate the sidecar under a
    // budget too small to rederive it. The rederivation failure must keep the
    // exact {resource, limit, observed} triple instead of flattening to text.
    let compilation =
        compile_did("type Item = opt opt opt opt opt opt nat; service : {};").unwrap();
    let raw: candid_core::RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();
    let error = candid_core::SourceInfo::try_from_raw(
        raw,
        compilation.contract(),
        &Limits {
            max_type_depth: 2,
            ..Limits::default()
        },
    )
    .unwrap_err();
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    assert_eq!(violation.path.as_deref(), Some("$"));
    assert!(violation
        .message
        .starts_with("the embedded source bundle could not rederive provenance:"));
    let info = violation
        .resource_limit
        .as_ref()
        .expect("the resource triple must survive rederivation");
    assert_eq!(info.resource, "type_depth");
    assert_eq!(info.limit, 2);
    assert!(info.observed > 2);
    // Violations never carry compile-only phase/severity keys.
    let rendered = serde_json::to_value(violation).unwrap();
    assert!(rendered.get("phase").is_none());
    assert!(rendered.get("severity").is_none());
}

fn invalid_contract_error(
    mutate: impl Fn(&mut serde_json::Value),
    limits: &Limits,
) -> candid_core::ContractValidationError {
    let fixture = include_str!("fixtures/conformance/empty_actor.contract.json");
    let mut json: serde_json::Value = serde_json::from_str(fixture).unwrap();
    mutate(&mut json);
    match Contract::from_json_with_limits(&json.to_string(), limits) {
        Err(ContractJsonError::InvalidContract(error)) => error,
        other => panic!("expected an invalid-contract error, got {other:?}"),
    }
}

#[test]
fn zero_diagnostics_limit_yields_one_bounded_sentinel() {
    // An error collection is never empty: at max_diagnostics = 0 the first
    // observed violation produces exactly one structured sentinel.
    let limits = Limits {
        max_diagnostics: 0,
        ..Limits::default()
    };
    let error = invalid_contract_error(
        |json| json["identities"]["contract"] = json!("not-a-valid-id"),
        &limits,
    );
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([{
            "code": "resource_limit_exceeded",
            "path": "$",
            "message": "resource diagnostics exceeded limit 0; observed at least 1",
            "resource_limit": { "resource": "diagnostics", "limit": 0, "observed": 1 },
        }])
    );

    // Later observations update the same sentinel in place; the collection
    // never grows past the single guaranteed item.
    let error = invalid_contract_error(
        |json| {
            json["format"] = json!("bogus-format");
            json["identities"]["contract"] = json!("not-a-valid-id");
        },
        &limits,
    );
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([{
            "code": "resource_limit_exceeded",
            "path": "$",
            "message": "resource diagnostics exceeded limit 0; observed at least 2",
            "resource_limit": { "resource": "diagnostics", "limit": 0, "observed": 2 },
        }])
    );
}

#[test]
fn positive_diagnostics_limits_truncate_exactly_as_before() {
    // At capacity, the last retained slot is replaced by the sentinel …
    let error = invalid_contract_error(
        |json| {
            json["format"] = json!("bogus-format");
            json["identities"]["contract"] = json!("not-a-valid-id");
        },
        &Limits {
            max_diagnostics: 1,
            ..Limits::default()
        },
    );
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([{
            "code": "resource_limit_exceeded",
            "path": "$",
            "message": "resource diagnostics exceeded limit 1; observed at least 2",
            "resource_limit": { "resource": "diagnostics", "limit": 1, "observed": 2 },
        }])
    );

    // … and a cap that fits every violation stores them all, unsynthesized.
    let error = invalid_contract_error(
        |json| {
            json["format"] = json!("bogus-format");
            json["identities"]["contract"] = json!("not-a-valid-id");
        },
        &Limits {
            max_diagnostics: 2,
            ..Limits::default()
        },
    );
    assert_eq!(
        serde_json::to_value(&error.violations).unwrap(),
        json!([
            {
                "code": "unsupported_contract_format",
                "path": "$.format",
                "message": "expected \"candid-core\", found \"bogus-format\"",
            },
            {
                "code": "invalid_contract_id_format",
                "path": "$.identities.contract",
                "message": "contract identity must use candid-core:contract:v1:sha256:<64 lowercase hex>",
            },
        ])
    );
}

#[test]
fn legacy_diagnostic_json_round_trips_byte_identically() {
    // Both legacy wire shapes deserialize into the unified item and
    // re-serialize without gaining or losing a byte.
    for legacy in [
        r#"{"code":"did_parse_error","phase":"parse","severity":"error","message":"m","span":{"source_name":"memory:/inline.did","start_byte":3,"end_byte":4},"notes":["n"]}"#,
        r#"{"code":"resource_limit_exceeded","path":"$","message":"m","resource_limit":{"resource":"input_bytes","limit":1,"observed":2}}"#,
        r#"{"code":"host_value_kind_mismatch","path":"$","message":"expected opt, found text"}"#,
    ] {
        let item: Diagnostic = serde_json::from_str(legacy).expect("legacy shapes deserialize");
        assert_eq!(serde_json::to_string(&item).unwrap(), legacy);
    }
}

#[test]
fn unknown_keys_are_still_rejected() {
    assert!(
        serde_json::from_str::<Diagnostic>(r#"{"code":"c","message":"m","bogus":true}"#).is_err()
    );
    assert!(
        serde_json::from_str::<SourceSpan>(r#"{"source_name":"s","start":1,"end":2}"#).is_err()
    );
    assert!(serde_json::from_str::<candid_core::RelatedLocation>(
        r#"{"message":"m","bogus":true}"#
    )
    .is_err());
}

#[test]
fn constructors_pin_the_domain_field_conventions() {
    let compiler = Diagnostic::compiler("c", DiagnosticPhase::Load, "m");
    assert_eq!(compiler.phase, Some(DiagnosticPhase::Load));
    assert_eq!(compiler.severity, Some(Severity::Error));
    assert_eq!(compiler.path, None);

    let violation = Diagnostic::violation("c", "$.x", "m");
    assert_eq!(violation.phase, None);
    assert_eq!(violation.severity, None);
    assert_eq!(violation.path.as_deref(), Some("$.x"));

    let resource = Diagnostic::resource_violation("input_bytes", 1, 2);
    assert_eq!(
        serde_json::to_value(&resource).unwrap(),
        json!({
            "code": "resource_limit_exceeded",
            "path": "$",
            "message": "resource input_bytes exceeded limit 1; observed 2",
            "resource_limit": { "resource": "input_bytes", "limit": 1, "observed": 2 },
        })
    );

    // Source-scoped locations never fabricate offsets; exact spans always
    // carry both.
    assert_eq!(
        serde_json::to_value(SourceSpan::source_only("memory:/a.did")).unwrap(),
        json!({ "source_name": "memory:/a.did" })
    );
    assert_eq!(
        serde_json::to_value(SourceSpan::exact(None, 1, 2)).unwrap(),
        json!({ "start_byte": 1, "end_byte": 2 })
    );
}
