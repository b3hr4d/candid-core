use crate::model::{
    Actor, Contract, ContractValidationError, Declaration, Field, MethodMode, PrimitiveType,
    ServiceMethod, TypeNode, TypeRef,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Validate structural rules, then minimize semantic-equivalent nodes and
/// deterministically re-index the arena before calculating its fingerprint.
pub(crate) fn canonicalize_contract(
    contract: &Contract,
) -> Result<Contract, ContractValidationError> {
    crate::validate::validate_structure(contract)?;
    Ok(canonicalize_with_mapping_unchecked(contract).contract)
}

pub(crate) struct Canonicalized {
    pub contract: Contract,
    /// Maps every input node to its canonical node. Several input nodes can map
    /// to one output node when they are semantically bisimilar.
    pub old_to_new: Vec<TypeRef>,
}

pub(crate) fn canonicalize_with_mapping_unchecked(contract: &Contract) -> Canonicalized {
    let quotient = quotient_semantic_nodes(contract);
    let indexed = canonicalize_indexed(&quotient.contract);
    Canonicalized {
        contract: indexed.contract,
        old_to_new: quotient
            .old_to_quotient
            .into_iter()
            .map(|reference| indexed.old_to_new[reference as usize])
            .collect(),
    }
}

pub(crate) fn semantic_fingerprint(contract: &Contract) -> String {
    fingerprint_for_canonical(&canonicalize_with_mapping_unchecked(contract).contract)
}

struct QuotientGraph {
    contract: Contract,
    old_to_quotient: Vec<TypeRef>,
}

/// Candid type definitions are equi-recursive: aliases and duplicate source
/// definitions do not create a new semantic wire type. Partition refinement
/// computes the finite graph's greatest labelled bisimulation, which gives a
/// stable quotient before any numeric TypeRef ordering is considered.
fn quotient_semantic_nodes(contract: &Contract) -> QuotientGraph {
    let classes = semantic_classes(&contract.types);
    let class_count = classes.iter().copied().max().map_or(0, |class| class + 1);
    let mut representatives = vec![None; class_count];
    for (index, class) in classes.iter().copied().enumerate() {
        representatives[class].get_or_insert(index);
    }

    let types = representatives
        .into_iter()
        .map(|representative| {
            let representative = representative.expect("every partition class has a member");
            remap_node_to_classes(&contract.types[representative], &classes)
        })
        .collect();
    let old_to_quotient: Vec<_> = classes.iter().copied().map(class_to_ref).collect();
    let remap = |reference: TypeRef| old_to_quotient[reference as usize];

    let declarations = contract
        .declarations
        .iter()
        .map(|declaration| Declaration {
            name: declaration.name.clone(),
            ty: remap(declaration.ty),
        })
        .collect();
    let actor = contract.actor.as_ref().map(|actor| match actor {
        Actor::Service { service } => Actor::Service {
            service: remap(*service),
        },
        Actor::Class { class } => Actor::Class {
            class: remap(*class),
        },
    });

    QuotientGraph {
        contract: Contract {
            contract_version: contract.contract_version,
            fingerprint: contract.fingerprint.clone(),
            types,
            declarations,
            actor,
        },
        old_to_quotient,
    }
}

fn semantic_classes(types: &[TypeNode]) -> Vec<usize> {
    let mut classes =
        assign_partition_ids(types.iter().map(local_signature).collect::<Vec<Vec<u8>>>());

    loop {
        let next = assign_partition_ids(
            types
                .iter()
                .enumerate()
                .map(|(index, node)| refined_signature(node, classes[index], &classes))
                .collect(),
        );
        if !partition_was_split(&classes, &next) {
            return next;
        }
        classes = next;
    }
}

fn assign_partition_ids(signatures: Vec<Vec<u8>>) -> Vec<usize> {
    let mut ids = BTreeMap::new();
    for signature in &signatures {
        ids.entry(signature.clone()).or_insert(usize::MAX);
    }
    for (index, id) in ids.values_mut().enumerate() {
        *id = index;
    }
    signatures.iter().map(|signature| ids[signature]).collect()
}

fn partition_was_split(previous: &[usize], next: &[usize]) -> bool {
    let mut first_next_by_previous = BTreeMap::new();
    for (previous, next) in previous.iter().zip(next) {
        match first_next_by_previous.entry(*previous) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(*next);
            }
            std::collections::btree_map::Entry::Occupied(entry) if *entry.get() != *next => {
                return true
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
    }
    false
}

fn local_signature(node: &TypeNode) -> Vec<u8> {
    let mut output = Vec::new();
    match node {
        TypeNode::Primitive { primitive } => {
            output.push(0);
            output.push(primitive_tag(*primitive));
        }
        TypeNode::Opt { .. } => output.push(1),
        TypeNode::Vec { .. } => output.push(2),
        TypeNode::Record { fields } => {
            output.push(3);
            let mut fields = fields.to_vec();
            fields.sort_by(field_order);
            write_len(&mut output, fields.len());
            for field in fields {
                write_u32(&mut output, field.id);
            }
        }
        TypeNode::Variant { fields } => {
            output.push(4);
            let mut fields = fields.to_vec();
            fields.sort_by(field_order);
            write_len(&mut output, fields.len());
            for field in fields {
                write_u32(&mut output, field.id);
            }
        }
        TypeNode::Func {
            args,
            results,
            mode,
        } => {
            output.extend([5, mode_tag(*mode)]);
            write_len(&mut output, args.len());
            write_len(&mut output, results.len());
        }
        TypeNode::Service { methods } => {
            output.push(6);
            let mut methods = methods.to_vec();
            methods.sort_by(method_order);
            write_len(&mut output, methods.len());
            for method in methods {
                write_u32(&mut output, method.id);
                write_string(&mut output, &method.name);
            }
        }
        TypeNode::Class { init, .. } => {
            output.push(7);
            write_len(&mut output, init.len());
        }
    }
    output
}

fn refined_signature(node: &TypeNode, own_class: usize, classes: &[usize]) -> Vec<u8> {
    let mut output = local_signature(node);
    output.push(255);
    write_usize(&mut output, own_class);
    for child in sorted_children(node) {
        write_usize(&mut output, classes[child as usize]);
    }
    output
}

fn sorted_children(node: &TypeNode) -> Vec<TypeRef> {
    match node {
        TypeNode::Primitive { .. } => Vec::new(),
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => vec![*inner],
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            let mut fields = fields.to_vec();
            fields.sort_by(field_order);
            fields.into_iter().map(|field| field.ty).collect()
        }
        TypeNode::Func { args, results, .. } => args.iter().chain(results).copied().collect(),
        TypeNode::Service { methods } => {
            let mut methods = methods.to_vec();
            methods.sort_by(method_order);
            methods.into_iter().map(|method| method.function).collect()
        }
        TypeNode::Class { init, service } => init
            .iter()
            .copied()
            .chain(std::iter::once(*service))
            .collect(),
    }
}

