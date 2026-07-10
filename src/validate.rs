use crate::canonical;
use crate::model::{
    Actor, Contract, ContractValidationError, ContractViolation, MethodMode, ServiceMethod,
    TypeNode, TypeRef, CONTRACT_VERSION,
};
use std::collections::{BTreeSet, VecDeque};

pub(crate) fn validate_contract(contract: &Contract) -> Result<(), ContractValidationError> {
    validate_structure(contract)?;
    let expected = canonical::semantic_fingerprint(contract);
    if contract.fingerprint != expected {
        return Err(ContractValidationError::single(
            "fingerprint_mismatch",
            "$.fingerprint",
            format!("expected {expected}, found {}", contract.fingerprint),
        ));
    }
    Ok(())
}

/// Checks only JSON/graph invariants. Fingerprint verification is intentionally
/// separate so the compiler can canonicalize a newly built graph before it has
/// a fingerprint.
pub(crate) fn validate_structure(contract: &Contract) -> Result<(), ContractValidationError> {
    let mut violations = Vec::new();
    if contract.contract_version != CONTRACT_VERSION {
        violation(
            &mut violations,
            "unsupported_contract_version",
            "$.contract_version",
            format!(
                "expected Contract version {CONTRACT_VERSION}, found {}",
                contract.contract_version
            ),
        );
    }
    if !is_sha256_fingerprint(&contract.fingerprint) {
        violation(
            &mut violations,
            "invalid_fingerprint_format",
            "$.fingerprint",
            "fingerprint must be sha256:<64 lowercase hexadecimal characters>",
        );
    }

    let mut declaration_names = BTreeSet::new();
    for (index, declaration) in contract.declarations.iter().enumerate() {
        let base = format!("$.declarations[{index}]");
        if declaration.name.is_empty() {
            violation(
                &mut violations,
                "empty_declaration_name",
                format!("{base}.name"),
                "declaration names must not be empty",
            );
        }
        if !declaration_names.insert(&declaration.name) {
            violation(
                &mut violations,
                "duplicate_declaration_name",
                format!("{base}.name"),
                format!("duplicate declaration name {:?}", declaration.name),
            );
        }
        validate_ref(
            declaration.ty,
            &format!("{base}.type"),
            contract.types.len(),
            &mut violations,
        );
    }

    for (index, node) in contract.types.iter().enumerate() {
        validate_node(index, node, contract, &mut violations);
    }
    validate_actor(contract, &mut violations);
    validate_class_placement(contract, &mut violations);
    validate_reachability(contract, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(ContractValidationError { violations })
    }
}

fn validate_node(
    index: usize,
    node: &TypeNode,
    contract: &Contract,
    violations: &mut Vec<ContractViolation>,
) {
    let base = format!("$.types[{index}]");
    match node {
        TypeNode::Primitive { .. } => {}
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => {
            validate_ref(
                *inner,
                &format!("{base}.inner"),
                contract.types.len(),
                violations,
            );
        }
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            let mut field_ids = BTreeSet::new();
            for (field_index, field) in fields.iter().enumerate() {
                let field_base = format!("{base}.fields[{field_index}]");
                if !field_ids.insert(field.id) {
                    violation(
                        violations,
                        "duplicate_field_id",
                        format!("{field_base}.id"),
                        format!("field ID {} occurs more than once", field.id),
                    );
                }
                validate_ref(
                    field.ty,
                    &format!("{field_base}.type"),
                    contract.types.len(),
                    violations,
                );
            }
        }
        TypeNode::Func {
            args,
            results,
            mode,
        } => {
            for (argument_index, argument) in args.iter().enumerate() {
                validate_ref(
                    *argument,
                    &format!("{base}.args[{argument_index}]"),
                    contract.types.len(),
                    violations,
                );
            }
            for (result_index, result) in results.iter().enumerate() {
                validate_ref(
                    *result,
                    &format!("{base}.results[{result_index}]"),
                    contract.types.len(),
                    violations,
                );
            }
            if *mode == MethodMode::Oneway && !results.is_empty() {
                violation(
                    violations,
                    "oneway_has_results",
                    format!("{base}.results"),
                    "oneway functions must have no results",
                );
            }
        }
        TypeNode::Service { methods } => {
            validate_service_methods(index, methods, contract, violations)
        }
        TypeNode::Class { init, service } => {
            for (argument_index, argument) in init.iter().enumerate() {
                validate_ref(
                    *argument,
                    &format!("{base}.init[{argument_index}]"),
                    contract.types.len(),
                    violations,
                );
            }
            let service_path = format!("{base}.service");
            validate_ref(*service, &service_path, contract.types.len(), violations);
            if let Some(target) = contract.types.get(*service as usize) {
                if !matches!(target, TypeNode::Service { .. }) {
                    violation(
                        violations,
                        "class_service_not_service",
                        service_path,
                        "a class service reference must target a service type",
                    );
                }
            }
        }
    }
}

