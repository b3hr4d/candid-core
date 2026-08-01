use super::*;

/// Reject stack-hostile syntax before any recursive upstream parser or checker
/// sees it. The token stream skips strings and comments, so their contents do
/// not affect the operational nesting budget.
pub(super) fn check_source_nesting(
    source: &str,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(), CompileError> {
    let limits = budget.limits().clone();
    let mut delimiters = 0usize;
    let mut unary = 0usize;
    for token in Tokenizer::new(source) {
        budget
            .checkpoint()
            .map_err(|error| budget_error(error, DiagnosticPhase::Parse, "source preflight"))?;
        let (_, token, _) = match token {
            Ok(token) => token,
            // Preserve the parser's established lexical diagnostic.
            Err(_) => return Ok(()),
        };
        match token {
            Token::Opt | Token::Vec => unary = unary.saturating_add(1),
            Token::LParen | Token::LBrace => {
                delimiters = delimiters.saturating_add(1);
                unary = 0;
            }
            Token::RParen | Token::RBrace => {
                delimiters = delimiters.saturating_sub(1);
                unary = 0;
            }
            _ => unary = 0,
        }
        let observed = delimiters.saturating_add(unary);
        if observed > limits.max_source_nesting {
            return Err(CompileError::resource_limit(
                "source_nesting",
                limits.max_source_nesting,
                observed,
                format!(
                    "Candid source nesting {observed} exceeds limit {}",
                    limits.max_source_nesting
                ),
            ));
        }
    }
    Ok(())
}

/// Charge one traversal step of the `max_type_depth` guard walks against the
/// shared `type_preflight_work` counter, so both depth guards fail closed and
/// stay interruptible on a budget with no deadline configured (issue #125).
pub(super) fn charge_type_preflight(
    budget: &mut crate::budget::Budget<'_>,
    limit: usize,
    amount: usize,
    phase: DiagnosticPhase,
    operation: &str,
) -> Result<(), CompileError> {
    budget
        .charge("type_preflight_work", limit, amount)
        .map(|_| ())
        .map_err(|error| budget_error(error, phase, operation))
}

/// The recursive declarations active on one expansion path, as interned
/// indices.
///
/// Shared rather than copied. A record or variant pushes one child state per
/// field and every one of them inherits the parent's set unchanged, so
/// copying would allocate `fields * set` entries in a single loop iteration —
/// *before* any of those children is popped and charged. That burst is
/// attacker-controlled on both factors (field count and cycle length) and the
/// parent's own charge does not cover it, which is the whole failure mode
/// `type_preflight_work` exists to prevent. Sharing means a child costs a
/// reference-count bump, so the memory a state can commit before its charge
/// is a constant.
///
/// The set is rebuilt only by [`ActiveNames::with`], on the one transition
/// that changes it — entering a recursive name — and that copy is bounded by
/// the charge already paid for the state making it.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ActiveNames(std::rc::Rc<BTreeSet<usize>>);

impl ActiveNames {
    pub(super) fn contains(&self, index: &usize) -> bool {
        self.0.contains(index)
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// This set plus `index`, as a new set. The only allocating operation.
    pub(super) fn with(&self, index: usize) -> Self {
        let mut next = (*self.0).clone();
        next.insert(index);
        Self(std::rc::Rc::new(next))
    }
}

/// Nodes of a directed graph that lie on at least one cycle: members of a
/// strongly connected component of size two or more, plus self-loops.
///
/// Iterative Kosaraju, because this runs on untrusted declaration graphs and
/// must not recurse. Nodes are dense indices `0..node_count`; both DFS passes
/// use explicit stacks and cost `O(node_count + edges.len())`, which the
/// caller has already charged by metering the walk that produced `edges`.
pub(super) fn cyclic_nodes(node_count: usize, edges: &[(usize, usize)]) -> Vec<bool> {
    let mut forward = vec![Vec::new(); node_count];
    let mut reverse = vec![Vec::new(); node_count];
    let mut cyclic = vec![false; node_count];
    for &(from, to) in edges {
        if from == to {
            cyclic[from] = true;
        } else {
            forward[from].push(to);
            reverse[to].push(from);
        }
    }

    let mut visited = vec![false; node_count];
    let mut order = Vec::with_capacity(node_count);
    for root in 0..node_count {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut stack = vec![(root, 0usize)];
        while let Some(&(node, next)) = stack.last() {
            if let Some(&child) = forward[node].get(next) {
                stack.last_mut().expect("frame just read").1 += 1;
                if !visited[child] {
                    visited[child] = true;
                    stack.push((child, 0));
                }
            } else {
                order.push(node);
                stack.pop();
            }
        }
    }

    let mut component = vec![usize::MAX; node_count];
    let mut component_size = Vec::new();
    for &root in order.iter().rev() {
        if component[root] != usize::MAX {
            continue;
        }
        let id = component_size.len();
        component_size.push(0usize);
        component[root] = id;
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            component_size[id] += 1;
            for &previous in &reverse[node] {
                if component[previous] == usize::MAX {
                    component[previous] = id;
                    stack.push(previous);
                }
            }
        }
    }

    for node in 0..node_count {
        if component_size[component[node]] >= 2 {
            cyclic[node] = true;
        }
    }
    cyclic
}

/// Declarations that participate in a reference cycle, each mapped to a dense
/// index.
///
/// A name outside every cycle can never repeat on one expansion path, so the
/// depth walk need not track it in its per-path active set. That keeps active
/// sets to the (small, usually empty) recursive core, which is what lets
/// identical expansion states deduplicate instead of re-expanding a shared
/// subtree once per incoming edge.
///
/// The walk tracks the *index*, never the name. `type_preflight_work` charges
/// one unit per tracked name, so every operation that unit pays for has to
/// cost the same whatever the name is: comparing indices is a machine-word
/// compare, while comparing identifiers walks their bytes, and a Candid
/// identifier is bounded on this path only by `max_source_bytes`. Set
/// membership and the memo's ordering comparisons are both on this key, so
/// interning is what keeps the advertised bound covering the real cost rather
/// than a fixed multiple of an attacker-chosen length.
fn recursive_declaration_names<'a>(
    declarations: &BTreeMap<&'a str, &'a IDLType>,
    max_work: usize,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<BTreeMap<&'a str, usize>, CompileError> {
    let names: Vec<&str> = declarations.keys().copied().collect();
    let index_of: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (*name, index))
        .collect();
    let mut edges = Vec::new();
    for (name, ty) in declarations {
        let from = index_of[name];
        let mut stack: Vec<&IDLType> = vec![ty];
        while let Some(ty) = stack.pop() {
            charge_type_preflight(
                budget,
                max_work,
                1,
                DiagnosticPhase::TypeCheck,
                "type-depth preflight",
            )?;
            match ty {
                IDLType::VarT(target) => {
                    if let Some(&to) = index_of.get(target.as_str()) {
                        edges.push((from, to));
                    }
                }
                IDLType::OptT(inner) | IDLType::VecT(inner) => stack.push(inner),
                IDLType::RecordT(fields) | IDLType::VariantT(fields) => {
                    stack.extend(fields.iter().map(|field| &field.typ));
                }
                IDLType::FuncT(function) => {
                    stack.extend(function.args.iter().chain(&function.rets).map(|ty| &ty.typ));
                }
                IDLType::ServT(methods) => {
                    stack.extend(methods.iter().map(|method| &method.typ));
                }
                IDLType::ClassT(init, service) => {
                    stack.push(service);
                    stack.extend(init.iter().map(|ty| &ty.typ));
                }
                IDLType::PrimT(_) | IDLType::PrincipalT => {}
            }
        }
    }
    let cyclic = cyclic_nodes(names.len(), &edges);
    Ok(names
        .into_iter()
        .zip(&cyclic)
        .filter(|(_, &in_cycle)| in_cycle)
        .enumerate()
        .map(|(index, (name, _))| (name, index))
        .collect())
}

