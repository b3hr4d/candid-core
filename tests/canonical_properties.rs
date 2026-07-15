use candid_core::{
    Contract, Declaration, Field, Limits, PrimitiveType, RawContract, SourceId, TypeNode,
};
use proptest::prelude::*;

fn primitive(text: bool) -> TypeNode {
    TypeNode::Primitive {
        primitive: if text {
            PrimitiveType::Text
        } else {
            PrimitiveType::Nat
        },
    }
}

fn permuted_contract(left_is_text: bool, right_is_text: bool, permuted: bool) -> Contract {
    let types = if permuted {
        vec![
            primitive(right_is_text),
            primitive(left_is_text),
            TypeNode::Record {
                fields: vec![Field { id: 10, ty: 1 }, Field { id: 20, ty: 0 }],
            },
        ]
    } else {
        vec![
            TypeNode::Record {
                fields: vec![Field { id: 10, ty: 1 }, Field { id: 20, ty: 2 }],
            },
            primitive(left_is_text),
            primitive(right_is_text),
        ]
    };
    let root = if permuted { 2 } else { 0 };
    Contract::build_raw(
        RawContract::new(
            types,
            vec![Declaration {
                name: "Payload".to_string(),
                ty: root,
            }],
            None,
        ),
        &Limits::default(),
    )
    .unwrap()
}

proptest! {
    #[test]
    fn canonicalization_is_idempotent(left_is_text: bool, right_is_text: bool) {
        let contract = permuted_contract(left_is_text, right_is_text, false);
        prop_assert_eq!(contract.canonicalize().unwrap(), contract);
    }

    #[test]
    fn input_arena_permutations_preserve_canonical_identity(left_is_text: bool, right_is_text: bool) {
        let ordered = permuted_contract(left_is_text, right_is_text, false);
        let permuted = permuted_contract(left_is_text, right_is_text, true);
        prop_assert_eq!(&ordered, &permuted);
        prop_assert_eq!(ordered.contract_id(), permuted.contract_id());
    }

    #[test]
    fn equivalent_source_ordering_preserves_semantic_identity(reverse: bool) {
        let fields = if reverse { "right: text; left: nat" } else { "left: nat; right: text" };
        let source = format!("type Payload = record {{ {fields} }}; service : {{ read: (Payload) -> (Payload) query }};");
        let contract = candid_core::compile_did(&source).unwrap().into_parts().0;
        let canonical = candid_core::compile_did("type Payload = record { left: nat; right: text }; service : { read: (Payload) -> (Payload) query };").unwrap().into_parts().0;
        prop_assert_eq!(contract, canonical);
    }


    #[test]
    fn source_id_parse_serde_round_trip_preserves_normalized_id(
        scheme in "[a-z][a-z0-9-]{0,15}",
        components in prop::collection::vec("[a-zA-Z0-9_-]{1,16}", 1..8),
    ) {
        let input = format!("{scheme}:/{}", components.join("/./"));
        let parsed = SourceId::parse(&input).unwrap();
        let json = serde_json::to_string(&parsed).unwrap();
        let deserialized: SourceId = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&deserialized, &parsed);
        prop_assert_eq!(deserialized.as_str(), format!("{scheme}:/{}", components.join("/")));
        prop_assert_eq!(deserialized.scheme(), scheme);
        prop_assert_eq!(deserialized.path(), components.join("/"));
    }
}