fn remap_node_to_classes(node: &TypeNode, classes: &[usize]) -> TypeNode {
    let remap = |reference: TypeRef| class_to_ref(classes[reference as usize]);
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
            fields: remap_fields_with(fields, remap),
        },
        TypeNode::Variant { fields } => TypeNode::Variant {
            fields: remap_fields_with(fields, remap),
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
        TypeNode::Service { methods } => {
            let mut methods = methods.clone();
            methods.sort_by(method_order);
            TypeNode::Service {
                methods: methods
                    .into_iter()
                    .map(|method| ServiceMethod {
                        name: method.name,
                        id: method.id,
                        function: remap(method.function),
                    })
                    .collect(),
            }
        }
        TypeNode::Class { init, service } => TypeNode::Class {
            init: init.iter().map(|reference| remap(*reference)).collect(),
            service: remap(*service),
        },
    }
}

struct IndexedCanonical {
    contract: Contract,
    old_to_new: Vec<TypeRef>,
}

/// The quotient has one node per semantic type class. Its partition IDs are
/// deterministic, so numeric fallback below is safe and cannot depend on the
/// input arena's TypeRef assignment.
fn canonicalize_indexed(contract: &Contract) -> IndexedCanonical {
    let mut old_to_new = vec![None; contract.types.len()];
    let mut output = Vec::<Option<TypeNode>>::new();

    if let Some(actor) = &contract.actor {
        visit(
            actor_type_ref(actor),
            &contract.types,
            &mut old_to_new,
            &mut output,
        );
    }

    let mut declaration_roots: Vec<_> = contract.declarations.iter().map(|d| d.ty).collect();
    declaration_roots.sort_unstable();
    declaration_roots.dedup();
    for root in declaration_roots {
        visit(root, &contract.types, &mut old_to_new, &mut output);
    }

    for root in (0..contract.types.len()).map(|index| index as TypeRef) {
        if old_to_new[root as usize].is_none() {
            visit(root, &contract.types, &mut old_to_new, &mut output);
        }
    }

    let remap = |reference: TypeRef| -> TypeRef {
        old_to_new[reference as usize]
            .expect("validated Contract references must have a canonical mapping")
    };
    let mut declarations: Vec<Declaration> = contract
        .declarations
        .iter()
        .map(|declaration| Declaration {
            name: declaration.name.clone(),
            ty: remap(declaration.ty),
        })
        .collect();
    declarations.sort_by(|left, right| left.name.cmp(&right.name).then(left.ty.cmp(&right.ty)));

    let actor = contract.actor.as_ref().map(|actor| match actor {
        Actor::Service { service } => Actor::Service {
            service: remap(*service),
        },
        Actor::Class { class } => Actor::Class {
            class: remap(*class),
        },
    });
    let types = output
        .into_iter()
        .map(|node| node.expect("canonical traversal fills every reserved node"))
        .collect();
    let mut contract = Contract {
        contract_version: contract.contract_version,
        fingerprint: String::new(),
        types,
        declarations,
        actor,
    };
    contract.fingerprint = fingerprint_for_canonical(&contract);
    IndexedCanonical {
        contract,
        old_to_new: old_to_new
            .into_iter()
            .map(|reference| {
                reference.expect("validated Contract nodes must have canonical mappings")
            })
            .collect(),
    }
}

