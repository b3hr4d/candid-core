//! Per-declaration omission (issue #189): which declarations and actor
//! methods a module leaves out, and why.
//!
//! The analysis runs before any text is emitted and decides everything the
//! emitter then skips, so the emitter itself never meets an unrepresentable
//! shape. `schemaFromContract` (`ts/contract.ts`) runs the same analysis,
//! step for step, so the two report the same omissions for the same Contract;
//! a change here without the same change there breaks the loader crosscheck.
//!
//! # The rule
//!
//! A node has a *direct cause* when rendering its own structure would emit
//! text that is wrong or cannot compile: a record (neither empty nor
//! tuple-shaped) or variant with a field or arm name shaped like `_N_`
//! (`reserved_field_name`), or a variant arm whose payload is a declared
//! `opt` of a never-domain type (`ambiguous_variant_arm`). For a variant the
//! arms are checked in order, reserved name before ambiguity per arm.
//!
//! A declaration has a *name cause* when its name cannot be exported:
//! not identifier-shaped (`invalid_declaration_name`), or one of the actor
//! surface's export names `actor` / `Actor` (`reserved_export_name`).
//!
//! A node is *tainted* when it has a direct cause, when it is rendered by the
//! name of a declaration with a name cause (the first declaration of a
//! composite node names it, see `first_names`), or when any node it has an
//! edge to is tainted. Edges are every structural reference — `opt`/`vec`
//! inner, record fields and variant arms, `func` arguments then results,
//! `service` methods — so taint spreads through nested reference types to
//! the containing declaration. Taint is computed as a multi-source
//! breadth-first search over reverse edges, which also gives every tainted
//! node its *distance*: the number of edges to the nearest cause.
//!
//! A declaration is omitted when it has a name cause or its node is tainted;
//! an actor method is omitted when its function node is tainted. A tainted
//! declared node always names an omitted declaration, so an emitted
//! declaration or method can never reference an omitted one.
//!
//! # The reported reason
//!
//! A name cause wins. A declaration that renders another declaration's name
//! (a later alias of a composite node) reports `references_omitted` via that
//! name. Otherwise the reason is the *node reason* of its node (for a method,
//! of its function node, or `references_omitted` via the function's
//! declaration when it has one): the node's direct cause if it has one;
//! otherwise, among its children in edge order, the first whose distance is
//! smaller than the node's — `references_omitted` via its declaration when
//! the child is declared, else that child's own node reason. Distances
//! strictly decrease along the walk, so it always ends, and shortest
//! distances do not depend on traversal order, so both implementations agree.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use candid_core::{Actor, Contract, PrimitiveType, TypeNode, TypeRef};

use crate::{
    is_reserved_numeric_name, is_ts_property_identifier, is_tuple_shaped, Omission, OmissionKind,
    OmissionReason, TsGenError, TsNames, RESERVED_EXPORT_NAMES,
};

/// What the emitter leaves out.
pub(crate) struct Analysis {
    /// Per node: `Some(distance to the nearest cause)` when tainted.
    distance: Vec<Option<u32>>,
    declarations: BTreeSet<String>,
    methods: BTreeSet<String>,
    /// Declarations first, then methods, each group sorted by name.
    pub(crate) omitted: Vec<Omission>,
}

impl Analysis {
    pub(crate) fn omits_declaration(&self, name: &str) -> bool {
        self.declarations.contains(name)
    }

    pub(crate) fn omits_method(&self, name: &str) -> bool {
        self.methods.contains(name)
    }

    pub(crate) fn is_tainted(&self, reference: TypeRef) -> bool {
        self.distance
            .get(reference as usize)
            .is_some_and(Option::is_some)
    }
}

/// The name cause of a declaration, if any.
fn name_cause(name: &str) -> Option<OmissionReason> {
    if !is_ts_property_identifier(name) {
        Some(OmissionReason::InvalidDeclarationName)
    } else if RESERVED_EXPORT_NAMES.contains(&name) {
        Some(OmissionReason::ReservedExportName)
    } else {
        None
    }
}

