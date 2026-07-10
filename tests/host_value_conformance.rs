use candid_contract_runtime::{
    compile_did, validate_host_value, Contract, HostFieldValue, HostValue, Limits,
};

fn declaration(contract: &Contract, name: &str) -> u32 {
    contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == name)
        .unwrap_or_else(|| panic!("missing declaration {name}"))
        .ty
}

fn field(name: &str, value: HostValue) -> HostFieldValue {
    HostFieldValue {
        id: candid_parser::candid::idl_hash(name),
        value,
    }
}

#[test]
fn every_constructible_primitive_has_a_lossless_tagged_value() {
    let compilation = compile_did(
        r#"
        type All = record {
          null_value: null; bool_value: bool; nat_value: nat; int_value: int;
          n8: nat8; n16: nat16; n32: nat32; n64: nat64;
          i8: int8; i16: int16; i32: int32; i64: int64;
          f32: float32; f64: float64; text_value: text;
          reserved_value: reserved; principal_value: principal;
        };
        service : {};
        "#,
    )
    .unwrap();
    let contract = compilation.contract();
    let selector = contract.bind_type(declaration(contract, "All")).unwrap();
    let value = HostValue::Record {
        fields: vec![
            field("null_value", HostValue::Null),
            field("bool_value", HostValue::Bool { value: true }),
            field(
                "nat_value",
                HostValue::Nat {
                    value: "999999999999999999999999999999999999999999".to_string(),
                },
            ),
            field(
                "int_value",
                HostValue::Int {
                    value: "-999999999999999999999999999999999999999999".to_string(),
                },
            ),
            field("n8", HostValue::Nat8 { value: u8::MAX }),
            field("n16", HostValue::Nat16 { value: u16::MAX }),
            field("n32", HostValue::Nat32 { value: u32::MAX }),
            field(
                "n64",
                HostValue::Nat64 {
                    value: u64::MAX.to_string(),
                },
            ),
            field("i8", HostValue::Int8 { value: i8::MIN }),
            field("i16", HostValue::Int16 { value: i16::MIN }),
            field("i32", HostValue::Int32 { value: i32::MIN }),
            field(
                "i64",
                HostValue::Int64 {
                    value: i64::MIN.to_string(),
                },
            ),
            field(
                "f32",
                HostValue::Float32 {
                    bits: "80000000".to_string(),
                },
            ),
            field(
                "f64",
                HostValue::Float64 {
                    bits: "7ff8000000000001".to_string(),
                },
            ),
            field(
                "text_value",
                HostValue::Text {
                    value: "hello 🌍".to_string(),
                },
            ),
            field("reserved_value", HostValue::Reserved),
            field(
                "principal_value",
                HostValue::Principal {
                    value: "aaaaa-aa".to_string(),
                },
            ),
        ],
    };

    validate_host_value(contract, &selector, &value, &Limits::default()).unwrap();
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        HostValue::from_json_with_limits(&json, &Limits::default()).unwrap(),
        value
    );
}

#[test]
fn composite_values_preserve_presence_tags_and_reference_values() {
    let compilation = compile_did(
        r#"
        type Callback = func () -> ();
        type Endpoint = service { ping: () -> () };
        type Choice = variant { ok: text; err: nat };
        type Composite = record {
          maybe: opt text;
          items: vec nat16;
          choice: Choice;
          endpoint: Endpoint;
          callback: Callback;
        };
        service : {};
        "#,
    )
    .unwrap();
    let contract = compilation.contract();
    let selector = contract
        .bind_type(declaration(contract, "Composite"))
        .unwrap();
    let value = HostValue::Record {
        fields: vec![
            field("maybe", HostValue::Opt { value: None }),
            field(
                "items",
                HostValue::Vec {
                    values: vec![
                        HostValue::Nat16 { value: 1 },
                        HostValue::Nat16 { value: 65_535 },
                    ],
                },
            ),
            field(
                "choice",
                HostValue::Variant {
                    id: candid_parser::candid::idl_hash("ok"),
                    value: Box::new(HostValue::Text {
                        value: "done".to_string(),
                    }),
                },
            ),
            field(
                "endpoint",
                HostValue::Service {
                    principal: "aaaaa-aa".to_string(),
                },
            ),
            field(
                "callback",
                HostValue::Func {
                    principal: "aaaaa-aa".to_string(),
                    method: "ping".to_string(),
                },
            ),
        ],
    };
    validate_host_value(contract, &selector, &value, &Limits::default()).unwrap();
}

#[test]
fn empty_and_noncanonical_numeric_representations_are_rejected() {
    let compilation = compile_did("type Never = empty; type N64 = nat64; service : {};").unwrap();
    let contract = compilation.contract();
    let empty = contract.bind_type(declaration(contract, "Never")).unwrap();
    assert!(validate_host_value(contract, &empty, &HostValue::Null, &Limits::default()).is_err());

    let n64 = contract.bind_type(declaration(contract, "N64")).unwrap();
    for value in ["01", "18446744073709551616"] {
        assert!(validate_host_value(
            contract,
            &n64,
            &HostValue::Nat64 {
                value: value.to_string()
            },
            &Limits::default()
        )
        .is_err());
    }

    let nat_compilation = compile_did("type Big = nat; service : {};").unwrap();
    let nat_contract = nat_compilation.contract();
    let big = nat_contract
        .bind_type(declaration(nat_contract, "Big"))
        .unwrap();
    let limits = Limits {
        max_value_bytes: 4,
        ..Limits::default()
    };
    let error = validate_host_value(
        nat_contract,
        &big,
        &HostValue::Nat {
            value: "12345".to_string(),
        },
        &limits,
    )
    .unwrap_err();
    assert_eq!(
        error.violations[0]
            .resource_limit
            .as_ref()
            .unwrap()
            .resource,
        "value_bytes"
    );
}