/// Follow parsed declaration references across the complete resolved bundle
/// with an explicit stack before the upstream checker can recursively expand
/// a long chain of shallow aliases.
///
/// The walk is exact — the same states are refused at the same depths as a
/// full path-sensitive expansion — but shared subtrees are visited once per
/// distinct `(node, depth, active recursive names)` state rather than once
/// per path, and every state is charged against `type_preflight_work`, so a
/// record DAG with shared aliases costs linear work instead of O(2^n) and an
/// input that defeats deduplication fails closed instead of hanging
/// (issue #125).
pub(super) fn check_programs_type_depth<'a>(
    programs: impl IntoIterator<Item = &'a IDLProg>,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(), CompileError> {
    let limits = budget.limits().clone();
    let programs: Vec<_> = programs.into_iter().collect();
    let mut declarations = BTreeMap::new();
    for program in &programs {
        for declaration in &program.decs {
            if let Dec::TypD(binding) = declaration {
                declarations
                    .entry(binding.id.as_str())
                    .or_insert(&binding.typ);
            }
        }
    }
    let max_work = limits.max_type_preflight_work;
    let recursive = recursive_declaration_names(&declarations, max_work, budget)?;
    let mut pending: Vec<_> = declarations
        .values()
        .copied()
        .chain(
            programs
                .iter()
                .filter_map(|program| program.actor.as_ref().map(|actor| &actor.typ)),
        )
        .map(|ty| (ty, 0usize, ActiveNames::default()))
        .collect();

    let mut visited = BTreeSet::new();
    while let Some((ty, depth, active_names)) = pending.pop() {
        // One unit per state plus one per tracked recursive name: cloning and
        // comparing the path set below is what the extra units pay for.
        charge_type_preflight(
            budget,
            max_work,
            1usize.saturating_add(active_names.len()),
            DiagnosticPhase::TypeCheck,
            "type-depth preflight",
        )?;
        if depth > limits.max_type_depth {
            return Err(CompileError::resource_limit(
                "type_depth",
                limits.max_type_depth,
                depth,
                format!(
                    "Candid type depth {depth} exceeds limit {}",
                    limits.max_type_depth
                ),
            ));
        }
        if !visited.insert((ty as *const IDLType as usize, depth, active_names.clone())) {
            continue;
        }
        let next_depth = depth.saturating_add(1);
        match ty {
            IDLType::VarT(name) => {
                if let Some(resolved) = declarations.get(name.as_str()).copied() {
                    let recursive_index = recursive.get(name.as_str()).copied();
                    if !recursive_index.is_some_and(|index| active_names.contains(&index)) {
                        let next_names = match recursive_index {
                            // The one place the set actually changes, so the
                            // one place it is rebuilt. That copy is O(set),
                            // and this state's own charge already covered it.
                            Some(index) => active_names.with(index),
                            // A name outside every cycle cannot repeat on
                            // this path, so tracking it would only split
                            // otherwise-identical states.
                            None => active_names,
                        };
                        pending.push((resolved, depth, next_names));
                    }
                }
            }
            IDLType::OptT(inner) | IDLType::VecT(inner) => {
                pending.push((inner, next_depth, active_names));
            }
            IDLType::RecordT(fields) | IDLType::VariantT(fields) => {
                for field in fields {
                    pending.push((&field.typ, next_depth, active_names.clone()));
                }
            }
            IDLType::FuncT(function) => {
                for ty in function.args.iter().chain(&function.rets) {
                    pending.push((&ty.typ, next_depth, active_names.clone()));
                }
            }
            IDLType::ServT(methods) => {
                for method in methods {
                    pending.push((&method.typ, next_depth, active_names.clone()));
                }
            }
            IDLType::ClassT(init, service) => {
                pending.push((service, next_depth, active_names.clone()));
                for ty in init {
                    pending.push((&ty.typ, next_depth, active_names.clone()));
                }
            }
            IDLType::PrimT(_) | IDLType::PrincipalT => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use crate::budget::Budget;
    use crate::Limits;

    fn preflight_work(source: &str) -> usize {
        let limits = Limits::default();
        let mut budget = Budget::from_limits(&limits);
        let program: IDLProg = source.parse().unwrap();
        check_programs_type_depth(std::iter::once(&program), &mut budget).unwrap();
        budget.consumed("type_preflight_work")
    }

    fn shared_fanout(levels: usize) -> String {
        let mut source = String::new();
        for index in 1..=levels {
            source.push_str(&format!(
                "type T{index} = record {{ a: T{}; b: T{} }};\n",
                index + 1,
                index + 1
            ));
        }
        source.push_str(&format!("type T{} = nat;\n", levels + 1));
        source.push_str("service : { go: (T1) -> () };");
        source
    }

    #[test]
    fn cyclic_nodes_marks_self_loops_and_cycle_members_only() {
        // A chain is acyclic.
        assert_eq!(cyclic_nodes(3, &[(0, 1), (1, 2)]), [false, false, false]);
        // A self-loop is a cycle of one.
        assert_eq!(cyclic_nodes(2, &[(0, 0), (0, 1)]), [true, false]);
        // A two-cycle with a tail: the tail stays acyclic.
        assert_eq!(
            cyclic_nodes(3, &[(0, 1), (1, 0), (1, 2)]),
            [true, true, false]
        );
        // Diamond with a back edge: 0→1→3, 0→2→3, 3→0. Every node lies on a
        // cycle — including 2, which naive back-edge marking of the DFS stack
        // misses because 2→3 is a cross edge to a finished node.
        assert_eq!(
            cyclic_nodes(4, &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 0)]),
            [true, true, true, true]
        );
        // Duplicate edges do not fabricate a cycle.
        assert_eq!(cyclic_nodes(2, &[(0, 1), (0, 1)]), [false, false]);
    }

    /// Pins the charge model on a minimal program: one unit per
    /// recursion-map node (the `nat` body), then one per expansion state —
    /// the actor service, its method's func, the argument `VarT`, the
    /// resolved `nat`, and the declaration walked as its own root.
    #[test]
    fn minimal_program_charges_exactly_six_units() {
        assert_eq!(
            preflight_work("type T = nat; service : { f: (T) -> () };"),
            6
        );
    }

    /// Only names on a cycle are tracked per path, at one extra unit per
    /// state inside their expansion; sibling references to the recursive
    /// name still deduplicate.
    #[test]
    fn recursive_names_cost_one_extra_unit_per_state_inside_their_expansion() {
        assert_eq!(
            preflight_work("type L = opt L; type M = record { a: L; b: L }; service : {};"),
            19
        );
    }

    /// The issue #125 regression at the unit level: 2^24 states would dwarf
    /// any of these numbers.
    ///
    /// What remains grows about quadratically in the number of nested record
    /// levels, because each level adds a depth at which the shared tail is a
    /// distinct state. That factor is bounded by `max_type_depth`, since a
    /// level that does not add depth cannot add a depth to be distinct at —
    /// which is why a pure alias chain, below, is flatly linear.
    #[test]
    fn shared_subtrees_deduplicate_instead_of_doubling_per_level() {
        assert_eq!(preflight_work(&shared_fanout(6)), 138);
        assert_eq!(preflight_work(&shared_fanout(12)), 414);
        assert_eq!(preflight_work(&shared_fanout(24)), 1_398);
    }

    /// Aliases add no depth, so every reference to the chain's tail is the
    /// same `(node, depth, active names)` state and each declaration root
    /// contributes a constant: exactly four units per declaration, with no
    /// dependence on how many roots re-enter the chain. This is the linear
    /// bound issue #125 asks the guard to restore — the un-deduplicated walk
    /// re-walked the whole tail once per root.
    #[test]
    fn alias_chains_cost_a_constant_per_declaration() {
        let alias_chain = |length: usize| {
            let mut source = String::new();
            for index in 1..length {
                source.push_str(&format!("type A{index} = A{};\n", index + 1));
            }
            source.push_str(&format!("type A{length} = nat;\n"));
            source.push_str("service : { f: (A1) -> () };");
            source
        };
        assert_eq!(preflight_work(&alias_chain(100)), 402);
        assert_eq!(preflight_work(&alias_chain(500)), 2_002);
        assert_eq!(preflight_work(&alias_chain(1_000)), 4_002);
        assert_eq!(preflight_work(&alias_chain(2_000)), 8_002);
    }
}
