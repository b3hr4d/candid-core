use candid_core::{Contract, Declaration, Field, Limits, PrimitiveType, RawContract, TypeNode};

fn deep_record(depth: u32) -> RawContract {
    let mut types = (0..depth)
        .map(|index| TypeNode::Record {
            fields: vec![Field {
                id: index,
                ty: index + 1,
            }],
        })
        .collect::<Vec<_>>();
    types.push(TypeNode::Primitive {
        primitive: PrimitiveType::Nat,
    });
    RawContract::new(
        types,
        vec![Declaration {
            name: "AdversarialDepth".to_string(),
            ty: 0,
        }],
        None,
    )
}

#[test]
fn canonicalization_adversarial_work_threshold_is_enforced() {
    let input = deep_record(1_024);
    let accepted = Limits {
        max_canonicalization_work: 20_000,
        ..Limits::default()
    };
    assert!(Contract::build_raw(input.clone(), &accepted).is_ok());

    let rejected = Limits {
        max_canonicalization_work: 1_000,
        ..Limits::default()
    };
    let error = Contract::build_raw(input, &rejected).unwrap_err();
    let limit = error
        .violations
        .iter()
        .find_map(|violation| violation.resource_limit.as_ref())
        .expect("canonicalization work must be charged");
    assert_eq!(limit.resource, "canonicalization_work");
    assert_eq!(limit.limit, rejected.max_canonicalization_work);
}
