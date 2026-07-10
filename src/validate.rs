use crate::canonical;
use crate::limits::Limits;
use crate::model::{
    Actor, Contract, ContractValidationError, ContractViolation, MethodMode, ServiceMethod,
    TypeNode, TypeRef, CANONICALIZATION_PROFILE, CONTRACT_FORMAT, CONTRACT_VERSION, FORMAT_VERSION,
    SEMANTICS_PROFILE,
};
use std::collections::{BTreeSet, VecDeque};

pub(crate) fn validate_contract_with_limits(
    contract: &Contract,
    limits: &Limits,
) -> Result<(), ContractValidationError> {
    validate_structure_with_limits(contract, limits)?;
    let expected = canonical::expected_canonical(contract, limits)?;
    if contract.fingerprint != expected.fingerprint {
        return Err(ContractValidationError::single(
            "fingerprint_mismatch",
            "$.fingerprint",
            format!(
                "expected {}, found {}",
                expected.fingerprint, contract.fingerprint
            ),
        ));
    }
    if contract.identities.contract != expected.identities.contract {
        return Err(ContractValidationError::single(
            "contract_id_mismatch",
            "$.identities.contract",
            format!(
                "expected {}, found {}",
                expected.identities.contract, contract.identities.contract
            ),
        ));
    }
    if contract.identities.interface != expected.identities.interface {
        return Err(ContractValidationError::single(
            "interface_id_mismatch",
            "$.identities.interface",
            format!(
                "expected {:?}, found {:?}",
                expected.identities.interface, contract.identities.interface
            ),
        ));
    }
    Ok(())
}

/// Checks only JSON/graph invariants. Fingerprint verification is intentionally
/// separate so the compiler can canonicalize a newly built graph before it has
/// a fingerprint.
pub(crate) fn validate_structure_with_limits(
    contract: &Contract,
    limits: &Limits,
) -> Result<(), ContractValidationError> {
    if limits.deadline_exceeded() {
        return Err(ContractValidationError::single(
            "operation_deadline_exceeded",
            "$",
            "Contract validation deadline has elapsed",
        ));
    }
    enforce_limits(contract, limits)?;
    let mut violations = Vec::new();
    if contract.format != CONTRACT_FORMAT {
        violation(
            &mut violations,
            "unsupported_contract_format",
            "$.format",
            format!("expected {CONTRACT_FORMAT:?}, found {:?}", contract.format),
        );
    }
    if contract.format_version != FORMAT_VERSION {
        violation(
            &mut violations,
            "unsupported_format_version",
            "$.format_version",
            format!(
                "expected {FORMAT_VERSION}, found {}",
                contract.format_version
            ),
        );
    }
    if contract.semantics_profile != SEMANTICS_PROFILE {
        violation(
            &mut violations,
            "unsupported_semantics_profile",
            "$.semantics_profile",
            format!(
                "expected {SEMANTICS_PROFILE:?}, found {:?}",
                contract.semantics_profile
            ),
        );
    }
    if contract.canonicalization_profile != CANONICALIZATION_PROFILE {
        violation(
            &mut violations,
            "unsupported_canonicalization_profile",
            "$.canonicalization_profile",
            format!(
                "expected {CANONICALIZATION_PROFILE:?}, found {:?}",
                contract.canonicalization_profile
            ),
        );
    }
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
    if !is_content_id(&contract.identities.contract, "ccr:contract:v1") {
        violation(
            &mut violations,
            "invalid_contract_id_format",
            "$.identities.contract",
            "contract identity must use ccr:contract:v1:sha256:<64 lowercase hex>",
        );
    }
    match (&contract.actor, &contract.identities.interface) {
        (None, None) => {}
        (None, Some(_)) => violation(
            &mut violations,
            "actorless_contract_has_interface_id",
            "$.identities.interface",
            "an actorless Contract must not declare an interface identity",
        ),
        (Some(_), Some(interface)) if is_content_id(interface, "ccr:interface:v1") => {}
        (Some(_), Some(_)) => violation(
            &mut violations,
            "invalid_interface_id_format",
            "$.identities.interface",
            "interface identity must use ccr:interface:v1:sha256:<64 lowercase hex>",
        ),
        (Some(_), None) => violation(
            &mut violations,
            "actor_contract_missing_interface_id",
            "$.identities.interface",
            "a Contract with an actor requires an interface identity",
        ),
    }
    if contract.producer.name.is_empty() || contract.producer.version.is_empty() {
        violation(
            &mut violations,
            "invalid_producer",
            "$.producer",
            "producer name and version must not be empty",
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

fn enforce_limits(contract: &Contract, limits: &Limits) -> Result<(), ContractValidationError> {
    if contract.types.len() > limits.max_type_nodes {
        return Err(ContractValidationError::resource_limit(
            "type_nodes",
            limits.max_type_nodes,
            contract.types.len(),
        ));
    }
    if contract.declarations.len() > limits.max_declarations {
        return Err(ContractValidationError::resource_limit(
            "declarations",
            limits.max_declarations,
            contract.declarations.len(),
        ));
    }

    let mut edges = 0usize;
    let mut fields = 0usize;
    let mut methods = 0usize;
    let mut function_values = 0usize;
    let mut string_bytes = contract
        .declarations
        .iter()
        .map(|declaration| declaration.name.len())
        .sum::<usize>();
    for node in &contract.types {
        match node {
            TypeNode::Primitive { .. } => {}
            TypeNode::Opt { .. } | TypeNode::Vec { .. } => edges += 1,
            TypeNode::Record {
                fields: node_fields,
            }
            | TypeNode::Variant {
                fields: node_fields,
            } => {
                fields = fields.saturating_add(node_fields.len());
                edges = edges.saturating_add(node_fields.len());
            }
            TypeNode::Func { args, results, .. } => {
                let count = args.len().saturating_add(results.len());
                function_values = function_values.saturating_add(count);
                edges = edges.saturating_add(count);
            }
            TypeNode::Service {
                methods: node_methods,
            } => {
                methods = methods.saturating_add(node_methods.len());
                edges = edges.saturating_add(node_methods.len());
                string_bytes = string_bytes
                    .saturating_add(node_methods.iter().map(|method| method.name.len()).sum());
            }
            TypeNode::Class { init, .. } => {
                function_values = function_values.saturating_add(init.len());
                edges = edges.saturating_add(init.len().saturating_add(1));
            }
        }
    }
    for (resource, limit, observed) in [
        ("graph_edges", limits.max_graph_edges, edges),
        ("fields", limits.max_fields, fields),
        ("methods", limits.max_methods, methods),
        (
            "function_values",
            limits.max_function_values,
            function_values,
        ),
        ("string_bytes", limits.max_string_bytes, string_bytes),
    ] {
        if observed > limit {
            return Err(ContractValidationError::resource_limit(
                resource, limit, observed,
            ));
        }
    }
    Ok(())
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

fn is_content_id(value: &str, domain: &str) -> bool {
    value
        .strip_prefix(&format!("{domain}:sha256:"))
        .is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
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
        resource_limit: None,
    });
}
