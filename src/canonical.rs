use crate::budget::Budget;
use crate::model::{
    Actor, Contract, ContractIdentities, ContractValidationError, Declaration, Field, MethodMode,
    PrimitiveType, ServiceMethod, TypeNode, TypeRef,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};

/// Validate structural rules, then minimize semantic-equivalent nodes and
/// deterministically re-index the arena before calculating its identities.
pub(crate) fn canonicalize_contract_with_budget(
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<Contract, ContractValidationError> {
    crate::validate::validate_structure_with_budget(contract, budget)?;
    Ok(canonicalize_with_mapping_unchecked_with_budget(contract, budget)?.contract)
}

pub(crate) struct Canonicalized {
    pub contract: Contract,
    /// Maps every input node to its canonical node. Several input nodes can map
    /// to one output node when they are semantically bisimilar.
    pub old_to_new: Vec<TypeRef>,
}

pub(crate) fn canonicalize_with_mapping_unchecked_with_budget(
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<Canonicalized, ContractValidationError> {
    let quotient = quotient_semantic_nodes(contract, budget)?;
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let indexed = canonicalize_indexed(&quotient.contract, budget)?;
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    Ok(Canonicalized {
        contract: indexed.contract,
        old_to_new: quotient
            .old_to_quotient
            .into_iter()
            .map(|reference| indexed.old_to_new[reference as usize])
            .collect(),
    })
}

struct QuotientGraph {
    contract: Contract,
    old_to_quotient: Vec<TypeRef>,
}

/// Candid type definitions are equi-recursive: aliases and duplicate source
/// definitions do not create a new semantic wire type. Partition refinement
/// computes the finite graph's greatest labelled bisimulation, which gives a
/// stable quotient before any numeric TypeRef ordering is considered.
fn quotient_semantic_nodes(
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<QuotientGraph, ContractValidationError> {
    let classes = semantic_classes(&contract.types, budget)?;
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

    Ok(QuotientGraph {
        contract: Contract {
            format: contract.format.clone(),
            format_version: contract.format_version,
            semantics_profile: contract.semantics_profile.clone(),
            canonicalization_profile: contract.canonicalization_profile.clone(),
            identities: contract.identities.clone(),
            producer: contract.producer.clone(),
            types,
            declarations,
            actor,
        },
        old_to_quotient,
    })
}

fn semantic_classes(
    types: &[TypeNode],
    budget: &mut Budget<'_>,
) -> Result<Vec<usize>, ContractValidationError> {
    let max_work = budget.limits().max_canonicalization_work;
    let round_work = signature_round_work(types);
    budget
        .charge("canonicalization_work", max_work, round_work)
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let mut classes =
        assign_partition_ids(types.iter().map(local_signature).collect::<Vec<Vec<u8>>>());

    loop {
        budget
            .charge("canonicalization_work", max_work, round_work)
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        let next = assign_partition_ids(
            types
                .iter()
                .enumerate()
                .map(|(index, node)| refined_signature(node, classes[index], &classes))
                .collect(),
        );
        if !partition_was_split(&classes, &next) {
            return Ok(next);
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
fn canonicalize_indexed(
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<IndexedCanonical, ContractValidationError> {
    let max_work = budget.limits().max_canonicalization_work;
    budget
        .charge("canonicalization_work", max_work, reindex_work(contract))
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let mut old_to_new = vec![None; contract.types.len()];
    let mut new_to_old = Vec::<TypeRef>::new();

    if let Some(actor) = &contract.actor {
        visit_iterative(
            actor_type_ref(actor),
            &contract.types,
            &mut old_to_new,
            &mut new_to_old,
        );
    }

    let mut declaration_roots: Vec<_> = contract.declarations.iter().map(|d| d.ty).collect();
    declaration_roots.sort_unstable();
    declaration_roots.dedup();
    for root in declaration_roots {
        visit_iterative(root, &contract.types, &mut old_to_new, &mut new_to_old);
    }

    for root in (0..contract.types.len()).map(|index| index as TypeRef) {
        if old_to_new[root as usize].is_none() {
            visit_iterative(root, &contract.types, &mut old_to_new, &mut new_to_old);
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
    let types = new_to_old
        .into_iter()
        .map(|old| rewrite_node_with(old, &contract.types, &remap))
        .collect();
    let mut contract = Contract {
        format: contract.format.clone(),
        format_version: contract.format_version,
        semantics_profile: contract.semantics_profile.clone(),
        canonicalization_profile: contract.canonicalization_profile.clone(),
        identities: ContractIdentities {
            contract: String::new(),
            interface: None,
        },
        producer: contract.producer.clone(),
        types,
        declarations,
        actor,
    };
    contract.identities = identities_for_canonical(&contract, budget)?;
    Ok(IndexedCanonical {
        contract,
        old_to_new: old_to_new
            .into_iter()
            .map(|reference| {
                reference.expect("validated Contract nodes must have canonical mappings")
            })
            .collect(),
    })
}

fn identities_for_canonical(
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<ContractIdentities, ContractValidationError> {
    #[derive(Serialize)]
    struct ContractPayload<'a> {
        format: &'a str,
        format_version: u32,
        semantics_profile: &'a str,
        canonicalization_profile: &'a str,
        types: &'a [TypeNode],
        declarations: &'a [Declaration],
        actor: &'a Option<Actor>,
    }

    #[derive(Serialize)]
    struct InterfacePayload<'a> {
        semantics_profile: &'a str,
        canonicalization_profile: &'a str,
        types: &'a [TypeNode],
        actor: &'a Actor,
    }

    let contract_payload = ContractPayload {
        format: &contract.format,
        format_version: contract.format_version,
        semantics_profile: &contract.semantics_profile,
        canonicalization_profile: &contract.canonicalization_profile,
        types: &contract.types,
        declarations: &contract.declarations,
        actor: &contract.actor,
    };
    let contract_id =
        domain_hash_with_budget("candid-core:contract:v1", &contract_payload, budget)?;

    let interface = contract
        .actor
        .as_ref()
        .map(|actor| {
            let reachable = reachable_from(actor_type_ref(actor), &contract.types);
            let prefix_len = reachable.iter().take_while(|reached| **reached).count();
            debug_assert!(reachable[prefix_len..].iter().all(|reached| !reached));
            let payload = InterfacePayload {
                semantics_profile: &contract.semantics_profile,
                canonicalization_profile: &contract.canonicalization_profile,
                types: &contract.types[..prefix_len],
                actor,
            };
            domain_hash_with_budget("candid-core:interface:v1", &payload, budget)
        })
        .transpose()?;

    Ok(ContractIdentities {
        contract: contract_id,
        interface,
    })
}

pub(crate) fn domain_hash(domain: &str, payload: &impl Serialize) -> String {
    let value = serde_json::to_value(payload)
        .expect("built-in Contract identity payloads must serialize to JSON");
    let canonical = jcs_bytes(&value);
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(canonical);
    format!("{domain}:sha256:{}", hex::encode(hasher.finalize()))
}

fn domain_hash_with_budget(
    domain: &str,
    payload: &impl Serialize,
    budget: &mut Budget<'_>,
) -> Result<String, ContractValidationError> {
    let value = serde_json::to_value(payload)
        .expect("built-in Contract identity payloads must serialize to JSON");
    let canonical = jcs_bytes(&value);
    // One unit for producing each canonical byte and one for hashing it. The
    // domain separator is hashed as well.
    let work = canonical
        .len()
        .saturating_mul(2)
        .saturating_add(domain.len())
        .saturating_add(1);
    let max_work = budget.limits().max_canonicalization_work;
    budget
        .charge("canonicalization_work", max_work, work)
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(canonical);
    Ok(format!(
        "{domain}:sha256:{}",
        hex::encode(hasher.finalize())
    ))
}

fn signature_round_work(types: &[TypeNode]) -> usize {
    types.iter().fold(0usize, |total, node| {
        let (edges, bytes, sortable) = node_work(node);
        total
            .saturating_add(1)
            .saturating_add(edges)
            .saturating_add(bytes)
            .saturating_add(sort_comparison_work(sortable))
    })
}

fn reindex_work(contract: &Contract) -> usize {
    let graph = signature_round_work(&contract.types);
    let declaration_bytes = contract
        .declarations
        .iter()
        .map(|declaration| declaration.name.len().saturating_add(1))
        .sum::<usize>();
    graph
        .saturating_mul(2) // traversal plus node rewriting
        .saturating_add(declaration_bytes)
        .saturating_add(sort_comparison_work(contract.declarations.len()))
}

fn node_work(node: &TypeNode) -> (usize, usize, usize) {
    match node {
        TypeNode::Primitive { .. } => (0, 2, 0),
        TypeNode::Opt { .. } | TypeNode::Vec { .. } => (1, 1, 0),
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            (fields.len(), fields.len().saturating_mul(4), fields.len())
        }
        TypeNode::Func { args, results, .. } => (args.len().saturating_add(results.len()), 10, 0),
        TypeNode::Service { methods } => (
            methods.len(),
            methods.iter().fold(0usize, |bytes, method| {
                bytes.saturating_add(4).saturating_add(method.name.len())
            }),
            methods.len(),
        ),
        TypeNode::Class { init, .. } => (init.len().saturating_add(1), 5, 0),
    }
}

fn sort_comparison_work(length: usize) -> usize {
    if length < 2 {
        return 0;
    }
    let log = usize::BITS as usize - (length - 1).leading_zeros() as usize;
    length.saturating_mul(log)
}

fn jcs_bytes(value: &Value) -> Vec<u8> {
    fn write(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null => output.extend_from_slice(b"null"),
            Value::Bool(true) => output.extend_from_slice(b"true"),
            Value::Bool(false) => output.extend_from_slice(b"false"),
            Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
            Value::String(string) => output.extend_from_slice(
                serde_json::to_string(string)
                    .expect("JSON strings always serialize")
                    .as_bytes(),
            ),
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output);
                }
                output.push(b']');
            }
            Value::Object(object) => {
                output.push(b'{');
                let mut entries: Vec<_> = object.iter().collect();
                entries.sort_by_key(|(key, _)| *key);
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend_from_slice(
                        serde_json::to_string(key)
                            .expect("JSON object keys always serialize")
                            .as_bytes(),
                    );
                    output.push(b':');
                    write(value, output);
                }
                output.push(b'}');
            }
        }
    }

    let mut output = Vec::new();
    write(value, &mut output);
    output
}

fn actor_type_ref(actor: &Actor) -> TypeRef {
    match actor {
        Actor::Service { service } => *service,
        Actor::Class { class } => *class,
    }
}

fn visit_iterative(
    old: TypeRef,
    types: &[TypeNode],
    old_to_new: &mut [Option<TypeRef>],
    new_to_old: &mut Vec<TypeRef>,
) {
    let mut stack = vec![old];
    while let Some(reference) = stack.pop() {
        if old_to_new[reference as usize].is_some() {
            continue;
        }
        let new = new_to_old.len() as TypeRef;
        old_to_new[reference as usize] = Some(new);
        new_to_old.push(reference);
        let children = sorted_children(&types[reference as usize]);
        stack.extend(children.into_iter().rev());
    }
}

fn rewrite_node_with(
    old: TypeRef,
    types: &[TypeNode],
    remap: &impl Fn(TypeRef) -> TypeRef,
) -> TypeNode {
    match &types[old as usize] {
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

fn reachable_from(root: TypeRef, types: &[TypeNode]) -> Vec<bool> {
    let mut reached = vec![false; types.len()];
    let mut work = VecDeque::from([root]);
    while let Some(reference) = work.pop_front() {
        if reached[reference as usize] {
            continue;
        }
        reached[reference as usize] = true;
        work.extend(sorted_children(&types[reference as usize]));
    }
    reached
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
