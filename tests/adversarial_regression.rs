use candid_core::{ContractDraft, Declaration, Field, Limits, PrimitiveType, TypeNode};

fn deep_record(depth: u32) -> ContractDraft {
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
    ContractDraft::new(
        types,
        vec![Declaration {
            name: "AdversarialDepth".to_string(),
            ty: 0,
        }],
        None,
    )
}

fn wide_record(width: u32) -> ContractDraft {
    let mut types = vec![TypeNode::Primitive {
        primitive: PrimitiveType::Nat,
    }];
    types.push(TypeNode::Record {
        fields: (0..width).map(|id| Field { id, ty: 0 }).collect(),
    });
    ContractDraft::new(
        types,
        vec![Declaration {
            name: "AdversarialWidth".to_string(),
            ty: 1,
        }],
        None,
    )
}

fn assert_work_limit(input: ContractDraft, accepted_limit: usize, rejected_limit: usize) {
    let accepted = Limits::default().with_max_canonicalization_work(accepted_limit);
    assert!(
        input.clone().build_with_limits(&accepted).is_ok(),
        "graph must remain within its canonicalization regression threshold"
    );

    let rejected = Limits::default().with_max_canonicalization_work(rejected_limit);
    let error = input.build_with_limits(&rejected).unwrap_err();
    let limit = error
        .violations
        .iter()
        .find_map(|violation| violation.resource_limit.as_ref())
        .expect("canonicalization work must be charged");
    assert_eq!(limit.resource, "canonicalization_work");
    assert_eq!(limit.limit, rejected.max_canonicalization_work() as u64);
    assert!(limit.observed > limit.limit);
}

#[test]
fn canonicalization_adversarial_work_threshold_is_enforced() {
    assert_work_limit(deep_record(1_024), 1_000_000, 100_000);
}

#[test]
fn canonicalization_wide_graph_threshold_is_enforced() {
    assert_work_limit(wide_record(1_024), 1_000_000, 100_000);
}
