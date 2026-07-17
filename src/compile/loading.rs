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
) -> Result<(Vec<SourceUnit>, crate::SourceId), CompileError> {
    struct Pending {
        source_id: crate::SourceId,
        include_actor: bool,
        depth: usize,
        ancestors: Vec<crate::SourceId>,
    }

    let limits = &context.limits;
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
    let mut accounting = SourceAccounting::default();
    let mut import_edges = 0usize;

    while let Some(request) = pending.pop() {
        check_source_deadline(limits)?;
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
            .load(&source_id, limits)
            .map_err(crate::ResolveError::into_compile_error)?;
        let resolved = accept_resolved_source(&source_id, resolved, limits, &mut accounting)?;
        check_source_nesting(&resolved.source, limits)?;
        let program = parse_program(&resolved.source, Some(resolved.id.as_str().to_string()))?;
        check_programs_type_depth(std::iter::once(&program), limits)?;
        let imports: Vec<_> = program
            .decs
            .iter()
            .filter_map(|declaration| match declaration {
                Dec::ImportType(import) => Some((import.clone(), SourceImportKind::Type)),
                Dec::ImportServ(import) => Some((import.clone(), SourceImportKind::Service)),
                Dec::TypD(_) => None,
            })
            .collect();
        import_edges = import_edges.saturating_add(imports.len());
        if import_edges > limits.max_import_edges {
            return Err(CompileError::resource_limit(
                "import_edges",
                limits.max_import_edges,
                import_edges,
                format!("import edges exceed limit {}", limits.max_import_edges),
            ));
        }
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

#[derive(Default)]
pub(super) struct SourceAccounting {
    sources: usize,
    bundle_bytes: usize,
}

fn check_source_deadline(limits: &crate::Limits) -> Result<(), CompileError> {
    if limits.deadline_exceeded() {
        return Err(CompileError::single(
            "operation_deadline_exceeded",
            DiagnosticPhase::Load,
            "source resolution deadline has elapsed",
        ));
    }
    Ok(())
}

pub(super) fn accept_source(
    id: &str,
    source_bytes: usize,
    limits: &crate::Limits,
    accounting: &mut SourceAccounting,
) -> Result<(), CompileError> {
    check_source_deadline(limits)?;
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
    let sources = accounting.sources.saturating_add(1);
    if sources > limits.max_sources {
        return Err(CompileError::resource_limit(
            "sources",
            limits.max_sources,
            sources,
            format!("source count exceeds limit {}", limits.max_sources),
        ));
    }
    let bundle_bytes = accounting.bundle_bytes.saturating_add(source_bytes);
    if bundle_bytes > limits.max_bundle_bytes {
        return Err(CompileError::resource_limit(
            "bundle_bytes",
            limits.max_bundle_bytes,
            bundle_bytes,
            format!(
                "source bundle uses {bundle_bytes} bytes; limit is {}",
                limits.max_bundle_bytes
            ),
        ));
    }
    accounting.sources = sources;
    accounting.bundle_bytes = bundle_bytes;
    Ok(())
}

fn accept_resolved_source(
    expected_id: &crate::SourceId,
    resolved: crate::ResolvedSource,
    limits: &crate::Limits,
    accounting: &mut SourceAccounting,
) -> Result<crate::ResolvedSource, CompileError> {
    check_source_deadline(limits)?;
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
    accept_source(
        resolved.id.as_str(),
        resolved.source.len(),
        limits,
        accounting,
    )?;
    resolved
        .verify()
        .map_err(crate::ResolveError::into_compile_error)?;
    Ok(resolved)
}
