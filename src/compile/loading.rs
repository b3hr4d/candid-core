use super::*;

pub(super) struct SourceUnit {
    pub(super) name: String,
    pub(super) source: String,
    pub(super) program: IDLProg,
    pub(super) imports: Vec<ResolvedImport>,
    pub(super) include_actor: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedImport {
    pub(super) import: String,
    pub(super) target: crate::SourceId,
    pub(super) kind: SourceImportKind,
}

pub(super) fn load_source_units_with_resolver(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    context: &RuntimeContext,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(Vec<SourceUnit>, crate::SourceId), CompileError> {
    struct Pending {
        source_id: crate::SourceId,
        include_actor: bool,
        depth: usize,
        ancestors: Vec<crate::SourceId>,
    }

    let limits = budget.limits().clone();
    let mut units = Vec::<SourceUnit>::new();
    let mut indexes = BTreeMap::<crate::SourceId, usize>::new();
    let entry_id = resolver
        .identify(None, entry)
        .map_err(crate::ResolveError::into_compile_error)
        .and_then(validate_resolver_id)?;
    let mut pending = vec![Pending {
        source_id: entry_id.clone(),
        include_actor: true,
        depth: 0,
        ancestors: Vec::new(),
    }];

    while let Some(request) = pending.pop() {
        budget
            .checkpoint()
            .map_err(|error| budget_error(error, DiagnosticPhase::Load, "source resolution"))?;
        if request.depth > limits.max_import_depth {
            return Err(CompileError::resource_limit(
                "import_depth",
                limits.max_import_depth,
                request.depth,
                format!(
                    "import depth {} exceeds limit {}",
                    request.depth, limits.max_import_depth
                ),
            ));
        }
        let source_id = request.source_id;
        if request.ancestors.contains(&source_id) {
            return Err(CompileError::single(
                "did_import_cycle",
                DiagnosticPhase::Load,
                format!("import cycle reached {:?}", source_id.as_str()),
            ));
        }
        if let Some(index) = indexes.get(&source_id).copied() {
            units[index].include_actor |= request.include_actor;
            continue;
        }
        let resolved = resolver
            .load_with_context(&source_id, context)
            .map_err(crate::ResolveError::into_compile_error)?;
        budget
            .checkpoint()
            .map_err(|error| budget_error(error, DiagnosticPhase::Load, "source loading"))?;
        let resolved = accept_resolved_source(&source_id, resolved, budget)?;
        check_source_nesting(&resolved.source, budget)?;
        let program = parse_program(
            &resolved.source,
            Some(resolved.id.as_str().to_string()),
            budget,
        )?;
        check_programs_type_depth(std::iter::once(&program), budget)?;
        let imports: Vec<_> = program
            .decs
            .iter()
            .filter_map(|declaration| match declaration {
                Dec::ImportType(import) => Some((import.clone(), SourceImportKind::Type)),
                Dec::ImportServ(import) => Some((import.clone(), SourceImportKind::Service)),
                Dec::TypD(_) => None,
            })
            .collect();
        budget
            .charge("import_edges", limits.max_import_edges, imports.len())
            .map_err(|error| budget_error(error, DiagnosticPhase::Load, "import resolution"))?;
        let resolved_imports = imports
            .into_iter()
            .map(|(import, kind)| {
                let target = resolver
                    .identify(Some(&resolved.id), &import)
                    .map_err(crate::ResolveError::into_compile_error)
                    .and_then(validate_resolver_id)?;
                Ok(ResolvedImport {
                    import,
                    target,
                    kind,
                })
            })
            .collect::<Result<Vec<_>, CompileError>>()?;
        let index = units.len();
        indexes.insert(source_id.clone(), index);
        units.push(SourceUnit {
            name: resolved.id.as_str().to_string(),
            source: resolved.source,
            program,
            imports: resolved_imports.clone(),
            include_actor: request.include_actor,
        });
        let mut ancestors = request.ancestors;
        ancestors.push(resolved.id.clone());
        for import in resolved_imports.into_iter().rev() {
            pending.push(Pending {
                source_id: import.target,
                include_actor: import.kind == SourceImportKind::Service,
                depth: request.depth + 1,
                ancestors: ancestors.clone(),
            });
        }
    }
    Ok((units, entry_id))
}

fn validate_resolver_id(id: crate::SourceId) -> Result<crate::SourceId, CompileError> {
    let normalized =
        crate::SourceId::parse(id.as_str()).map_err(crate::ResolveError::into_compile_error)?;
    if normalized != id {
        return Err(CompileError::single(
            "did_invalid_source_id",
            DiagnosticPhase::Load,
            format!(
                "resolver returned non-canonical source ID {:?}",
                id.as_str()
            ),
        ));
    }
    Ok(normalized)
}

pub(super) fn accept_source(
    id: &str,
    source_bytes: usize,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(), CompileError> {
    let limits = budget.limits().clone();
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::Load, "source accounting"))?;
    if source_bytes > limits.max_source_bytes {
        return Err(CompileError::resource_limit(
            "source_bytes",
            limits.max_source_bytes,
            source_bytes,
            format!(
                "source {id:?} uses {source_bytes} bytes; limit is {}",
                limits.max_source_bytes
            ),
        ));
    }
    // Bound the logical source ID, after the pre-existing content-byte check so
    // its precedence is unchanged, and before charging so an oversized path is
    // rejected without consuming the source/bundle budget.
    if id.len() > limits.max_source_id_bytes {
        return Err(CompileError::resource_limit(
            "source_id_bytes",
            limits.max_source_id_bytes,
            id.len(),
            format!(
                "source ID {id:?} uses {} bytes; limit is {}",
                id.len(),
                limits.max_source_id_bytes
            ),
        ));
    }
    budget
        .charge("sources", limits.max_sources, 1)
        .map_err(|error| budget_error(error, DiagnosticPhase::Load, "source accounting"))?;
    budget
        .charge("bundle_bytes", limits.max_bundle_bytes, source_bytes)
        .map_err(|error| budget_error(error, DiagnosticPhase::Load, "source accounting"))?;
    Ok(())
}

fn accept_resolved_source(
    expected_id: &crate::SourceId,
    resolved: crate::ResolvedSource,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<crate::ResolvedSource, CompileError> {
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::Load, "source loading"))?;
    let resolved_id = validate_resolver_id(resolved.id.clone())?;
    if resolved_id != *expected_id {
        return Err(CompileError::single(
            "did_resolver_identity_mismatch",
            DiagnosticPhase::Load,
            format!(
                "resolver identified {:?} but loaded {:?}",
                expected_id.as_str(),
                resolved.id.as_str()
            ),
        ));
    }
    accept_source(resolved.id.as_str(), resolved.source.len(), budget)?;
    resolved
        .verify()
        .map_err(crate::ResolveError::into_compile_error)?;
    Ok(resolved)
}
