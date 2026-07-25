use candid_core::{
    Actor, Contract, ContractDraft, Declaration, Field, MethodMode, PrimitiveType, ServiceMethod,
    TypeNode, TypeRef,
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
    ContractDraft::new(
        types,
        vec![Declaration {
            name: "Payload".to_string(),
            ty: root,
        }],
        None,
    )
    .build()
    .unwrap()
}

fn declarations_only_contract(names: &[&str]) -> Contract {
    ContractDraft::new(
        vec![TypeNode::Primitive {
            primitive: PrimitiveType::Nat,
        }],
        names
            .iter()
            .map(|name| Declaration {
                name: name.to_string(),
                ty: 0,
            })
            .collect(),
        None,
    )
    .build()
    .unwrap()
}

fn declaration_names(contract: &Contract) -> Vec<&str> {
    contract
        .declarations()
        .iter()
        .map(|declaration| declaration.name.as_str())
        .collect()
}

/// Issue #14: canonical declaration order compares names as UTF-8 bytes
/// (equivalently, Unicode scalar values). U+FF61 encodes as EF BD A1 and
/// U+10000 as F0 90 80 80, so the supplementary-plane name sorts *after* the
/// high-BMP name — the opposite of a UTF-16 code-unit comparison, which
/// would see the surrogate pair D800 DC00 first. Dynamic Unicode only ever
/// orders graph collections like this array; canonical-JSON *object keys*
/// are fixed ASCII schema keys, where UTF-8 and UTF-16 order coincide.
#[test]
fn canonical_declaration_order_is_utf8_scalar_order_not_utf16() {
    let contract = declarations_only_contract(&["\u{10000}", "\u{ff61}"]);
    assert_eq!(declaration_names(&contract), ["\u{ff61}", "\u{10000}"]);
}

/// Issue #14: canonicalization performs no Unicode normalization. The NFC
/// and NFD spellings of "é" are canonically equivalent but byte-different;
/// both are preserved exactly and remain distinct declarations, and the
/// serialized JSON carries both spellings unescaped.
#[test]
fn canonically_equivalent_spellings_stay_distinct_and_unnormalized() {
    let nfc = "\u{e9}";
    let nfd = "e\u{301}";
    let contract = declarations_only_contract(&[nfc, nfd]);
    // 0x65 ('e') sorts before 0xC3 (the first UTF-8 byte of U+00E9).
    assert_eq!(declaration_names(&contract), [nfd, nfc]);

    let json = contract.to_json_pretty().unwrap();
    assert!(json.contains(nfc) && json.contains(nfd));

    let nfc_only = declarations_only_contract(&[nfc]);
    let nfd_only = declarations_only_contract(&[nfd]);
    assert_ne!(nfc_only.contract_id(), nfd_only.contract_id());
}

/// Issue #14: dynamic Unicode lives in JSON string *values*; every object
/// key of the serialized Contract stays a fixed ASCII schema key even when
/// declaration and method names are exotic. This is what keeps the
/// constrained UTF-8-ordered writer JCS-compatible for identity payloads.
#[test]
fn serialized_contract_object_keys_stay_fixed_ascii_schema_keys() {
    fn assert_ascii_keys(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        key.bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                        "object key {key:?} is not a fixed ASCII schema key"
                    );
                    assert_ascii_keys(value);
                }
            }
            serde_json::Value::Array(values) => values.iter().for_each(assert_ascii_keys),
            _ => {}
        }
    }

    let contract = ContractDraft::new(
        vec![
            TypeNode::Service {
                methods: vec![ServiceMethod {
                    name: "m\u{e9}thode".to_string(),
                    id: candid_parser::candid::idl_hash("m\u{e9}thode"),
                    function: 1,
                }],
            },
            TypeNode::Func {
                args: vec![],
                results: vec![2],
                mode: MethodMode::Query,
            },
            TypeNode::Primitive {
                primitive: PrimitiveType::Text,
            },
        ],
        vec![Declaration {
            name: "\u{10000}\u{ff61}\"\\\n".to_string(),
            ty: 2,
        }],
        Some(Actor::Service { service: 0 }),
    )
    .build()
    .unwrap();
    assert_ascii_keys(&serde_json::to_value(&contract).unwrap());
}

/// A deliberately nasty arena: a service whose two colliding-hash methods
/// reference bisimilar duplicate func nodes, plus mutually recursive records
/// behind opt edges. `idl_hash("jhwlzguu") == idl_hash("jsyrjsvk")`.
fn nasty_graph() -> (Vec<TypeNode>, Vec<Declaration>, Option<Actor>) {
    let collision = candid_parser::candid::idl_hash("jhwlzguu");
    assert_eq!(collision, candid_parser::candid::idl_hash("jsyrjsvk"));
    let types = vec![
        TypeNode::Service {
            methods: vec![
                ServiceMethod {
                    name: "jsyrjsvk".to_string(),
                    id: collision,
                    function: 2,
                },
                ServiceMethod {
                    name: "jhwlzguu".to_string(),
                    id: collision,
                    function: 1,
                },
            ],
        },
        TypeNode::Func {
            args: vec![3],
            results: vec![],
            mode: MethodMode::Update,
        },
        TypeNode::Func {
            args: vec![3],
            results: vec![],
            mode: MethodMode::Update,
        },
        TypeNode::Record {
            fields: vec![Field { id: 98, ty: 4 }],
        },
        TypeNode::Opt { inner: 5 },
        TypeNode::Record {
            fields: vec![Field { id: 97, ty: 6 }],
        },
        TypeNode::Opt { inner: 3 },
    ];
    let declarations = vec![
        Declaration {
            name: "A".to_string(),
            ty: 3,
        },
        Declaration {
            name: "B".to_string(),
            ty: 5,
        },
    ];
    (types, declarations, Some(Actor::Service { service: 0 }))
}

