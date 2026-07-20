use candid_core::{compile_did, validate_host_value, Contract, HostValue, Limits};
use serde_json::{json, Value};

fn declaration(contract: &Contract, name: &str) -> u32 {
    contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == name)
        .unwrap_or_else(|| panic!("missing declaration {name}"))
        .ty
}

fn parse(value: Value) -> HostValue {
    HostValue::from_json_with_limits(&serde_json::to_string(&value).unwrap(), &Limits::default())
        .unwrap()
}

fn field(name: &str, value: Value) -> Value {
    json!({ "id": candid_parser::candid::idl_hash(name), "value": value })
}

#[test]
fn json_boundary_accepts_all_canonical_scalar_forms() {
    let values = [
        json!({ "kind": "null" }),
        json!({ "kind": "bool", "value": true }),
        json!({ "kind": "nat", "value": "0" }),
        json!({ "kind": "int", "value": "-1" }),
        json!({ "kind": "nat8", "value": 255 }),
        json!({ "kind": "nat16", "value": 65535 }),
        json!({ "kind": "nat32", "value": 4294967295u64 }),
        json!({ "kind": "nat64", "value": "18446744073709551615" }),
        json!({ "kind": "int8", "value": -128 }),
        json!({ "kind": "int16", "value": -32768 }),
        json!({ "kind": "int32", "value": -2147483648i64 }),
        json!({ "kind": "int64", "value": "-9223372036854775808" }),
        json!({ "kind": "float32", "bits": "deadbeef" }),
        json!({ "kind": "float64", "bits": "7ff8000000000001" }),
        json!({ "kind": "text", "value": "hello" }),
        json!({ "kind": "reserved" }),
        json!({ "kind": "principal", "value": "aaaaa-aa" }),
        json!({ "kind": "service", "principal": "aaaaa-aa" }),
        json!({ "kind": "func", "principal": "aaaaa-aa", "method": "go" }),
    ];

    for value in values {
        parse(value);
    }
}

#[test]
fn json_boundary_rejects_noncanonical_scalars_and_unknown_fields() {
    let values = [
        json!({ "kind": "nat", "value": "01" }),
        json!({ "kind": "int", "value": "-0" }),
        json!({ "kind": "nat64", "value": "18446744073709551616" }),
        json!({ "kind": "int64", "value": "9223372036854775808" }),
        json!({ "kind": "float32", "bits": "DEADBEEF" }),
        json!({ "kind": "float64", "bits": "7ff800000000001" }),
        json!({ "kind": "principal", "value": "AAAAA-AA" }),
        json!({ "kind": "principal", "value": "aaaaaaa" }),
        json!({ "kind": "service", "principal": "aaaaa-aa", "extra": true }),
        json!({ "kind": "record", "fields": [{ "id": 1, "value": { "kind": "null" }, "extra": true }] }),
    ];

    for value in values {
        assert!(HostValue::from_json_with_limits(
            &serde_json::to_string(&value).unwrap(),
            &Limits::default()
        )
        .is_err());
    }
}

#[test]
fn json_boundary_validates_nested_values_and_resource_limits() {
    let invalid = [
        json!({ "kind": "opt", "value": { "kind": "nat", "value": "00" } }),
        json!({ "kind": "vec", "values": [{ "kind": "float32", "bits": "DEADBEEF" }] }),
        json!({ "kind": "record", "fields": [field("x", json!({ "kind": "principal", "value": "AAAAA-AA" }))] }),
        json!({ "kind": "variant", "id": 3, "value": { "kind": "int64", "value": "9223372036854775808" } }),
    ];
    for value in invalid {
        let error = HostValue::from_json_with_limits(
            &serde_json::to_string(&value).unwrap(),
            &Limits::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains('$'));
    }

    let value = json!({ "kind": "vec", "values": [{ "kind": "nat", "value": "1" }, { "kind": "nat", "value": "2" }] });
    let limits = Limits {
        max_value_elements: 2,
        ..Limits::default()
    };
    assert!(
        HostValue::from_json_with_limits(&serde_json::to_string(&value).unwrap(), &limits).is_err()
    );
}

#[test]
fn public_constructors_produce_only_canonical_scalars() {
    assert!(HostValue::nat("01").is_err());
    assert!(HostValue::int("-0").is_err());
    assert!(HostValue::nat64("18446744073709551616").is_err());
    assert!(HostValue::int64("9223372036854775808").is_err());
    assert!(HostValue::float32("DEADBEEF").is_err());
    assert!(HostValue::float64("7ff800000000001").is_err());
    assert!(HostValue::principal("AAAAA-AA").is_err());
    assert!(HostValue::service("AAAAA-AA").is_err());
    assert!(HostValue::func("AAAAA-AA", "go").is_err());

    HostValue::record(
        vec![candid_core::HostFieldValue::new(
            1,
            HostValue::vector(vec![HostValue::nat("1").unwrap()], &Limits::default()).unwrap(),
        )],
        &Limits::default(),
    )
    .unwrap();
}

