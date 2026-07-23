//! Issue #23 conformance: clean draft construction, versioned Limits
//! profiles, and portable fixed-width numeric wire values.
//!
//! Pins the exact serialized shapes of `ContractDraft`, the portable
//! `Limits`/`RuntimeContext` configuration, `ResourceLimitInfo`, and
//! `SourceSpan`; every `InteractiveV1` default number; the structured
//! configuration error codes/paths/messages; and the defined behavior of
//! zero limits and elapsed deadlines.

use candid_core::{
    compile_did, Contract, ContractDraft, Declaration, Diagnostic, Field, Limits, LimitsConfig,
    LimitsProfile, PrimitiveType, ProducerInfo, RawContract, RuntimeContext, SourceSpan, TypeNode,
    LIMITS_CONFIG_VERSION,
};
use serde_json::json;

fn nat_draft() -> ContractDraft {
    ContractDraft::new(
        vec![TypeNode::Primitive {
            primitive: PrimitiveType::Nat,
        }],
        vec![Declaration {
            name: "Amount".to_string(),
            ty: 0,
        }],
        None,
    )
}

/// A graph with a duplicate field ID and a dangling reference: guaranteed to
/// produce more than one violation, for diagnostics-cap tests.
fn invalid_draft() -> ContractDraft {
    ContractDraft::new(
        vec![TypeNode::Record {
            fields: (0..8).map(|_| Field { id: 0, ty: 9 }).collect(),
        }],
        vec![Declaration {
            name: "Broken".to_string(),
            ty: 0,
        }],
        None,
    )
}

// --- A. ContractDraft: producer drafts carry no fake identities ------------

#[test]
fn serialized_draft_contains_only_draft_fields_and_never_identities() {
    let compiled = compile_did("service : { ping: () -> () };").unwrap();
    let contract = compiled.contract();
    let draft = ContractDraft::new(
        contract.types().to_vec(),
        contract.declarations().to_vec(),
        contract.actor().cloned(),
    )
    .with_producer(contract.producer().clone());

    let value = serde_json::to_value(&draft).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["actor", "declarations", "producer", "types"]);
    for forbidden in [
        "format",
        "format_version",
        "semantics_profile",
        "canonicalization_profile",
        "identities",
    ] {
        assert!(
            value.get(forbidden).is_none(),
            "a draft must never serialize {forbidden}"
        );
    }

    // Serialized field order, and omission of absent actor/producer, pinned
    // on the emitted text itself.
    let empty = ContractDraft::new(vec![], vec![], None);
    assert_eq!(
        serde_json::to_string(&empty).unwrap(),
        r#"{"types":[],"declarations":[]}"#,
        "actor and producer are omitted when absent"
    );
}

#[test]
fn draft_decode_rejects_identity_and_format_keys_as_unknown_fields() {
    for (key, value) in [
        ("format", json!("candid-core")),
        ("format_version", json!(1)),
        ("semantics_profile", json!("candid-1")),
        ("canonicalization_profile", json!("candid-core-canon-1")),
        ("identities", json!({"contract": "x"})),
    ] {
        let document = json!({"types": [], key: value});
        let error = serde_json::from_value::<ContractDraft>(document).unwrap_err();
        assert!(
            error.to_string().contains("unknown field"),
            "{key}: {error}"
        );
    }
}

#[test]
fn draft_declarations_default_to_empty_and_round_trip() {
    let minimal: ContractDraft = serde_json::from_value(json!({"types": []})).unwrap();
    assert_eq!(minimal, ContractDraft::new(vec![], vec![], None));

    let draft = nat_draft();
    let round_tripped: ContractDraft =
        serde_json::from_str(&serde_json::to_string(&draft).unwrap()).unwrap();
    assert_eq!(round_tripped, draft);
}

