//! The cycle rule (issue #218): every cycle in the type graph passes through
//! a declared node.
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
//! The check is a pure graph property, independent of what the module emits
//! or omits: it looks for a cycle among the nodes that are *not* declared
//! (the generator's declared set, `first_names`: every composite node some
//! declaration names), over the same edges the omission analysis uses —
//! `opt`/`vec` inner, record fields and variant arms, `func` arguments then
//! results, `service` methods; a class has none, since it is never rendered
//! as a whole. Primitive nodes have no edges and so lie on no cycle. It is a
//! colour-marking depth-first search with an explicit stack (ADR 0005),
//! linear in nodes plus edges, started from each undeclared node in arena
//! order and following children in edge order, so the node it reports is
//! deterministic: the first node found whose edge closes a cycle, named by
//! the end of that edge.

use std::collections::BTreeMap;

use candid_core::{Contract, TypeRef};

use crate::omissions::children;
use crate::TsGenError;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Not reached yet.
    New,
    /// On the current depth-first path.
    Open,
    /// Fully explored: no cycle through no declaration passes through it.
    Done,
}

/// Refuse a Contract with a cycle that passes through no declared node.
///
/// Every reference must already be in range (the omission analysis checks
/// that first); `declared` is the generator's declared set.
pub(crate) fn check(
    contract: &Contract,
    declared: &BTreeMap<TypeRef, String>,
) -> Result<(), TsGenError> {
    let types = contract.types();
    let edges: Vec<Vec<TypeRef>> = types.iter().map(children).collect();
    let mut marks = vec![Mark::New; types.len()];
    // Each entry is a node on the current path and the index of the next of
    // its edges to follow.
    let mut path: Vec<(usize, usize)> = Vec::new();
    for start in 0..types.len() {
        if marks[start] != Mark::New || is_declared(declared, start) {
            continue;
        }
        marks[start] = Mark::Open;
        path.push((start, 0));
        while let Some((node, next)) = path.last_mut() {
            let Some(&child) = edges[*node].get(*next) else {
                marks[*node] = Mark::Done;
                path.pop();
                continue;
            };
            *next += 1;
            let child_index = child as usize;
            if is_declared(declared, child_index) {
                continue;
            }
            match marks[child_index] {
                Mark::Open => return Err(TsGenError::UndeclaredCycle { reference: child }),
                Mark::Done => {}
                Mark::New => {
                    marks[child_index] = Mark::Open;
                    path.push((child_index, 0));
                }
            }
        }
    }
    Ok(())
}

fn is_declared(declared: &BTreeMap<TypeRef, String>, index: usize) -> bool {
    TypeRef::try_from(index).is_ok_and(|reference| declared.contains_key(&reference))
}
