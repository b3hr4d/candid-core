//! Authoritative parity and regression coverage for the crate's internal
//! Candid name hash.
//!
//! Issue #24 removed `candid_parser` from the base dependency graph, which
//! means Contract validation can no longer call `candid_parser::candid::
//! idl_hash` to check a `ServiceMethod.id`. The replacement is a normative
//! eight-line implementation in the base feature set. Everything that
//! *depends* on it — every `contract_id`, every `interface_id`, every
//! canonical byte — is unchanged only if the two functions agree on every
//! input, so this file pins that agreement directly and then pins the
//! validation behaviour built on top of it.
//!
//! `candid_parser` is a dev-dependency precisely so these assertions run in
//! every feature configuration, `--no-default-features` included, where the
//! library itself never links it.

use candid_core::{
    Actor, ContractDraft, Declaration, Field, MethodMode, PrimitiveType, ServiceMethod, TypeNode,
};
use candid_parser::candid::idl_hash;

/// A single-method service whose method ID is supplied by the caller, so a
/// deliberately wrong ID can be pushed through validation.
fn service_draft(name: &str, id: u32) -> ContractDraft {
    ContractDraft::new(
        vec![
            TypeNode::Func {
                args: Vec::new(),
                results: Vec::new(),
                mode: MethodMode::Update,
            },
            TypeNode::Service {
                methods: vec![ServiceMethod {
                    name: name.to_string(),
                    id,
                    function: 0,
                }],
            },
        ],
        Vec::new(),
        Some(Actor::Service { service: 1 }),
    )
}

#[test]
fn ascii_names_hash_exactly_like_the_reference_implementation() {
    // The empty name has no service-method spelling — validation rejects it
    // before the hash is consulted — so its parity is pinned by the unit test
    // in `src/name_hash.rs` instead.
    for name in [
        "a",
        "z",
        "ok",
        "err",
        "ping",
        "transfer",
        "icrc1_transfer",
        "hyphen-name",
        "_leading_underscore",
        "0",
        "1234567890",
        "0.did",
        "a\"b",
        "tab\tseparated",
    ] {
        let draft = service_draft(name, idl_hash(name));
        // The draft builds only if the crate's own hash agrees with the
        // reference for this name: `build` validates every method ID.
        assert!(
            draft.clone().build().is_ok(),
            "method {name:?} must validate with the reference hash"
        );
        assert_eq!(
            service_method_id(draft),
            idl_hash(name),
            "hash mismatch for {name:?}"
        );
    }
}

#[test]
fn unicode_names_hash_over_utf8_bytes_exactly_like_the_reference() {
    for name in [
        "méthode",
        "日本語",
        "Ünicöde",
        "\u{1f600}\u{1f680}",
        "\u{80}",
        "\u{7ff}",
        "\u{800}",
        "\u{ffff}",
        "\u{10000}",
        "\u{10ffff}",
        "a\u{0}b",
    ] {
        let draft = service_draft(name, idl_hash(name));
        assert!(
            draft.clone().build().is_ok(),
            "method {name:?} must validate with the reference hash"
        );
        assert_eq!(service_method_id(draft), idl_hash(name));
    }
}

#[test]
fn long_names_wrap_identically_past_the_u32_boundary() {
    // The accumulator wraps after roughly five bytes; sweep lengths well past
    // that, including sizes where a naive `u32` implementation would panic in
    // a debug build.
    for length in [1usize, 4, 5, 6, 7, 32, 255, 256, 1_000, 4_096] {
        for unit in ["z", "\u{e9}", "\u{1f600}"] {
            let name = unit.repeat(length);
            let draft = service_draft(&name, idl_hash(&name));
            assert!(
                draft.clone().build().is_ok(),
                "a {length}-unit {unit:?} name must validate"
            );
            assert_eq!(service_method_id(draft), idl_hash(&name));
        }
    }
}