#[test]
fn draft_rejects_explicit_actor_null_like_raw_contract() {
    let error = serde_json::from_value::<ContractDraft>(json!({"types": [], "actor": null}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("invalid type: null"),
        "explicit actor: null must be a decode error, not a second spelling of \
         \"no actor\": {error}"
    );
    let error = serde_json::from_value::<ContractDraft>(json!({"types": [], "producer": null}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("invalid type: null"),
        "explicit producer: null must be a decode error, not a second spelling \
         of \"default producer\": {error}"
    );
    // The equivalent RawContract rejection is unchanged.
    assert!(serde_json::from_value::<RawContract>(json!({
        "format": "candid-core",
        "format_version": 1,
        "semantics_profile": "candid-1",
        "canonicalization_profile": "candid-core-canon-1",
        "identities": {"contract": "candid-core:contract:v1:sha256:x"},
        "producer": {
            "name": "n", "version": "v",
            "candid_version": "c", "candid_parser_version": "p"
        },
        "types": [],
        "actor": null
    }))
    .is_err());
}

#[test]
fn draft_build_defaults_producer_to_current_and_honors_overrides() {
    let default_built = nat_draft().build().unwrap();
    assert_eq!(default_built.producer(), &ProducerInfo::current());

    let custom = ProducerInfo {
        name: "external-tool".to_string(),
        version: "9.9.9".to_string(),
        candid_version: "0.0.1".to_string(),
        candid_parser_version: "0.0.2".to_string(),
    };
    let overridden = nat_draft().with_producer(custom.clone()).build().unwrap();
    assert_eq!(overridden.producer(), &custom);
    // Producer stays outside authenticated identity.
    assert_eq!(default_built.contract_id(), overridden.contract_id());

    // A producer supplied on the wire decodes and is honored at build time.
    let decoded: ContractDraft = serde_json::from_value(json!({
        "types": [{"kind": "primitive", "primitive": "nat"}],
        "declarations": [{"name": "Amount", "type": 0}],
        "producer": {
            "name": "external-tool", "version": "9.9.9",
            "candid_version": "0.0.1", "candid_parser_version": "0.0.2"
        }
    }))
    .unwrap();
    assert_eq!(decoded.build().unwrap().producer(), &custom);
}

#[test]
fn draft_build_matches_the_compiler_and_the_verified_raw_path_exactly() {
    let compiled = compile_did(
        r#"
        type Payload = record { amount: nat; note: text };
        service : { submit: (Payload) -> (); read: () -> (Payload) query };
        "#,
    )
    .unwrap();
    let contract = compiled.contract();

    let rebuilt = ContractDraft::new(
        contract.types().to_vec(),
        contract.declarations().to_vec(),
        contract.actor().cloned(),
    )
    .build()
    .unwrap();
    assert_eq!(&rebuilt, contract, "identical graph, identities, and bytes");

    let verified = Contract::try_from_raw(RawContract::from(contract)).unwrap();
    assert_eq!(&verified, contract);
    assert_eq!(rebuilt.contract_id(), verified.contract_id());
    assert_eq!(rebuilt.interface_id(), verified.interface_id());
}

#[test]
fn draft_build_runs_under_the_same_budgets_as_every_other_entry_point() {
    let starved = Limits::default().with_max_canonicalization_work(1);
    let error = nat_draft().build_with_limits(&starved).unwrap_err();
    let info = error.violations[0].resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "canonicalization_work");
    assert_eq!(info.limit, 1);

    let error = nat_draft()
        .build_with_context(&RuntimeContext::new(
            Limits::default().with_deadline_unix_ms(Some(1)),
        ))
        .unwrap_err();
    assert_eq!(error.violations[0].code, "operation_deadline_exceeded");
}

// --- B. Versioned Limits profile and portable config -----------------------

#[test]
fn interactive_v1_default_numbers_are_pinned_exactly() {
    let limits = Limits::default();
    assert_eq!(limits.profile(), LimitsProfile::InteractiveV1);
    assert_eq!(LimitsProfile::InteractiveV1.limits(), limits);
    assert_eq!(LimitsProfile::InteractiveV1.wire_name(), "interactive_v1");
    assert_eq!(LIMITS_CONFIG_VERSION, 1);

    assert_eq!(limits.max_input_bytes(), 4_194_304);
    assert_eq!(limits.max_source_bytes(), 1_048_576);
    assert_eq!(limits.max_bundle_bytes(), 8_388_608);
    assert_eq!(limits.max_sources(), 256);
    assert_eq!(limits.max_source_id_bytes(), 1_024);
    assert_eq!(limits.max_import_depth(), 64);
    assert_eq!(limits.max_import_edges(), 1_024);
    assert_eq!(limits.max_source_nesting(), 256);
    assert_eq!(limits.max_type_depth(), 256);
    assert_eq!(limits.max_value_nesting(), 64);
    assert_eq!(limits.max_type_nodes(), 100_000);
    assert_eq!(limits.max_graph_edges(), 1_000_000);
    assert_eq!(limits.max_declarations(), 100_000);
    assert_eq!(limits.max_fields(), 500_000);
    assert_eq!(limits.max_methods(), 100_000);
    assert_eq!(limits.max_function_values(), 500_000);
    assert_eq!(limits.max_string_bytes(), 1_048_576);
    assert_eq!(limits.max_producer_bytes(), 4_096);
    assert_eq!(limits.max_diagnostics(), 100);
    assert_eq!(limits.max_canonicalization_work(), 10_000_000);
    assert_eq!(limits.max_provenance_work(), 10_000_000);
    assert_eq!(limits.max_source_identity_work(), 400_000_000);
    assert_eq!(limits.max_value_depth(), 256);
    assert_eq!(limits.max_value_elements(), 1_000_000);
    assert_eq!(limits.max_value_bytes(), 16_777_216);
    assert_eq!(limits.deadline_unix_ms(), None);
}

#[test]
fn default_limits_serialize_to_the_exact_pinned_config_document() {
    assert_eq!(
        serde_json::to_string(&Limits::default()).unwrap(),
        r#"{"version":1,"profile":"interactive_v1","overrides":{}}"#
    );
    let decoded: Limits =
        serde_json::from_str(r#"{"version":1,"profile":"interactive_v1","overrides":{}}"#).unwrap();
    assert_eq!(decoded, Limits::default());
    // A missing overrides object means no overrides.
    let decoded: Limits =
        serde_json::from_str(r#"{"version":1,"profile":"interactive_v1"}"#).unwrap();
    assert_eq!(decoded, Limits::default());
}

#[test]
fn only_explicit_overrides_serialize_and_they_round_trip() {
    let limits = Limits::default()
        .with_max_input_bytes(512)
        .with_max_diagnostics(0)
        .with_deadline_unix_ms(Some(2_000_000_000_000));
    let value = serde_json::to_value(&limits).unwrap();
    assert_eq!(
        value,
        json!({
            "version": 1,
            "profile": "interactive_v1",
            "overrides": {
                "max_input_bytes": 512,
                "max_diagnostics": 0,
                "deadline_unix_ms": 2_000_000_000_000u64
            }
        })
    );
    let round_tripped: Limits = serde_json::from_value(value).unwrap();
    assert_eq!(round_tripped, limits);

    // An override explicitly set back to the frozen profile value is the same
    // policy as omission, and normalizes away deterministically.
    let explicit_default =
        Limits::default().with_max_input_bytes(Limits::default().max_input_bytes());
    assert_eq!(
        serde_json::to_string(&explicit_default).unwrap(),
        r#"{"version":1,"profile":"interactive_v1","overrides":{}}"#
    );
}

#[test]
fn unsupported_versions_and_profiles_fail_with_pinned_structured_errors() {
    let config: LimitsConfig = serde_json::from_value(json!({
        "version": 2, "profile": "interactive_v1", "overrides": {}
    }))
    .unwrap();
    let error = Limits::try_from(config).unwrap_err();
    assert_eq!(error.code(), "unsupported_limits_version");
    assert_eq!(error.path(), "$.version");
    assert_eq!(
        error.message(),
        "unsupported limits config version 2; this build supports version 1"
    );
    assert_eq!(
        error.to_string(),
        "unsupported_limits_version at $.version: unsupported limits config version 2; \
         this build supports version 1"
    );

    let config: LimitsConfig = serde_json::from_value(json!({
        "version": 1, "profile": "server_v1", "overrides": {}
    }))
    .unwrap();
    let error = Limits::try_from(config).unwrap_err();
    assert_eq!(error.code(), "unsupported_limits_profile");
    assert_eq!(error.path(), "$.profile");
    assert_eq!(
        error.message(),
        "unknown limits profile \"server_v1\"; known profiles: \"interactive_v1\""
    );

    // The serde path wraps the same structured rendering.
    let serde_error = serde_json::from_value::<Limits>(json!({
        "version": 2, "profile": "interactive_v1", "overrides": {}
    }))
    .unwrap_err()
    .to_string();
    assert!(
        serde_error.contains("unsupported_limits_version at $.version"),
        "{serde_error}"
    );
}

#[test]
fn unknown_config_and_override_fields_are_rejected() {
    for document in [
        json!({"version": 1, "profile": "interactive_v1", "overrides": {}, "extra": 1}),
        json!({"version": 1, "profile": "interactive_v1",
               "overrides": {"max_speed": 88}}),
        json!({"version": 1, "profile": "interactive_v1",
               "overrides": {"limits": {}}}),
    ] {
        let error = serde_json::from_value::<Limits>(document.clone())
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field"), "{document}: {error}");
    }
    // An explicit null override is rejected, not read as "no override".
    let error = serde_json::from_value::<Limits>(json!({
        "version": 1, "profile": "interactive_v1",
        "overrides": {"max_input_bytes": null}
    }))
    .unwrap_err()
    .to_string();
    assert!(error.contains("invalid type: null"), "{error}");
    // Missing version or profile is likewise rejected.
    for document in [
        json!({"profile": "interactive_v1", "overrides": {}}),
        json!({"version": 1, "overrides": {}}),
    ] {
        assert!(serde_json::from_value::<Limits>(document).is_err());
    }
}

#[cfg(target_pointer_width = "32")]
#[test]
fn overrides_beyond_usize_are_rejected_with_the_pinned_error_on_32_bit() {
    // On a 32-bit host the public deserialization path itself must reject a
    // 64-bit override; 64-bit hosts prove the same seam through the simulated
    // boundary unit tests in `src/limits.rs`.
    let error = serde_json::from_value::<Limits>(json!({
        "version": 1, "profile": "interactive_v1",
        "overrides": {"max_input_bytes": 5_000_000_000u64}
    }))
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("limit_override_unrepresentable at $.overrides.max_input_bytes"),
        "{error}"
    );
}

#[test]
fn runtime_context_serializes_limits_as_the_portable_config_only() {
    let context = RuntimeContext::default();
    assert_eq!(
        serde_json::to_value(&context).unwrap(),
        json!({
            "limits": {"version": 1, "profile": "interactive_v1", "overrides": {}}
        })
    );
    let decoded: RuntimeContext = serde_json::from_value(json!({
        "limits": {
            "version": 1, "profile": "interactive_v1",
            "overrides": {"deadline_unix_ms": 7u64}
        }
    }))
    .unwrap();
    assert_eq!(decoded.limits.deadline_unix_ms(), Some(7));
    assert!(serde_json::from_value::<RuntimeContext>(json!({
        "limits": {"version": 1, "profile": "interactive_v1", "overrides": {}},
        "cancellation": true
    }))
    .is_err());
}

#[test]
fn all_zero_limits_deserialize_round_trip_and_fail_closed() {
    let all_zero = Limits::default()
        .with_max_input_bytes(0)
        .with_max_source_bytes(0)
        .with_max_bundle_bytes(0)
        .with_max_sources(0)
        .with_max_source_id_bytes(0)
        .with_max_import_depth(0)
        .with_max_import_edges(0)
        .with_max_source_nesting(0)
        .with_max_type_depth(0)
        .with_max_value_nesting(0)
        .with_max_type_nodes(0)
        .with_max_graph_edges(0)
        .with_max_declarations(0)
        .with_max_fields(0)
        .with_max_methods(0)
        .with_max_function_values(0)
        .with_max_string_bytes(0)
        .with_max_producer_bytes(0)
        .with_max_diagnostics(0)
        .with_max_canonicalization_work(0)
        .with_max_provenance_work(0)
        .with_max_source_identity_work(0)
        .with_max_value_depth(0)
        .with_max_value_elements(0)
        .with_max_value_bytes(0);

    // Every zero is a legal, defined configuration — never a decode error.
    let round_tripped: Limits =
        serde_json::from_str(&serde_json::to_string(&all_zero).unwrap()).unwrap();
    assert_eq!(round_tripped, all_zero);

    // And it fails closed: any input at all exceeds the zero input gate.
    let error = Contract::from_json_with_limits("{}", &all_zero).unwrap_err();
    let candid_core::ContractJsonError::InvalidContract(error) = error else {
        panic!("the byte gate must fire before JSON decoding: {error:?}");
    };
    let info = error.violations[0].resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "input_bytes");
    assert_eq!(info.limit, 0);
    assert_eq!(info.observed, 2);
}

#[test]
fn zero_diagnostics_from_the_portable_config_keeps_the_single_sentinel() {
    let limits: Limits = serde_json::from_value(json!({
        "version": 1, "profile": "interactive_v1",
        "overrides": {"max_diagnostics": 0}
    }))
    .unwrap();
    assert_eq!(limits.max_diagnostics(), 0);

    let error = invalid_draft().build_with_limits(&limits).unwrap_err();
    assert_eq!(
        error.violations.len(),
        1,
        "a zero cap retains exactly the sentinel, never an empty collection"
    );
    let sentinel = &error.violations[0];
    assert_eq!(sentinel.code, "resource_limit_exceeded");
    let info = sentinel.resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "diagnostics");
    assert_eq!(info.limit, 0);
    assert!(info.observed > 0);
}