fn validate_service_methods(
    node_index: usize,
    methods: &[ServiceMethod],
    contract: &Contract,
    violations: &mut Vec<ContractViolation>,
) {
    let base = format!("$.types[{node_index}].methods");
    let mut names = BTreeSet::new();
    for (method_index, method) in methods.iter().enumerate() {
        let method_base = format!("{base}[{method_index}]");
        if method.name.is_empty() {
            violation(
                violations,
                "empty_method_name",
                format!("{method_base}.name"),
                "service method names must not be empty",
            );
        }
        if !names.insert(&method.name) {
            violation(
                violations,
                "duplicate_method_name",
                format!("{method_base}.name"),
                format!("duplicate service method name {:?}", method.name),
            );
        }
        let expected_id = candid_parser::candid::idl_hash(&method.name);
        if method.id != expected_id {
            violation(
                violations,
                "method_id_mismatch",
                format!("{method_base}.id"),
                format!(
                    "method ID {} does not equal Candid hash {} for {:?}",
                    method.id, expected_id, method.name
                ),
            );
        }
        let function_path = format!("{method_base}.function");
        validate_ref(
            method.function,
            &function_path,
            contract.types.len(),
            violations,
        );
        if let Some(target) = contract.types.get(method.function as usize) {
            if !matches!(target, TypeNode::Func { .. }) {
                violation(
                    violations,
                    "service_method_not_function",
                    function_path,
                    "a service method reference must target a func type",
                );
            }
        }
    }
}

fn validate_actor(contract: &Contract, violations: &mut Vec<ContractViolation>) {
    let Some(actor) = &contract.actor else {
        return;
    };
    match actor {
        Actor::Service { service } => {
            let path = "$.actor.service";
            validate_ref(*service, path, contract.types.len(), violations);
            if let Some(target) = contract.types.get(*service as usize) {
                if !matches!(target, TypeNode::Service { .. }) {
                    violation(
                        violations,
                        "actor_service_not_service",
                        path,
                        "a service actor must target a service type",
                    );
                }
            }
        }
        Actor::Class { class } => {
            let path = "$.actor.class";
            validate_ref(*class, path, contract.types.len(), violations);
            if let Some(target) = contract.types.get(*class as usize) {
                if !matches!(target, TypeNode::Class { .. }) {
                    violation(
                        violations,
                        "actor_class_not_class",
                        path,
                        "a class actor must target a class type",
                    );
                }
            }
        }
    }
}

/// Candid's `service : (args) -> service` constructor syntax exists only for
/// the top-level actor. A class is not a first-class Candid type and therefore
/// must not appear under a type edge or named declaration.
fn validate_class_placement(contract: &Contract, violations: &mut Vec<ContractViolation>) {
    let class_nodes: Vec<_> = contract
        .types
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, TypeNode::Class { .. }).then_some(index))
        .collect();
    let actor_class = match &contract.actor {
        Some(Actor::Class { class }) if (*class as usize) < contract.types.len() => Some(*class),
        _ => None,
    };

    for class in &class_nodes {
        if actor_class != Some(*class as TypeRef) {
            violation(
                violations,
                "class_not_actor_root",
                format!("$.types[{class}]"),
                "class nodes are only valid as the top-level class actor root",
            );
        }
    }
    for (index, declaration) in contract.declarations.iter().enumerate() {
        if matches!(
            contract.types.get(declaration.ty as usize),
            Some(TypeNode::Class { .. })
        ) {
            violation(
                violations,
                "class_not_actor_root",
                format!("$.declarations[{index}].type"),
                "a named declaration must not target a class node",
            );
        }
    }
    for (index, node) in contract.types.iter().enumerate() {
        for (path, reference) in type_child_paths(index, node) {
            if matches!(
                contract.types.get(reference as usize),
                Some(TypeNode::Class { .. })
            ) {
                violation(
                    violations,
                    "class_not_first_class_type",
                    path,
                    "a class cannot appear through a Candid type edge",
                );
            }
        }
    }
}