fn fingerprint_for_canonical(contract: &Contract) -> String {
    #[derive(Serialize)]
    struct SemanticPayload<'a> {
        contract_version: u32,
        types: &'a [TypeNode],
        actor: &'a Option<Actor>,
    }

    let payload = SemanticPayload {
        contract_version: contract.contract_version,
        types: &contract.types,
        actor: &contract.actor,
    };
    let bytes = serde_json::to_vec(&payload)
        .expect("the built-in semantic Contract model must always serialize to JSON");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn actor_type_ref(actor: &Actor) -> TypeRef {
    match actor {
        Actor::Service { service } => *service,
        Actor::Class { class } => *class,
    }
}

fn visit(
    old: TypeRef,
    types: &[TypeNode],
    old_to_new: &mut [Option<TypeRef>],
    output: &mut Vec<Option<TypeNode>>,
) -> TypeRef {
    if let Some(mapped) = old_to_new[old as usize] {
        return mapped;
    }
    let new = output.len() as TypeRef;
    old_to_new[old as usize] = Some(new);
    output.push(None);
    let node = rewrite_node(old, types, old_to_new, output);
    output[new as usize] = Some(node);
    new
}

fn rewrite_node(
    old: TypeRef,
    types: &[TypeNode],
    old_to_new: &mut [Option<TypeRef>],
    output: &mut Vec<Option<TypeNode>>,
) -> TypeNode {
    let map =
        |reference, old_to_new: &mut [Option<TypeRef>], output: &mut Vec<Option<TypeNode>>| {
            visit(reference, types, old_to_new, output)
        };
    match &types[old as usize] {
        TypeNode::Primitive { primitive } => TypeNode::Primitive {
            primitive: *primitive,
        },
        TypeNode::Opt { inner } => TypeNode::Opt {
            inner: map(*inner, old_to_new, output),
        },
        TypeNode::Vec { inner } => TypeNode::Vec {
            inner: map(*inner, old_to_new, output),
        },
        TypeNode::Record { fields } => TypeNode::Record {
            fields: remap_fields(fields, &map, old_to_new, output),
        },
        TypeNode::Variant { fields } => TypeNode::Variant {
            fields: remap_fields(fields, &map, old_to_new, output),
        },
        TypeNode::Func {
            args,
            results,
            mode,
        } => TypeNode::Func {
            args: args
                .iter()
                .map(|reference| map(*reference, old_to_new, output))
                .collect(),
            results: results
                .iter()
                .map(|reference| map(*reference, old_to_new, output))
                .collect(),
            mode: *mode,
        },
        TypeNode::Service { methods } => {
            let mut methods = methods.clone();
            methods.sort_by(method_order);
            TypeNode::Service {
                methods: methods
                    .into_iter()
                    .map(|method| ServiceMethod {
                        name: method.name,
                        id: method.id,
                        function: map(method.function, old_to_new, output),
                    })
                    .collect(),
            }
        }
        TypeNode::Class { init, service } => TypeNode::Class {
            init: init
                .iter()
                .map(|reference| map(*reference, old_to_new, output))
                .collect(),
            service: map(*service, old_to_new, output),
        },
    }
}