#[test]
fn an_elapsed_deadline_from_the_portable_config_fails_closed() {
    let limits: Limits = serde_json::from_value(json!({
        "version": 1, "profile": "interactive_v1",
        "overrides": {"deadline_unix_ms": 1u64}
    }))
    .unwrap();
    assert!(limits.deadline_exceeded());
    let error = nat_draft().build_with_limits(&limits).unwrap_err();
    assert_eq!(error.violations[0].code, "operation_deadline_exceeded");
}

// --- C. Fixed-width portable diagnostic and source values -------------------

#[test]
fn resource_limit_triples_keep_their_exact_wire_shape() {
    let violation = Diagnostic::resource_violation("value_depth", 3, 4);
    assert_eq!(
        serde_json::to_value(&violation).unwrap(),
        json!({
            "code": "resource_limit_exceeded",
            "path": "$",
            "message": "resource value_depth exceeded limit 3; observed 4",
            "resource_limit": {"resource": "value_depth", "limit": 3, "observed": 4}
        })
    );
}

#[test]
fn resource_limit_values_are_64_bit_on_every_platform() {
    let beyond_u32 = Diagnostic::resource_violation("input_bytes", 5_000_000_000, 6_000_000_000);
    let value = serde_json::to_value(&beyond_u32).unwrap();
    assert_eq!(value["resource_limit"]["limit"], json!(5_000_000_000u64));
    assert_eq!(value["resource_limit"]["observed"], json!(6_000_000_000u64));
    let round_tripped: Diagnostic = serde_json::from_value(value).unwrap();
    assert_eq!(round_tripped, beyond_u32);
    assert!(serde_json::from_value::<Diagnostic>(json!({
        "code": "resource_limit_exceeded",
        "path": "$",
        "message": "m",
        "resource_limit": {"resource": "x", "limit": 1, "observed": 2, "extra": 3}
    }))
    .is_err());
}