fn validate_reachability(contract: &Contract, violations: &mut Vec<ContractViolation>) {
    if contract.types.is_empty() {
        return;
    }
    let mut roots: Vec<TypeRef> = contract
        .declarations
        .iter()
        .map(|declaration| declaration.ty)
        .collect();
    if let Some(actor) = &contract.actor {
        roots.push(match actor {
            Actor::Service { service } => *service,
            Actor::Class { class } => *class,
        });
    }
    if roots.is_empty() {
        violation(
            violations,
            "rootless_type_arena",
            "$.types",
            "a non-empty arena requires an actor or at least one named declaration root",
        );
        return;
    }

    let mut reached = vec![false; contract.types.len()];
    let mut work = VecDeque::new();
    for root in roots {
        if (root as usize) < contract.types.len() {
            work.push_back(root);
        }
    }
    while let Some(reference) = work.pop_front() {
        let index = reference as usize;
        if reached[index] {
            continue;
        }
        reached[index] = true;
        for child in type_children(&contract.types[index]) {
            if (child as usize) < contract.types.len() && !reached[child as usize] {
                work.push_back(child);
            }
        }
    }
    for (index, was_reached) in reached.into_iter().enumerate() {
        if !was_reached {
            violation(
                violations,
                "orphan_type_node",
                format!("$.types[{index}]"),
                "every type node must be reachable from actor or declaration roots",
            );
        }
    }
}

fn type_children(node: &TypeNode) -> Vec<TypeRef> {
    match node {
        TypeNode::Primitive { .. } => Vec::new(),
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => vec![*inner],
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            fields.iter().map(|field| field.ty).collect()
        }
        TypeNode::Func { args, results, .. } => args.iter().chain(results).copied().collect(),
        TypeNode::Service { methods } => methods.iter().map(|method| method.function).collect(),
        TypeNode::Class { init, service } => init
            .iter()
            .copied()
            .chain(std::iter::once(*service))
            .collect(),
    }
}

fn type_child_paths(index: usize, node: &TypeNode) -> Vec<(String, TypeRef)> {
    let base = format!("$.types[{index}]");
    match node {
        TypeNode::Primitive { .. } => Vec::new(),
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => {
            vec![(format!("{base}.inner"), *inner)]
        }
        TypeNode::Record { fields } | TypeNode::Variant { fields } => fields
            .iter()
            .enumerate()
            .map(|(field_index, field)| (format!("{base}.fields[{field_index}].type"), field.ty))
            .collect(),
        TypeNode::Func { args, results, .. } => args
            .iter()
            .enumerate()
            .map(|(argument_index, argument)| (format!("{base}.args[{argument_index}]"), *argument))
            .chain(
                results.iter().enumerate().map(|(result_index, result)| {
                    (format!("{base}.results[{result_index}]"), *result)
                }),
            )
            .collect(),
        TypeNode::Service { methods } => methods
            .iter()
            .enumerate()
            .map(|(method_index, method)| {
                (
                    format!("{base}.methods[{method_index}].function"),
                    method.function,
                )
            })
            .collect(),
        TypeNode::Class { init, service } => init
            .iter()
            .enumerate()
            .map(|(argument_index, argument)| (format!("{base}.init[{argument_index}]"), *argument))
            .chain(std::iter::once((format!("{base}.service"), *service)))
            .collect(),
    }
}

fn validate_ref(
    reference: TypeRef,
    path: &str,
    type_count: usize,
    violations: &mut Vec<ContractViolation>,
) {
    if reference as usize >= type_count {
        violation(
            violations,
            "dangling_type_ref",
            path,
            format!("type reference {reference} is outside the arena of {type_count} node(s)"),
        );
    }
}

fn is_sha256_fingerprint(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn violation(
    violations: &mut Vec<ContractViolation>,
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    violations.push(ContractViolation {
        code: code.into(),
        path: path.into(),
        message: message.into(),
    });
}