#[test]
fn known_collision_pairs_still_collide_and_stay_distinct_methods() {
    // Distinct spellings that share one Candid ID. Validation must accept the
    // colliding IDs (they are correct) while still rejecting the duplicate
    // *name*, so the collision cannot be used to smuggle a bogus ID through.
    let collision = idl_hash("jhwlzguu");
    assert_eq!(collision, idl_hash("jsyrjsvk"));

    for name in ["jhwlzguu", "jsyrjsvk"] {
        assert!(service_draft(name, collision).build().is_ok());
    }

    let both = ContractDraft::new(
        vec![
            TypeNode::Func {
                args: Vec::new(),
                results: Vec::new(),
                mode: MethodMode::Update,
            },
            TypeNode::Service {
                methods: vec![
                    ServiceMethod {
                        name: "jhwlzguu".to_string(),
                        id: collision,
                        function: 0,
                    },
                    ServiceMethod {
                        name: "jsyrjsvk".to_string(),
                        id: collision,
                        function: 0,
                    },
                ],
            },
        ],
        Vec::new(),
        Some(Actor::Service { service: 1 }),
    );
    assert!(
        both.build().is_ok(),
        "two colliding method names are legal Candid and must stay valid"
    );
}

#[test]
fn service_method_validation_rejects_every_wrong_id() {
    for name in ["ping", "méthode", "jhwlzguu"] {
        let correct = idl_hash(name);
        for wrong in [
            correct.wrapping_add(1),
            correct.wrapping_sub(1),
            0,
            u32::MAX,
            idl_hash("some_other_name"),
        ] {
            if wrong == correct {
                continue;
            }
            let error = service_draft(name, wrong)
                .build()
                .expect_err("a wrong method ID must be rejected");
            let violation = &error.violations[0];
            assert_eq!(violation.code, "method_id_mismatch");
            assert_eq!(
                violation.path.as_deref(),
                Some("$.types[1].methods[0].id"),
                "the diagnostic path must stay unchanged"
            );
            assert_eq!(
                violation.message,
                format!("method ID {wrong} does not equal Candid hash {correct} for {name:?}")
            );
        }
    }
}

/// Field IDs are the same hash in the same wire position, so a record built
/// from reference-hashed labels must validate and keep its IDs verbatim.
#[test]
fn field_ids_from_the_reference_hash_survive_canonicalization() {
    let labels = ["left", "right", "méthode", "jhwlzguu", "jsyrjsvk"];
    let mut fields: Vec<Field> = labels
        .iter()
        .map(|label| Field {
            id: idl_hash(label),
            ty: 0,
        })
        .collect();
    fields.sort_by_key(|field| field.id);
    fields.dedup_by_key(|field| field.id);

    let contract = ContractDraft::new(
        vec![
            TypeNode::Primitive {
                primitive: PrimitiveType::Nat,
            },
            TypeNode::Record {
                fields: fields.clone(),
            },
        ],
        vec![Declaration {
            name: "Payload".to_string(),
            ty: 1,
        }],
        None,
    )
    .build()
    .expect("reference-hashed field IDs must validate");

    let observed: Vec<u32> = contract
        .types()
        .iter()
        .find_map(|node| match node {
            TypeNode::Record { fields } => Some(fields.iter().map(|field| field.id).collect()),
            _ => None,
        })
        .expect("the record survives canonicalization");
    assert_eq!(observed, fields.iter().map(|f| f.id).collect::<Vec<_>>());
}

/// Reads the single service method's ID back out of a built Contract, which is
/// the only externally visible way to observe what the crate hashed.
fn service_method_id(draft: ContractDraft) -> u32 {
    let contract = draft.build().expect("draft must validate");
    contract
        .types()
        .iter()
        .find_map(|node| match node {
            TypeNode::Service { methods } => methods.first().map(|method| method.id),
            _ => None,
        })
        .expect("the service survives canonicalization")
}