#[test]
fn source_spans_are_64_bit_and_keep_their_omission_and_rejection_rules() {
    let wide = SourceSpan::exact(
        Some("memory:/big.did".to_string()),
        4_294_967_296,
        4_294_967_297,
    );
    let value = serde_json::to_value(&wide).unwrap();
    assert_eq!(
        value,
        json!({
            "source_name": "memory:/big.did",
            "start_byte": 4_294_967_296u64,
            "end_byte": 4_294_967_297u64
        })
    );
    let round_tripped: SourceSpan = serde_json::from_value(value).unwrap();
    assert_eq!(round_tripped, wide);

    // Ordinary offsets keep their exact pre-existing JSON text.
    assert_eq!(
        serde_json::to_string(&SourceSpan::exact(None, 3, 4)).unwrap(),
        r#"{"start_byte":3,"end_byte":4}"#
    );
    assert_eq!(
        serde_json::to_string(&SourceSpan::source_only("memory:/lib.did")).unwrap(),
        r#"{"source_name":"memory:/lib.did"}"#
    );

    // Half-present and empty spans stay rejected.
    for rejected in [
        json!({"start_byte": 5}),
        json!({"end_byte": 5}),
        json!({"source_name": "s", "start_byte": 5}),
        json!({"source_name": "s", "end_byte": 5}),
        json!({}),
    ] {
        assert!(
            serde_json::from_value::<SourceSpan>(rejected.clone()).is_err(),
            "{rejected} must not decode"
        );
    }
}