fn remap_fields(
    fields: &[Field],
    map: &impl Fn(TypeRef, &mut [Option<TypeRef>], &mut Vec<Option<TypeNode>>) -> TypeRef,
    old_to_new: &mut [Option<TypeRef>],
    output: &mut Vec<Option<TypeNode>>,
) -> Vec<Field> {
    let mut fields = fields.to_vec();
    fields.sort_by(field_order);
    fields
        .into_iter()
        .map(|field| Field {
            id: field.id,
            ty: map(field.ty, old_to_new, output),
        })
        .collect()
}

fn remap_fields_with(fields: &[Field], remap: impl Fn(TypeRef) -> TypeRef) -> Vec<Field> {
    let mut fields = fields.to_vec();
    fields.sort_by(field_order);
    fields
        .into_iter()
        .map(|field| Field {
            id: field.id,
            ty: remap(field.ty),
        })
        .collect()
}

fn field_order(left: &Field, right: &Field) -> Ordering {
    left.id.cmp(&right.id).then(left.ty.cmp(&right.ty))
}

fn method_order(left: &ServiceMethod, right: &ServiceMethod) -> Ordering {
    left.id
        .cmp(&right.id)
        .then(left.name.cmp(&right.name))
        .then(left.function.cmp(&right.function))
}

fn class_to_ref(class: usize) -> TypeRef {
    u32::try_from(class).expect("a Contract arena uses u32 type references")
}

fn write_len(output: &mut Vec<u8>, length: usize) {
    write_usize(output, length);
}

fn write_usize(output: &mut Vec<u8>, value: usize) {
    write_u32(
        output,
        u32::try_from(value).expect("a Contract graph cannot exceed u32 entries"),
    );
}

fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend(value.to_be_bytes());
}

fn write_string(output: &mut Vec<u8>, value: &str) {
    write_len(output, value.len());
    output.extend(value.as_bytes());
}

fn primitive_tag(primitive: PrimitiveType) -> u8 {
    match primitive {
        PrimitiveType::Null => 0,
        PrimitiveType::Bool => 1,
        PrimitiveType::Nat => 2,
        PrimitiveType::Int => 3,
        PrimitiveType::Nat8 => 4,
        PrimitiveType::Nat16 => 5,
        PrimitiveType::Nat32 => 6,
        PrimitiveType::Nat64 => 7,
        PrimitiveType::Int8 => 8,
        PrimitiveType::Int16 => 9,
        PrimitiveType::Int32 => 10,
        PrimitiveType::Int64 => 11,
        PrimitiveType::Float32 => 12,
        PrimitiveType::Float64 => 13,
        PrimitiveType::Text => 14,
        PrimitiveType::Reserved => 15,
        PrimitiveType::Empty => 16,
        PrimitiveType::Principal => 17,
    }
}

fn mode_tag(mode: MethodMode) -> u8 {
    match mode {
        MethodMode::Update => 0,
        MethodMode::Query => 1,
        MethodMode::CompositeQuery => 2,
        MethodMode::Oneway => 3,
    }
}