fn remap_node(node: &TypeNode, remap: &dyn Fn(TypeRef) -> TypeRef) -> TypeNode {
    match node {
        TypeNode::Primitive { primitive } => TypeNode::Primitive {
            primitive: *primitive,
        },
        TypeNode::Opt { inner } => TypeNode::Opt {
            inner: remap(*inner),
        },
        TypeNode::Vec { inner } => TypeNode::Vec {
            inner: remap(*inner),
        },
        TypeNode::Record { fields } => TypeNode::Record {
            fields: fields
                .iter()
                .map(|field| Field {
                    id: field.id,
                    ty: remap(field.ty),
                })
                .collect(),
        },
        TypeNode::Variant { fields } => TypeNode::Variant {
            fields: fields
                .iter()
                .map(|field| Field {
                    id: field.id,
                    ty: remap(field.ty),
                })
                .collect(),
        },
        TypeNode::Func {
            args,
            results,
            mode,
        } => TypeNode::Func {
            args: args.iter().map(|reference| remap(*reference)).collect(),
            results: results.iter().map(|reference| remap(*reference)).collect(),
            mode: *mode,
        },
        TypeNode::Service { methods } => TypeNode::Service {
            methods: methods
                .iter()
                .map(|method| ServiceMethod {
                    name: method.name.clone(),
                    id: method.id,
                    function: remap(method.function),
                })
                .collect(),
        },
        TypeNode::Class { init, service } => TypeNode::Class {
            init: init.iter().map(|reference| remap(*reference)).collect(),
            service: remap(*service),
        },
    }
}

proptest! {
    /// Any arena permutation of the nasty graph — with declarations listed in
    /// any order — canonicalizes to the identical Contract and identities.
    #[test]
    fn arbitrary_arena_permutations_of_a_nasty_graph_converge(
        placement in Just((0..7usize).collect::<Vec<_>>()).prop_shuffle(),
        declaration_order in Just(vec![0usize, 1]).prop_shuffle(),
    ) {
        let (types, declarations, actor) = nasty_graph();
        let baseline = ContractDraft::new(types.clone(), declarations.clone(), actor.clone())
            .build()
            .unwrap();

        // placement[new] = old; old_to_new inverts it.
        let mut old_to_new = vec![0 as TypeRef; placement.len()];
        for (new, old) in placement.iter().enumerate() {
            old_to_new[*old] = new as TypeRef;
        }
        let remap = |reference: TypeRef| old_to_new[reference as usize];
        let permuted_types: Vec<TypeNode> = placement
            .iter()
            .map(|old| remap_node(&types[*old], &remap))
            .collect();
        let permuted_declarations: Vec<Declaration> = declaration_order
            .iter()
            .map(|index| Declaration {
                name: declarations[*index].name.clone(),
                ty: remap(declarations[*index].ty),
            })
            .collect();
        let permuted_actor = actor.map(|actor| match actor {
            Actor::Service { service } => Actor::Service { service: remap(service) },
            Actor::Class { class } => Actor::Class { class: remap(class) },
        });

        let permuted = ContractDraft::new(permuted_types, permuted_declarations, permuted_actor)
            .build()
            .unwrap();
        prop_assert_eq!(&permuted, &baseline);
        prop_assert_eq!(permuted.contract_id(), baseline.contract_id());
        prop_assert_eq!(permuted.interface_id(), baseline.interface_id());
    }

    /// Canonical declaration order is exactly "sort by UTF-8 name bytes",
    /// independent of listing order, across ASCII, combining marks, high-BMP,
    /// and supplementary-plane names.
    #[test]
    fn declaration_names_sort_by_utf8_bytes(
        names in proptest::sample::subsequence(
            vec![
                "a", "z", "_tail", "0digit", "~tilde",
                "e\u{301}", "\u{e9}", "\u{ff61}", "\u{10000}", "\u{10ffff}",
                "esc\"\\\nape",
            ],
            1..=6,
        ).prop_shuffle(),
    ) {
        let contract = declarations_only_contract(&names);
        let mut sorted = names.clone();
        sorted.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        prop_assert_eq!(declaration_names(&contract), sorted);
    }
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

}

/// The two properties that need a Candid source engine. Kept in their own
/// `proptest!` block so the model properties above still run with defaults
/// disabled.
#[cfg(feature = "compiler")]
mod compiler_properties {
    use candid_core::SourceId;
    use proptest::prelude::*;

    proptest! {
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
            scheme in "[a-z][a-z0-9-]{1,15}",
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
}