#[test]
fn local_canonicalization_is_separate_from_contract_validation() {
    let value = parse(json!({ "kind": "nat", "value": "1" }));
    let compilation = compile_did("type Small = nat8; service : {};").unwrap();
    let contract = compilation.contract();
    let selector = contract.bind_type(declaration(contract, "Small")).unwrap();

    let error = validate_host_value(contract, &selector, &value, &Limits::default()).unwrap_err();
    assert_eq!(error.violations[0].code, "host_value_kind_mismatch");
}

#[test]
fn validation_preserves_wide_container_resource_diagnostics() {
    let compilation = compile_did("type Items = vec nat; service : {};").unwrap();
    let contract = compilation.contract();
    let selector = contract.bind_type(declaration(contract, "Items")).unwrap();
    let value = parse(json!({
        "kind": "vec",
        "values": (0..100).map(|_| json!({ "kind": "nat", "value": "1" })).collect::<Vec<_>>(),
    }));
    let limits = Limits {
        max_value_elements: 10,
        ..Limits::default()
    };

    let error = validate_host_value(contract, &selector, &value, &limits).unwrap_err();
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded");
    assert_eq!(violation.path, "$");
    let info = violation.resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "value_elements");
    assert_eq!(info.limit, 10);
    assert_eq!(info.observed, 101);
}

#[test]
fn validation_bounds_record_scans_by_canonicalization_work() {
    let compilation =
        compile_did("type T = record { a: nat; b: nat; c: nat; d: nat }; service : {};").unwrap();
    let contract = compilation.contract();
    let selector = contract.bind_type(declaration(contract, "T")).unwrap();
    let value = parse(json!({
        "kind": "record",
        "fields": [
            field("a", json!({ "kind": "nat", "value": "1" })),
            field("b", json!({ "kind": "nat", "value": "1" })),
            field("c", json!({ "kind": "nat", "value": "1" })),
            field("d", json!({ "kind": "nat", "value": "1" })),
        ],
    }));
    let limits = Limits {
        max_canonicalization_work: 2,
        ..Limits::default()
    };

    let error = validate_host_value(contract, &selector, &value, &limits).unwrap_err();
    let info = error.violations[0].resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "canonicalization_work");
    assert_eq!(info.limit, 2);
    assert_eq!(info.observed, 3);
}

#[test]
fn validation_bounds_variant_tag_scans_by_canonicalization_work() {
    // The variant arm resolves the tag by scanning the type's field table,
    // exactly like the record arm resolves field IDs. A variant type may carry
    // up to `max_fields` tags and a `vec variant` value can force one scan per
    // element, so the scan must be charged too. A value selecting a tag that is
    // not in the table forces a full scan; a tight budget must interrupt it and
    // report `canonicalization_work`, not run to completion and report
    // `unknown_variant_id`.
    let compilation =
        compile_did("type V = variant { a: null; b: null; c: null; d: null }; service : {};")
            .unwrap();
    let contract = compilation.contract();
    let selector = contract.bind_type(declaration(contract, "V")).unwrap();
    let value = parse(json!({
        "kind": "variant",
        "id": u32::MAX,
        "value": { "kind": "null" },
    }));
    let limits = Limits {
        max_canonicalization_work: 2,
        ..Limits::default()
    };

    let error = validate_host_value(contract, &selector, &value, &limits).unwrap_err();
    let info = error.violations[0]
        .resource_limit
        .as_ref()
        .unwrap_or_else(|| panic!("variant scan must charge and fail closed: {error:#?}"));
    assert_eq!(info.resource, "canonicalization_work");
    assert_eq!(info.limit, 2);
    assert_eq!(info.observed, 3);
}

#[test]
fn variant_tag_scans_are_deterministic_across_runs() {
    // The same hostile input must fail the same way every time.
    let compilation =
        compile_did("type V = variant { a: null; b: null; c: null; d: null }; service : {};")
            .unwrap();
    let contract = compilation.contract();
    let selector = contract.bind_type(declaration(contract, "V")).unwrap();
    let value = parse(json!({
        "kind": "variant",
        "id": u32::MAX,
        "value": { "kind": "null" },
    }));
    let limits = Limits {
        max_canonicalization_work: 2,
        ..Limits::default()
    };

    let first = validate_host_value(contract, &selector, &value, &limits).unwrap_err();
    let second = validate_host_value(contract, &selector, &value, &limits).unwrap_err();
    assert_eq!(first, second);
}
