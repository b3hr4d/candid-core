//! The cycle rule (issue #218): every cycle the module renders passes
//! through a declared node.
//!
//! The renderer expands an undeclared node's structure in place and stops at
//! a declared one, which it writes as the declaration's name. A cycle through
//! no declared node would therefore expand forever, and a TypeScript type
//! alias has no spelling for an anonymous cycle anyway. No Candid source can
//! produce one — a recursive Candid type needs a name, and the compiler keeps
//! it as a declaration — but Contract validation accepts every cycle
//! (`docs/contract-graph.md`), so a Contract read from a document or built
//! from a draft can hold one. [`check`] refuses such a Contract whole, before
//! anything renders, with [`TsGenError::UndeclaredCycle`].
//!
//! The check covers exactly the graph the renderer walks, so every Contract
//! that generated before issue #218 still generates, byte-identically. Its
//! roots are what `Generator::module` renders, in that order: the body of
//! each declaration the module emits (an omitted declaration renders
//! nothing, and a later alias of a declared node renders only the first
//! name), then the actor's service — its surviving methods when something
//! under it is omitted — and each surviving method's arguments and results.
//! A class actor's init args are never rendered (issue #104), and neither is
//! anything reachable only from an omitted declaration or method, so a cycle
//! there does not refuse. From the roots it follows the edges of
//! `omissions::children` — `opt`/`vec` inner, record fields and variant arms
//! (tuples included), `func` arguments then results, `service` methods — and
//! stops at every declared node (the generator's declared set,
//! `first_names`: every composite node some declaration names), as the
//! renderer does. A root's own structure is walked even when it is declared,
//! as a declaration's right-hand side is.
//!
//! It is a colour-marking depth-first search with an explicit stack (ADR
//! 0005), linear in nodes plus edges: a node fully explored from one root is
//! not explored again from the next. Roots are taken in render order and
//! children in edge order, so the node it reports is deterministic: the
//! first node found whose incoming edge closes a cycle, named by the end of
//! that edge.

use std::collections::BTreeMap;

use candid_core::{Contract, TypeNode, TypeRef};

use crate::omissions::{children, node, Analysis};
use crate::{actor_service, TsGenError};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Not reached yet.
    New,
    /// On the current depth-first path.
    Open,
    /// Fully explored: no cycle through no declaration passes through it.
    Done,
}

/// Where the renderer starts a walk.
#[derive(Clone, Copy)]
enum Root {
    /// Rendered by name: a declared node stops the walk at once.
    ByName(TypeRef),
    /// The node's own structure is rendered even when it is declared.
    Structure(TypeRef),
}

/// Refuse a Contract with a cycle the renderer would walk that passes
/// through no declared node.
///
/// Every reference must already be in range (the omission analysis checks
/// that first); `declared` is the generator's declared set and `analysis`
/// the omissions the module makes.
pub(crate) fn check(
    contract: &Contract,
    declared: &BTreeMap<TypeRef, String>,
    analysis: &Analysis,
) -> Result<(), TsGenError> {
    let types = contract.types();
    let mut search = Search {
        edges: types.iter().map(children).collect(),
        marks: vec![Mark::New; types.len()],
        declared,
    };
    for root in roots(contract, declared, analysis)? {
        match root {
            Root::ByName(reference) => search.from(reference)?,
            Root::Structure(reference) if !search.is_declared(reference) => {
                search.from(reference)?;
            }
            Root::Structure(reference) => {
                for index in 0..search.edges[reference as usize].len() {
                    search.from(search.edges[reference as usize][index])?;
                }
            }
        }
    }
    Ok(())
}

/// The walks `Generator::module` starts, in its order.
fn roots(
    contract: &Contract,
    declared: &BTreeMap<TypeRef, String>,
    analysis: &Analysis,
) -> Result<Vec<Root>, TsGenError> {
    // `Generator::declaration_body`: a node declared under another first
    // name renders as that name, and walks nothing.
    let body = |reference: TypeRef, own_name: &str| match declared.get(&reference) {
        Some(first) if first != own_name => None,
        _ => Some(Root::Structure(reference)),
    };
    let mut roots = Vec::new();
    for declaration in contract.declarations() {
        if !analysis.omits_declaration(&declaration.name) {
            roots.extend(body(declaration.ty, &declaration.name));
        }
    }
    if let Some((service, _)) = actor_service(contract)? {
        let methods: Vec<TypeRef> = match node(contract, service)? {
            TypeNode::Service { methods } => methods
                .iter()
                .filter(|method| !analysis.omits_method(&method.name))
                .map(|method| method.function)
                .collect(),
            _ => Vec::new(),
        };
        if analysis.is_tainted(service) {
            roots.extend(methods.iter().map(|&function| Root::ByName(function)));
        } else {
            roots.extend(body(service, "actor"));
        }
        // The `Actor` signatures render each argument and result by name.
        roots.extend(methods.iter().map(|&function| Root::Structure(function)));
    }
    Ok(roots)
}

struct Search<'a> {
    edges: Vec<Vec<TypeRef>>,
    marks: Vec<Mark>,
    declared: &'a BTreeMap<TypeRef, String>,
}

impl Search<'_> {
    fn is_declared(&self, reference: TypeRef) -> bool {
        self.declared.contains_key(&reference)
    }

    /// Search from `start` as the renderer renders it by name.
    fn from(&mut self, start: TypeRef) -> Result<(), TsGenError> {
        let start = start as usize;
        if self.marks[start] != Mark::New || self.is_declared(start as TypeRef) {
            return Ok(());
        }
        self.marks[start] = Mark::Open;
        // Each entry is a node on the current path and the index of the next
        // of its edges to follow.
        let mut path: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some((node, next)) = path.last_mut() {
            let Some(&child) = self.edges[*node].get(*next) else {
                self.marks[*node] = Mark::Done;
                path.pop();
                continue;
            };
            *next += 1;
            if self.is_declared(child) {
                continue;
            }
            let child_index = child as usize;
            match self.marks[child_index] {
                Mark::Open => return Err(TsGenError::UndeclaredCycle { reference: child }),
                Mark::Done => {}
                Mark::New => {
                    self.marks[child_index] = Mark::Open;
                    path.push((child_index, 0));
                }
            }
        }
        Ok(())
    }
}