/// A node's outgoing edges, in edge order. A class has none: it exists only
/// at the actor root, which is never rendered as a whole.
pub(crate) fn children(node: &TypeNode) -> Vec<TypeRef> {
    match node {
        TypeNode::Primitive { .. } | TypeNode::Class { .. } => Vec::new(),
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => vec![*inner],
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            fields.iter().map(|field| field.ty).collect()
        }
        TypeNode::Func { args, results, .. } => args.iter().chain(results).copied().collect(),
        TypeNode::Service { methods } => methods.iter().map(|method| method.function).collect(),
    }
}

pub(crate) fn node(contract: &Contract, reference: TypeRef) -> Result<&TypeNode, TsGenError> {
    contract
        .types()
        .get(reference as usize)
        .ok_or(TsGenError::DanglingTypeRef { reference })
}

/// The node's direct cause, if any (see the module docs).
fn direct_cause(
    contract: &Contract,
    names: &TsNames,
    declared: &BTreeMap<TypeRef, String>,
    reference: TypeRef,
) -> Result<Option<OmissionReason>, TsGenError> {
    let reserved = |id: u32| {
        names
            .get(reference, id)
            .is_some_and(is_reserved_numeric_name)
    };
    match node(contract, reference)? {
        // Empty and tuple-shaped records render no keys. (A `_N_`-shaped
        // name hashes far outside the `0..n-1` ids a tuple has anyway.)
        TypeNode::Record { fields } if !fields.is_empty() && !is_tuple_shaped(fields) => Ok(fields
            .iter()
            .any(|field| reserved(field.id))
            .then_some(OmissionReason::ReservedFieldName)),
        TypeNode::Variant { fields } => {
            for field in fields {
                if reserved(field.id) {
                    return Ok(Some(OmissionReason::ReservedFieldName));
                }
                // A *declared* opt payload renders as a bare reference; when
                // its inner type is never-domain the reference's static type
                // is `Schema<null>`, which the type level must read as a bare
                // tag (issue #127). The anonymous form is not affected.
                if !declared.contains_key(&field.ty) {
                    continue;
                }
                let TypeNode::Opt { inner } = node(contract, field.ty)? else {
                    continue;
                };
                let never = match node(contract, *inner)? {
                    TypeNode::Primitive {
                        primitive: PrimitiveType::Empty,
                    } => true,
                    TypeNode::Variant { fields } => fields.is_empty(),
                    _ => false,
                };
                if never {
                    return Ok(Some(OmissionReason::AmbiguousVariantArm));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// The node an actor's methods live on: the service itself, or a class
/// actor's service.
fn actor_service(contract: &Contract) -> Result<Option<TypeRef>, TsGenError> {
    Ok(match contract.actor() {
        None => None,
        Some(Actor::Service { service }) => Some(*service),
        Some(Actor::Class { class }) => match node(contract, *class)? {
            TypeNode::Class { service, .. } => Some(*service),
            _ => Some(*class),
        },
    })
}

/// Run the analysis. Refuses — the whole Contract, as before issue #189 —
/// only for an invalid graph: a dangling reference, or a declaration naming
/// a class node (candid-core's `class_not_actor_root`, issue #129).
pub(crate) fn analyze(
    contract: &Contract,
    names: &TsNames,
    declared: &BTreeMap<TypeRef, String>,
) -> Result<Analysis, TsGenError> {
    for declaration in contract.declarations() {
        if matches!(node(contract, declaration.ty)?, TypeNode::Class { .. }) {
            return Err(TsGenError::UnsupportedConstruct {
                declaration: declaration.name.clone(),
                kind: "class",
            });
        }
    }

    let types = contract.types();
    let mut edges: Vec<Vec<TypeRef>> = Vec::with_capacity(types.len());
    let mut parents: Vec<Vec<TypeRef>> = vec![Vec::new(); types.len()];
    let mut direct: Vec<Option<OmissionReason>> = Vec::with_capacity(types.len());
    for (index, type_node) in types.iter().enumerate() {
        let reference = TypeRef::try_from(index).expect("arena indices fit a TypeRef");
        let outgoing = children(type_node);
        for &child in &outgoing {
            node(contract, child)?;
            parents[child as usize].push(reference);
        }
        edges.push(outgoing);
        direct.push(direct_cause(contract, names, declared, reference)?);
    }

    // Multi-source breadth-first search from every cause, over reverse edges.
    let mut distance: Vec<Option<u32>> = vec![None; types.len()];
    let mut queue = VecDeque::new();
    for (index, cause) in direct.iter().enumerate() {
        if cause.is_some() {
            distance[index] = Some(0);
            queue.push_back(index);
        }
    }
    for (&reference, first) in declared {
        if name_cause(first).is_some() && distance[reference as usize].is_none() {
            distance[reference as usize] = Some(0);
            queue.push_back(reference as usize);
        }
    }
    while let Some(index) = queue.pop_front() {
        let next = distance[index].expect("queued nodes are tainted") + 1;
        for &parent in &parents[index] {
            if distance[parent as usize].is_none() {
                distance[parent as usize] = Some(next);
                queue.push_back(parent as usize);
            }
        }
    }

    let mut memo: Vec<Option<(OmissionReason, Option<String>)>> = vec![None; types.len()];
    let mut node_reason = |start: TypeRef| -> (OmissionReason, Option<String>) {
        let mut chain = Vec::new();
        let mut current = start as usize;
        let reason = loop {
            if let Some(known) = &memo[current] {
                break known.clone();
            }
            chain.push(current);
            if let Some(cause) = direct[current] {
                break (cause, None);
            }
            let own = distance[current].expect("a node reason is asked of tainted nodes only");
            let closer = edges[current]
                .iter()
                .copied()
                .find(|&child| distance[child as usize].is_some_and(|d| d < own))
                .expect("a tainted node without a direct cause has a closer child");
            if let Some(first) = declared.get(&closer) {
                break (OmissionReason::ReferencesOmitted, Some(first.clone()));
            }
            current = closer as usize;
        };
        for index in chain {
            memo[index] = Some(reason.clone());
        }
        reason
    };

    let mut omitted = Vec::new();
    let mut omitted_declarations = BTreeSet::new();
    for declaration in contract.declarations() {
        let tainted = distance[declaration.ty as usize].is_some();
        let (reason, via) = if let Some(cause) = name_cause(&declaration.name) {
            (cause, None)
        } else if !tainted {
            continue;
        } else {
            match declared.get(&declaration.ty) {
                Some(first) if *first != declaration.name => {
                    (OmissionReason::ReferencesOmitted, Some(first.clone()))
                }
                _ => node_reason(declaration.ty),
            }
        };
        omitted_declarations.insert(declaration.name.clone());
        omitted.push(Omission {
            kind: OmissionKind::Declaration,
            name: declaration.name.clone(),
            reason,
            via,
        });
    }

    let mut omitted_methods = BTreeSet::new();
    if let Some(service) = actor_service(contract)? {
        if let TypeNode::Service { methods } = node(contract, service)? {
            for method in methods {
                if distance[method.function as usize].is_none() {
                    continue;
                }
                let (reason, via) = match declared.get(&method.function) {
                    Some(first) => (OmissionReason::ReferencesOmitted, Some(first.clone())),
                    None => node_reason(method.function),
                };
                omitted_methods.insert(method.name.clone());
                omitted.push(Omission {
                    kind: OmissionKind::Method,
                    name: method.name.clone(),
                    reason,
                    via,
                });
            }
        }
    }
    omitted.sort();

    Ok(Analysis {
        distance,
        declarations: omitted_declarations,
        methods: omitted_methods,
        omitted,
    })
}
