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

/// Follow parsed declaration references across the complete resolved bundle
/// with an explicit stack before the upstream checker can recursively expand
/// a long chain of shallow aliases.
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
    let mut pending: Vec<_> = declarations
        .values()
        .copied()
        .chain(
            programs
                .iter()
                .filter_map(|program| program.actor.as_ref().map(|actor| &actor.typ)),
        )
        .map(|ty| (ty, 0usize, BTreeSet::<&str>::new()))
        .collect();

    while let Some((ty, depth, active_names)) = pending.pop() {
        budget.checkpoint().map_err(|error| {
            budget_error(error, DiagnosticPhase::TypeCheck, "type-depth preflight")
        })?;
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
        let next_depth = depth.saturating_add(1);
        match ty {
            IDLType::VarT(name) => {
                if let Some(resolved) = declarations.get(name.as_str()) {
                    if !active_names.contains(name.as_str()) {
                        let mut next_names = active_names;
                        next_names.insert(name);
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
