use crate::canonical;
use crate::diagnostics::{CompileError, Diagnostic, DiagnosticPhase, Severity, SourceSpan};
use crate::limits::RuntimeContext;
use crate::model::{
    Actor, Contract, Declaration, Field, FieldLabelProvenance, MethodMode, PrimitiveType,
    RawContract, RawSourceInfo as SerializedSourceInfo, ServiceMethod, SourceActorInfo,
    SourceDeclaration, SourceFileInfo, SourceFunctionArgumentDirection, SourceFunctionArgumentInfo,
    SourceImportInfo, SourceImportKind, SourceInfo, SourceLabel, SourceMethodInfo, SourceOrigin,
    TypeNode, TypeRef, SOURCE_INFO_VERSION,
};
use candid_parser::candid::types::{FuncMode, Label, Type, TypeEnv, TypeInner};
use candid_parser::syntax::{pretty_print, Dec, IDLMergedProg, IDLProg, IDLType};
use candid_parser::token::{Token, Tokenizer};
use candid_parser::typing::ast_to_type;
use candid_parser::{check_file, check_prog};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileOptions {
    /// Preserve optional names, comments, raw source, and label spelling in a
    /// sidecar. This never changes the Contract or its identities.
    pub include_source_info: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            include_source_info: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation {
    contract: Contract,
    source_info: Option<SourceInfo>,
}

impl Compilation {
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn source_info(&self) -> Option<&SourceInfo> {
        self.source_info.as_ref()
    }

    pub fn into_parts(self) -> (Contract, Option<SourceInfo>) {
        (self.contract, self.source_info)
    }

    pub fn try_from_raw(
        raw_contract: RawContract,
        raw_source_info: Option<SerializedSourceInfo>,
        limits: &crate::Limits,
    ) -> Result<Self, crate::ContractValidationError> {
        let (contract, mapping) = Contract::from_raw_with_mapping(raw_contract, limits)?;
        let source_info = raw_source_info
            .map(SourceInfo::from)
            .map(|mut source_info| {
                remap_source_info(&mut source_info, &mapping)?;
                source_info.validate(&contract, limits)?;
                Ok::<SourceInfo, crate::ContractValidationError>(source_info)
            })
            .transpose()?;
        Ok(Self {
            contract,
            source_info,
        })
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct CompilationRef<'a> {
    contract: &'a Contract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_info: &'a Option<SourceInfo>,
}

impl Serialize for Compilation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        CompilationRef {
            contract: &self.contract,
            source_info: &self.source_info,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompilation {
    contract: RawContract,
    #[serde(default)]
    source_info: Option<SerializedSourceInfo>,
}

impl<'de> Deserialize<'de> for Compilation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawCompilation::deserialize(deserializer)?;
        let limits = crate::Limits::default();
        Self::try_from_raw(raw.contract, raw.source_info, &limits).map_err(D::Error::custom)
    }
}

impl TryFrom<(RawContract, Option<SerializedSourceInfo>)> for Compilation {
    type Error = crate::ContractValidationError;

    fn try_from(
        (contract, source_info): (RawContract, Option<SerializedSourceInfo>),
    ) -> Result<Self, Self::Error> {
        Self::try_from_raw(contract, source_info, &crate::Limits::default())
    }
}

fn remap_source_info(
    source_info: &mut SourceInfo,
    mapping: &[TypeRef],
) -> Result<(), crate::ContractValidationError> {
    let map = |reference: TypeRef| {
        mapping.get(reference as usize).copied().ok_or_else(|| {
            crate::ContractValidationError::single(
                "source_type_ref_out_of_bounds",
                "$",
                format!("source sidecar type reference {reference} is outside the input arena"),
            )
        })
    };
    for declaration in &mut source_info.declarations {
        declaration.ty = map(declaration.ty)?;
    }
    for field in &mut source_info.field_labels {
        field.container = map(field.container)?;
    }
    for method in &mut source_info.methods {
        method.service = map(method.service)?;
    }
    for argument in &mut source_info.function_arguments {
        argument.function = map(argument.function)?;
    }
    Ok(())
}

struct SourceUnit {
    name: String,
    source: String,
    program: IDLProg,
    imports: Vec<ResolvedImport>,
    include_actor: bool,
}

#[derive(Debug, Clone)]
struct ResolvedImport {
    import: String,
    target: crate::SourceId,
    kind: SourceImportKind,
}

/// Compile a self-contained DID source string. DID imports require a file path
/// because import resolution is owned by the official Candid semantic engine.
pub fn compile_did(source: &str) -> Result<Compilation, CompileError> {
    compile_did_with_options(source, CompileOptions::default())
}

pub fn compile_did_with_options(
    source: &str,
    options: CompileOptions,
) -> Result<Compilation, CompileError> {
    compile_did_with_context(source, options, &RuntimeContext::default())
}

pub fn compile_did_with_context(
    source: &str,
    options: CompileOptions,
    context: &RuntimeContext,
) -> Result<Compilation, CompileError> {
    let mut accounting = SourceAccounting::default();
    accept_source(
        "memory:/inline.did",
        source.len(),
        &context.limits,
        &mut accounting,
    )?;
    check_source_nesting(source, &context.limits)?;
    let program = parse_program(source, Some("memory:/inline.did".to_string()))?;
    check_program_type_depth(&program, &context.limits)?;
    let imports: Vec<_> = program
        .decs
        .iter()
        .filter_map(|declaration| match declaration {
            Dec::ImportType(path) | Dec::ImportServ(path) => Some(path.clone()),
            Dec::TypD(_) => None,
        })
        .collect();
    if !imports.is_empty() {
        let mut error = CompileError::single(
            "did_import_requires_file",
            DiagnosticPhase::Load,
            "DID source contains imports; compile it with compile_did_file so candid_parser can resolve them",
        );
        error.diagnostics[0].notes = imports
            .into_iter()
            .map(|path| format!("import: {path}"))
            .collect();
        return Err(error);
    }

    let mut environment = TypeEnv::new();
    let actor = check_prog(&mut environment, &program)
        .map_err(|error| candid_error(error, DiagnosticPhase::TypeCheck, None))?;
    let source_units = vec![SourceUnit {
        name: "memory:/inline.did".to_string(),
        source: source.to_string(),
        program,
        imports: Vec::new(),
        include_actor: true,
    }];
    lower_checked(
        &source_units,
        &environment,
        actor.as_ref(),
        options,
        context,
    )
}

/// Compile a DID file through `candid_parser::check_file`, including its
/// official filesystem import-resolution path.
pub fn compile_did_file(path: impl AsRef<Path>) -> Result<Compilation, CompileError> {
    compile_did_file_with_options(path, CompileOptions::default())
}

pub fn compile_did_file_with_options(
    path: impl AsRef<Path>,
    options: CompileOptions,
) -> Result<Compilation, CompileError> {
    compile_did_file_with_context(path, options, &RuntimeContext::default())
}

pub fn compile_did_file_with_context(
    path: impl AsRef<Path>,
    options: CompileOptions,
    context: &RuntimeContext,
) -> Result<Compilation, CompileError> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CompileError::single(
                "did_invalid_source_id",
                DiagnosticPhase::Load,
                format!("{} has no UTF-8 file name", path.display()),
            )
        })?;
    let resolver =
        crate::WorkspaceResolver::new(parent).map_err(crate::ResolveError::into_compile_error)?;
    compile_with_resolver(entry, &resolver, options, context)
}

pub fn compile_with_resolver(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    options: CompileOptions,
    context: &RuntimeContext,
) -> Result<Compilation, CompileError> {
    let (source_units, entry_id) = load_source_units_with_resolver(entry, resolver, context)?;
    let materialized = MaterializedBundle::new(&source_units, &entry_id)?;
    let (environment, actor, _) = check_file(&materialized.entry).map_err(candid_file_error)?;
    lower_checked(
        &source_units,
        &environment,
        actor.as_ref(),
        options,
        context,
    )
}

fn parse_program(source: &str, source_name: Option<String>) -> Result<IDLProg, CompileError> {
    source
        .parse::<IDLProg>()
        .map_err(|error| candid_error(error, DiagnosticPhase::Parse, source_name))
}

/// Reject stack-hostile syntax before any recursive upstream parser or checker
/// sees it. The token stream skips strings and comments, so their contents do
/// not affect the operational nesting budget.
fn check_source_nesting(source: &str, limits: &crate::Limits) -> Result<(), CompileError> {
    let mut delimiters = 0usize;
    let mut unary = 0usize;
    for token in Tokenizer::new(source) {
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

/// Follow parsed declaration references with an explicit stack before the
/// upstream checker can recursively expand a long chain of shallow aliases.
fn check_program_type_depth(program: &IDLProg, limits: &crate::Limits) -> Result<(), CompileError> {
    let declarations: BTreeMap<_, _> = program
        .decs
        .iter()
        .filter_map(|declaration| match declaration {
            Dec::TypD(binding) => Some((binding.id.as_str(), &binding.typ)),
            _ => None,
        })
        .collect();
    let mut pending: Vec<_> = declarations
        .values()
        .copied()
        .chain(program.actor.as_ref().map(|actor| &actor.typ))
        .map(|ty| (ty, 1usize, BTreeSet::<&str>::new()))
        .collect();

    while let Some((ty, depth, active_names)) = pending.pop() {
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
                        pending.push((resolved, next_depth, next_names));
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

fn load_source_units_with_resolver(
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
        check_program_type_depth(&program, limits)?;
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
struct SourceAccounting {
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

fn accept_source(
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

struct MaterializedBundle {
    root: PathBuf,
    entry: PathBuf,
}

impl MaterializedBundle {
    fn new(units: &[SourceUnit], entry: &crate::SourceId) -> Result<Self, CompileError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed);
        let root = std::env::temp_dir().join(format!("candid-core-{}-{id}", std::process::id()));
        let indexes = units
            .iter()
            .enumerate()
            .map(|(index, unit)| {
                let id = crate::SourceId::parse(&unit.name)
                    .map_err(crate::ResolveError::into_compile_error)?;
                Ok((id, index))
            })
            .collect::<Result<BTreeMap<_, _>, CompileError>>()?;
        let entry_index = indexes.get(entry).copied().ok_or_else(|| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                "entry source is missing from the resolved bundle",
            )
        })?;
        create_private_dir(&root).map_err(|error| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                format!("cannot create isolated source bundle: {error}"),
            )
        })?;
        let bundle = Self {
            entry: root.join(format!("{entry_index}.did")),
            root,
        };
        for (index, unit) in units.iter().enumerate() {
            let path = bundle.root.join(format!("{index}.did"));
            let source = materialized_source(unit, &indexes)?;
            fs::write(&path, source).map_err(|error| {
                CompileError::single(
                    "did_materialize_error",
                    DiagnosticPhase::Load,
                    format!("cannot materialize source {:?}: {error}", unit.name),
                )
            })?;
        }
        Ok(bundle)
    }
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn materialized_source(
    unit: &SourceUnit,
    indexes: &BTreeMap<crate::SourceId, usize>,
) -> Result<String, CompileError> {
    let mut source = String::new();
    for import in &unit.imports {
        let target = indexes.get(&import.target).copied().ok_or_else(|| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                format!(
                    "resolved import {:?} is missing from the source bundle",
                    import.target.as_str()
                ),
            )
        })?;
        match import.kind {
            SourceImportKind::Type => source.push_str(&format!("import \"{target}.did\";\n")),
            SourceImportKind::Service => {
                source.push_str(&format!("import service \"{target}.did\";\n"));
            }
        }
    }
    let program = parse_program(&unit.source, Some(unit.name.clone()))?;
    source.push_str(&pretty_print(&IDLMergedProg::new(program)));
    Ok(source)
}

impl Drop for MaterializedBundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn lower_checked(
    source_units: &[SourceUnit],
    environment: &TypeEnv,
    actor_type: Option<&Type>,
    options: CompileOptions,
    context: &RuntimeContext,
) -> Result<Compilation, CompileError> {
    check_type_depth(environment, actor_type, &context.limits)?;
    let mut lowerer = Lowerer::new(environment);
    let declaration_names: Vec<_> = environment.0.keys().cloned().collect();
    for name in &declaration_names {
        lowerer.lower_named(name).map_err(lower_error)?;
    }

    let actor = match actor_type {
        None => None,
        Some(actor_type) => {
            let reference = lowerer.lower_type(actor_type).map_err(lower_error)?;
            let node = lowerer.nodes[reference as usize].as_ref().ok_or_else(|| {
                lower_error("actor type was not fully lowered into the Contract arena")
            })?;
            match node {
                TypeNode::Service { .. } => Some(Actor::Service { service: reference }),
                TypeNode::Class { .. } => Some(Actor::Class { class: reference }),
                _ => return Err(lower_error(
                    "official Candid checker returned an actor that is neither service nor class",
                )),
            }
        }
    };

    let mut raw_source_info = RawSourceInfo::default();
    if options.include_source_info {
        let actor_service = actor_service_ref(actor.as_ref(), &lowerer).map_err(lower_error)?;
        collect_source_units_info(
            source_units,
            actor_service,
            environment,
            &mut lowerer,
            &mut raw_source_info,
        )
        .map_err(lower_error)?;
    }

    let LoweredGraph { types, named_refs } = lowerer.finish().map_err(lower_error)?;
    let declarations = declaration_names
        .iter()
        .map(|name| {
            Ok(Declaration {
                name: name.clone(),
                ty: *named_refs
                    .get(name)
                    .ok_or_else(|| format!("missing lowered declaration {name}"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(lower_error)?;

    // Structural validation needs a syntactically valid placeholder. The
    // canonicalizer then computes the real identities.
    let raw_contract = Contract::new_unchecked(types, declarations, actor);
    crate::validate::validate_structure_with_limits(&raw_contract, &context.limits)
        .map_err(|error| lower_error(format!("lowered Contract violated an invariant: {error}")))?;
    let canonicalized =
        canonical::canonicalize_with_mapping_unchecked_and_limits(&raw_contract, &context.limits)
            .map_err(|error| lower_error(format!("canonicalization failed: {error}")))?;

    let source_info = if options.include_source_info {
        let mut field_labels: Vec<_> = raw_source_info
            .field_labels
            .into_iter()
            .map(|label| FieldLabelProvenance {
                origin: label.origin,
                path: label.path,
                container: canonicalized.old_to_new[label.container as usize],
                id: label.id,
                label: label.label,
                docs: label.docs,
            })
            .collect();
        field_labels.sort_by(compare_field_label_provenance);

        let mut methods: Vec<_> = raw_source_info
            .methods
            .into_iter()
            .map(|method| SourceMethodInfo {
                origin: method.origin,
                path: method.path,
                service: canonicalized.old_to_new[method.service as usize],
                name: method.name,
                docs: method.docs,
            })
            .collect();
        methods.sort_by(compare_source_method_info);

        let mut function_arguments: Vec<_> = raw_source_info
            .function_arguments
            .into_iter()
            .map(|argument| SourceFunctionArgumentInfo {
                origin: argument.origin,
                path: argument.path,
                function: canonicalized.old_to_new[argument.function as usize],
                direction: argument.direction,
                position: argument.position,
                name: argument.name,
            })
            .collect();
        function_arguments.sort_by(compare_source_function_argument_info);

        let mut declarations: Vec<_> = raw_source_info
            .declarations
            .into_iter()
            .map(|declaration| SourceDeclaration {
                source: declaration.source,
                name: declaration.name,
                ty: canonicalized.old_to_new[declaration.ty as usize],
                docs: declaration.docs,
            })
            .collect();
        declarations.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.name.cmp(&right.name))
                .then(left.ty.cmp(&right.ty))
        });
        let mut actors: Vec<_> = raw_source_info
            .actors
            .into_iter()
            .map(|actor| SourceActorInfo {
                source: actor.source,
                docs: actor.docs,
            })
            .collect();
        actors.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.docs.cmp(&right.docs))
        });
        let mut sources: Vec<_> = source_units
            .iter()
            .map(|unit| SourceFileInfo {
                name: unit.name.clone(),
                source: unit.source.clone(),
            })
            .collect();
        sources.sort_by(|left, right| left.name.cmp(&right.name));
        let mut imports = source_imports(source_units);
        imports.sort();
        let source_bundle_id = crate::source::source_bundle_id(&sources, &imports);
        let source_info = SourceInfo {
            source_info_version: SOURCE_INFO_VERSION,
            contract_id: canonicalized.contract.contract_id().to_string(),
            source_bundle_id,
            sources,
            imports,
            declarations,
            field_labels,
            methods,
            function_arguments,
            actors,
        };
        source_info
            .validate(&canonicalized.contract, &context.limits)
            .map_err(source_info_compile_error)?;
        Some(source_info)
    } else {
        None
    };

    Ok(Compilation {
        contract: canonicalized.contract,
        source_info,
    })
}

fn check_type_depth(
    environment: &TypeEnv,
    actor_type: Option<&Type>,
    limits: &crate::Limits,
) -> Result<(), CompileError> {
    let mut roots: Vec<Type> = environment.0.values().cloned().collect();
    roots.extend(actor_type.cloned());
    let mut pending: Vec<_> = roots
        .into_iter()
        .map(|ty| (ty, 1usize, BTreeSet::<String>::new()))
        .collect();

    while let Some((ty, depth, active_names)) = pending.pop() {
        if depth > limits.max_type_depth {
            return Err(CompileError::resource_limit(
                "type_depth",
                limits.max_type_depth,
                depth,
                format!(
                    "checked Candid type depth {depth} exceeds limit {}",
                    limits.max_type_depth
                ),
            ));
        }
        let next_depth = depth.saturating_add(1);
        match ty.as_ref() {
            TypeInner::Var(name) => {
                if !active_names.contains(name) {
                    let mut next_names = active_names;
                    next_names.insert(name.clone());
                    let resolved = environment
                        .find_type(name)
                        .map_err(|error| lower_error(error.to_string()))?
                        .clone();
                    pending.push((resolved, next_depth, next_names));
                }
            }
            TypeInner::Opt(inner) | TypeInner::Vec(inner) => {
                pending.push((inner.clone(), next_depth, active_names));
            }
            TypeInner::Record(fields) | TypeInner::Variant(fields) => {
                for field in fields {
                    pending.push((field.ty.clone(), next_depth, active_names.clone()));
                }
            }
            TypeInner::Func(function) => {
                for ty in function.args.iter().chain(&function.rets) {
                    pending.push((ty.clone(), next_depth, active_names.clone()));
                }
            }
            TypeInner::Service(methods) => {
                for (_, ty) in methods {
                    pending.push((ty.clone(), next_depth, active_names.clone()));
                }
            }
            TypeInner::Class(init, service) => {
                pending.push((service.clone(), next_depth, active_names.clone()));
                for ty in init {
                    pending.push((ty.clone(), next_depth, active_names.clone()));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn source_info_compile_error(error: crate::ContractValidationError) -> CompileError {
    CompileError {
        diagnostics: error
            .violations
            .into_iter()
            .map(|violation| Diagnostic {
                code: violation.code,
                phase: DiagnosticPhase::Lower,
                severity: Severity::Error,
                message: format!("{}: {}", violation.path, violation.message),
                span: None,
                notes: Vec::new(),
                resource_limit: violation.resource_limit,
            })
            .collect(),
    }
}

fn source_imports(source_units: &[SourceUnit]) -> Vec<SourceImportInfo> {
    let mut imports = Vec::new();
    for unit in source_units {
        for import in &unit.imports {
            imports.push(SourceImportInfo {
                from: unit.name.clone(),
                import: import.import.clone(),
                to: import.target.as_str().to_string(),
                kind: import.kind,
            });
        }
    }
    imports
}

fn lower_error(message: impl Into<String>) -> CompileError {
    CompileError::single("contract_lowering_error", DiagnosticPhase::Lower, message)
}

fn candid_file_error(error: candid_parser::Error) -> CompileError {
    let phase = match &error {
        candid_parser::Error::Parse(_) => DiagnosticPhase::Parse,
        candid_parser::Error::Custom(inner)
            if inner.to_string().contains("Cannot import")
                || inner.to_string().contains("Cannot open")
                || inner.to_string().contains("io error") =>
        {
            DiagnosticPhase::Load
        }
        candid_parser::Error::Custom(_) | candid_parser::Error::CandidError(_) => {
            DiagnosticPhase::TypeCheck
        }
    };
    candid_error(error, phase, None)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn materialized_bundle_root_is_private_and_self_cleaning() {
        use std::os::unix::fs::PermissionsExt;

        let source = "service : {};";
        let entry = crate::SourceId::parse("memory:/private.did").unwrap();
        let unit = SourceUnit {
            name: entry.as_str().to_string(),
            source: source.to_string(),
            program: parse_program(source, Some(entry.as_str().to_string())).unwrap(),
            imports: Vec::new(),
            include_actor: true,
        };
        let bundle = MaterializedBundle::new(&[unit], &entry).unwrap();
        let root = bundle.root.clone();
        let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        drop(bundle);
        assert!(!root.exists());
    }
}

fn candid_error(
    error: candid_parser::Error,
    phase: DiagnosticPhase,
    source_name: Option<String>,
) -> CompileError {
    let message = error.to_string();
    let report = error.report();
    let span = report.labels.first().map(|label| SourceSpan {
        source_name,
        start_byte: label.range.start,
        end_byte: label.range.end,
    });
    let code = match phase {
        DiagnosticPhase::Parse => "did_parse_error",
        DiagnosticPhase::TypeCheck => "did_type_check_error",
        DiagnosticPhase::Load => "did_load_error",
        DiagnosticPhase::Lower => "contract_lowering_error",
    };
    CompileError {
        diagnostics: vec![Diagnostic {
            code: code.to_string(),
            phase,
            severity: Severity::Error,
            message,
            span,
            notes: report.notes,
            resource_limit: None,
        }],
    }
}

struct Lowerer<'a> {
    environment: &'a TypeEnv,
    nodes: Vec<Option<TypeNode>>,
    named_refs: BTreeMap<String, TypeRef>,
    primitive_refs: BTreeMap<PrimitiveType, TypeRef>,
    composite_refs: HashMap<Type, TypeRef>,
}

struct LoweredGraph {
    types: Vec<TypeNode>,
    named_refs: BTreeMap<String, TypeRef>,
}

impl<'a> Lowerer<'a> {
    fn new(environment: &'a TypeEnv) -> Self {
        Self {
            environment,
            nodes: Vec::new(),
            named_refs: BTreeMap::new(),
            primitive_refs: BTreeMap::new(),
            composite_refs: HashMap::new(),
        }
    }

    fn finish(self) -> Result<LoweredGraph, String> {
        let mut types = Vec::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.into_iter().enumerate() {
            types.push(node.ok_or_else(|| format!("type node {index} was left incomplete"))?);
        }
        Ok(LoweredGraph {
            types,
            named_refs: self.named_refs,
        })
    }

    fn lower_named(&mut self, name: &str) -> Result<TypeRef, String> {
        let mut pending = Vec::new();
        let reference = self.ensure_named(name, &mut pending)?;
        self.drain_pending(&mut pending)?;
        Ok(reference)
    }

    fn ensure_named(
        &mut self,
        name: &str,
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<TypeRef, String> {
        if let Some(reference) = self.named_refs.get(name) {
            return Ok(*reference);
        }
        let terminal = self.terminal_name(name)?;
        if terminal != name {
            let reference = self.ensure_named(&terminal, pending)?;
            self.named_refs.insert(name.to_string(), reference);
            return Ok(reference);
        }

        let ty = self
            .environment
            .find_type(&terminal)
            .map_err(|error| error.to_string())?
            .clone();
        if primitive_from_type(ty.as_ref()).is_some() {
            let reference = self.ensure_type(&ty, pending)?;
            self.named_refs.insert(terminal, reference);
            return Ok(reference);
        }
        if let Some(reference) = self.composite_refs.get(&ty) {
            self.named_refs.insert(terminal, *reference);
            return Ok(*reference);
        }

        let reference = self.reserve()?;
        self.named_refs.insert(terminal, reference);
        self.composite_refs.insert(ty.clone(), reference);
        pending.push((reference, ty));
        Ok(reference)
    }

    fn terminal_name(&self, name: &str) -> Result<String, String> {
        let mut seen = BTreeSet::new();
        let mut current = name.to_string();
        loop {
            if !seen.insert(current.clone()) {
                return Err(format!("unproductive alias cycle reached {current}"));
            }
            let ty = self
                .environment
                .find_type(&current)
                .map_err(|error| error.to_string())?;
            if let TypeInner::Var(next) = ty.as_ref() {
                current = next.clone();
            } else {
                return Ok(current);
            }
        }
    }

    fn lower_type(&mut self, ty: &Type) -> Result<TypeRef, String> {
        let mut pending = Vec::new();
        let reference = self.ensure_type(ty, &mut pending)?;
        self.drain_pending(&mut pending)?;
        Ok(reference)
    }

    fn ensure_type(
        &mut self,
        ty: &Type,
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<TypeRef, String> {
        if let TypeInner::Var(name) = ty.as_ref() {
            return self.ensure_named(name, pending);
        }
        if let Some(primitive) = primitive_from_type(ty.as_ref()) {
            if let Some(reference) = self.primitive_refs.get(&primitive) {
                return Ok(*reference);
            }
            let reference = self.reserve()?;
            self.primitive_refs.insert(primitive, reference);
            self.nodes[reference as usize] = Some(TypeNode::Primitive { primitive });
            return Ok(reference);
        }
        if let Some(reference) = self.composite_refs.get(ty) {
            return Ok(*reference);
        }
        let reference = self.reserve()?;
        self.composite_refs.insert(ty.clone(), reference);
        pending.push((reference, ty.clone()));
        Ok(reference)
    }

    fn drain_pending(&mut self, pending: &mut Vec<(TypeRef, Type)>) -> Result<(), String> {
        while let Some((reference, ty)) = pending.pop() {
            if self.nodes[reference as usize].is_none() {
                self.fill_one(reference, &ty, pending)?;
            }
        }
        Ok(())
    }

    fn reserve(&mut self) -> Result<TypeRef, String> {
        let reference = u32::try_from(self.nodes.len())
            .map_err(|_| "Contract type arena exceeds u32 references")?;
        self.nodes.push(None);
        Ok(reference)
    }

    fn fill_one(
        &mut self,
        reference: TypeRef,
        ty: &Type,
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<(), String> {
        let node = match ty.as_ref() {
            TypeInner::Opt(inner) => TypeNode::Opt {
                inner: self.ensure_type(inner, pending)?,
            },
            TypeInner::Vec(inner) => TypeNode::Vec {
                inner: self.ensure_type(inner, pending)?,
            },
            TypeInner::Record(fields) => TypeNode::Record {
                fields: self.lower_fields(fields, pending)?,
            },
            TypeInner::Variant(fields) => TypeNode::Variant {
                fields: self.lower_fields(fields, pending)?,
            },
            TypeInner::Func(function) => TypeNode::Func {
                args: function
                    .args
                    .iter()
                    .map(|argument| self.ensure_type(argument, pending))
                    .collect::<Result<Vec<_>, _>>()?,
                results: function
                    .rets
                    .iter()
                    .map(|result| self.ensure_type(result, pending))
                    .collect::<Result<Vec<_>, _>>()?,
                mode: lower_mode(&function.modes)?,
            },
            TypeInner::Service(methods) => {
                let mut lowered = methods
                    .iter()
                    .map(|(name, function)| {
                        Ok(ServiceMethod {
                            name: name.clone(),
                            id: candid_parser::candid::idl_hash(name),
                            function: self.ensure_type(function, pending)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                lowered.sort_by(|left, right| {
                    left.id
                        .cmp(&right.id)
                        .then(left.name.cmp(&right.name))
                        .then(left.function.cmp(&right.function))
                });
                TypeNode::Service { methods: lowered }
            }
            TypeInner::Class(init, service) => TypeNode::Class {
                init: init
                    .iter()
                    .map(|argument| self.ensure_type(argument, pending))
                    .collect::<Result<Vec<_>, _>>()?,
                service: self.ensure_type(service, pending)?,
            },
            TypeInner::Var(name) => {
                return Err(format!("unresolved alias {name} leaked into lowering"))
            }
            TypeInner::Unknown => {
                return Err("unknown Candid type leaked into lowering".to_string())
            }
            TypeInner::Knot(_) => {
                return Err("Rust-only recursive knot leaked into DID lowering".to_string())
            }
            TypeInner::Future => {
                return Err("unsupported non-DID future type leaked into lowering".to_string())
            }
            primitive if primitive_from_type(primitive).is_some() => {
                return Err("primitive type reached composite lowering".to_string())
            }
            other => {
                return Err(format!(
                    "unsupported Candid type leaked into lowering: {other:?}"
                ))
            }
        };
        self.nodes[reference as usize] = Some(node);
        Ok(())
    }

    fn lower_fields(
        &mut self,
        fields: &[candid_parser::candid::types::Field],
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<Vec<Field>, String> {
        let mut lowered = Vec::with_capacity(fields.len());
        for field in fields {
            let id = field.id.get_id();
            lowered.push(Field {
                id,
                ty: self.ensure_type(&field.ty, pending)?,
            });
        }
        lowered.sort_by(|left, right| left.id.cmp(&right.id).then(left.ty.cmp(&right.ty)));
        Ok(lowered)
    }
}

#[derive(Default)]
struct RawSourceInfo {
    declarations: Vec<RawDeclarationInfo>,
    field_labels: Vec<RawFieldLabelInfo>,
    methods: Vec<RawMethodInfo>,
    function_arguments: Vec<RawFunctionArgumentInfo>,
    actors: Vec<RawActorInfo>,
}

struct RawDeclarationInfo {
    source: String,
    name: String,
    ty: TypeRef,
    docs: Vec<String>,
}

struct RawActorInfo {
    source: String,
    docs: Vec<String>,
}

struct RawFieldLabelInfo {
    origin: SourceOrigin,
    path: String,
    container: TypeRef,
    id: u32,
    label: SourceLabel,
    docs: Vec<String>,
}

struct RawMethodInfo {
    origin: SourceOrigin,
    path: String,
    service: TypeRef,
    name: String,
    docs: Vec<String>,
}

struct RawFunctionArgumentInfo {
    origin: SourceOrigin,
    path: String,
    function: TypeRef,
    direction: SourceFunctionArgumentDirection,
    position: u32,
    name: String,
}

/// Walk the parser's source AST only for provenance. Every semantic type ref
/// comes from `ast_to_type` plus the checked environment, never from a
/// handwritten interpretation of Candid grammar or type rules.
fn actor_service_ref(
    actor: Option<&Actor>,
    lowerer: &Lowerer<'_>,
) -> Result<Option<TypeRef>, String> {
    match actor {
        None => Ok(None),
        Some(Actor::Service { service }) => Ok(Some(*service)),
        Some(Actor::Class { class }) => match lowerer.nodes[*class as usize].as_ref() {
            Some(TypeNode::Class { service, .. }) => Ok(Some(*service)),
            _ => Err("actor class was not fully lowered".to_string()),
        },
    }
}

fn collect_source_units_info(
    source_units: &[SourceUnit],
    actor_service: Option<TypeRef>,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    for unit in source_units {
        for declaration in &unit.program.decs {
            if let Dec::TypD(binding) = declaration {
                let origin = SourceOrigin::Declaration {
                    source: unit.name.clone(),
                    name: binding.id.clone(),
                };
                let ty = lowerer.lower_named(&binding.id)?;
                output.declarations.push(RawDeclarationInfo {
                    source: unit.name.clone(),
                    name: binding.id.clone(),
                    ty,
                    docs: binding.docs.clone(),
                });
                collect_type_source_info(
                    &binding.typ,
                    &origin,
                    &format!("type:{}", binding.id),
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
        if unit.include_actor {
            if let Some(actor) = &unit.program.actor {
                let origin = SourceOrigin::Actor {
                    source: unit.name.clone(),
                };
                output.actors.push(RawActorInfo {
                    source: unit.name.clone(),
                    docs: actor.docs.clone(),
                });
                collect_actor_source_info(
                    &actor.typ,
                    &origin,
                    actor_service,
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
    }
    Ok(())
}

fn collect_actor_source_info(
    ast: &IDLType,
    origin: &SourceOrigin,
    actor_service: Option<TypeRef>,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    match ast {
        IDLType::ServT(methods) => {
            if let Some(service) = actor_service {
                collect_service_source_info(
                    methods,
                    origin,
                    "actor",
                    service,
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
        IDLType::ClassT(init, service) => {
            for (position, argument) in init.iter().enumerate() {
                collect_type_source_info(
                    &argument.typ,
                    origin,
                    &format!("actor.init[{position}].type"),
                    environment,
                    lowerer,
                    output,
                )?;
            }
            if let IDLType::ServT(methods) = service.as_ref() {
                if let Some(service) = actor_service {
                    collect_service_source_info(
                        methods,
                        origin,
                        "actor.service",
                        service,
                        environment,
                        lowerer,
                        output,
                    )?;
                }
            }
        }
        IDLType::VarT(_) => {}
        other => {
            collect_type_source_info(other, origin, "actor", environment, lowerer, output)?;
        }
    }
    Ok(())
}

fn collect_type_source_info(
    ast: &IDLType,
    origin: &SourceOrigin,
    path: &str,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    let mut pending = vec![(ast, path.to_string())];
    while let Some((ast, path)) = pending.pop() {
        match ast {
            IDLType::RecordT(fields) | IDLType::VariantT(fields) => {
                let container = lower_ast_type(ast, environment, lowerer)?;
                for (position, field) in fields.iter().enumerate() {
                    let field_path = format!("{path}.fields[{position}]");
                    output.field_labels.push(RawFieldLabelInfo {
                        origin: origin.clone(),
                        path: field_path.clone(),
                        container,
                        id: field.label.get_id(),
                        label: lower_source_label(&field.label),
                        docs: field.docs.clone(),
                    });
                }
                for (position, field) in fields.iter().enumerate().rev() {
                    pending.push((&field.typ, format!("{path}.fields[{position}].type")));
                }
            }
            IDLType::FuncT(function) => {
                let function_ref = lower_ast_type(ast, environment, lowerer)?;
                for (position, argument) in function.args.iter().enumerate() {
                    record_function_argument_name(
                        origin,
                        &format!("{path}.args[{position}]"),
                        function_ref,
                        SourceFunctionArgumentDirection::Argument,
                        position,
                        argument.name.as_deref(),
                        output,
                    )?;
                }
                for (position, result) in function.rets.iter().enumerate() {
                    record_function_argument_name(
                        origin,
                        &format!("{path}.results[{position}]"),
                        function_ref,
                        SourceFunctionArgumentDirection::Result,
                        position,
                        result.name.as_deref(),
                        output,
                    )?;
                }
                for (position, result) in function.rets.iter().enumerate().rev() {
                    pending.push((&result.typ, format!("{path}.results[{position}].type")));
                }
                for (position, argument) in function.args.iter().enumerate().rev() {
                    pending.push((&argument.typ, format!("{path}.args[{position}].type")));
                }
            }
            IDLType::ServT(methods) => {
                let service = lower_ast_type(ast, environment, lowerer)?;
                for (position, method) in methods.iter().enumerate() {
                    let method_path = format!("{path}.methods[{position}]");
                    output.methods.push(RawMethodInfo {
                        origin: origin.clone(),
                        path: method_path,
                        service,
                        name: method.id.clone(),
                        docs: method.docs.clone(),
                    });
                }
                for (position, method) in methods.iter().enumerate().rev() {
                    pending.push((&method.typ, format!("{path}.methods[{position}].function")));
                }
            }
            IDLType::ClassT(init, service) => {
                pending.push((service, format!("{path}.service")));
                for (position, argument) in init.iter().enumerate().rev() {
                    pending.push((&argument.typ, format!("{path}.init[{position}].type")));
                }
            }
            IDLType::OptT(inner) | IDLType::VecT(inner) => {
                pending.push((inner, format!("{path}.inner")));
            }
            IDLType::PrimT(_) | IDLType::VarT(_) | IDLType::PrincipalT => {}
        }
    }
    Ok(())
}

fn collect_service_source_info(
    methods: &[candid_parser::syntax::Binding],
    origin: &SourceOrigin,
    path: &str,
    service: TypeRef,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    for (position, method) in methods.iter().enumerate() {
        let method_path = format!("{path}.methods[{position}]");
        output.methods.push(RawMethodInfo {
            origin: origin.clone(),
            path: method_path.clone(),
            service,
            name: method.id.clone(),
            docs: method.docs.clone(),
        });
        collect_type_source_info(
            &method.typ,
            origin,
            &format!("{method_path}.function"),
            environment,
            lowerer,
            output,
        )?;
    }
    Ok(())
}

fn lower_ast_type(
    ast: &IDLType,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
) -> Result<TypeRef, String> {
    let ty = ast_to_type(environment, ast).map_err(|error| error.to_string())?;
    lowerer.lower_type(&ty)
}

fn record_function_argument_name(
    origin: &SourceOrigin,
    path: &str,
    function: TypeRef,
    direction: SourceFunctionArgumentDirection,
    position: usize,
    name: Option<&str>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    let Some(name) = name else {
        return Ok(());
    };
    output.function_arguments.push(RawFunctionArgumentInfo {
        origin: origin.clone(),
        path: path.to_string(),
        function,
        direction,
        position: u32::try_from(position).map_err(|_| "function argument position exceeds u32")?,
        name: name.to_string(),
    });
    Ok(())
}

fn lower_source_label(label: &Label) -> SourceLabel {
    match label {
        Label::Named(name) => SourceLabel::Named { name: name.clone() },
        Label::Id(_) => SourceLabel::Numeric,
        Label::Unnamed(_) => SourceLabel::Positional,
    }
}

fn primitive_from_type(ty: &TypeInner) -> Option<PrimitiveType> {
    Some(match ty {
        TypeInner::Null => PrimitiveType::Null,
        TypeInner::Bool => PrimitiveType::Bool,
        TypeInner::Nat => PrimitiveType::Nat,
        TypeInner::Int => PrimitiveType::Int,
        TypeInner::Nat8 => PrimitiveType::Nat8,
        TypeInner::Nat16 => PrimitiveType::Nat16,
        TypeInner::Nat32 => PrimitiveType::Nat32,
        TypeInner::Nat64 => PrimitiveType::Nat64,
        TypeInner::Int8 => PrimitiveType::Int8,
        TypeInner::Int16 => PrimitiveType::Int16,
        TypeInner::Int32 => PrimitiveType::Int32,
        TypeInner::Int64 => PrimitiveType::Int64,
        TypeInner::Float32 => PrimitiveType::Float32,
        TypeInner::Float64 => PrimitiveType::Float64,
        TypeInner::Text => PrimitiveType::Text,
        TypeInner::Reserved => PrimitiveType::Reserved,
        TypeInner::Empty => PrimitiveType::Empty,
        TypeInner::Principal => PrimitiveType::Principal,
        _ => return None,
    })
}

fn lower_mode(modes: &[FuncMode]) -> Result<MethodMode, String> {
    match modes {
        [] => Ok(MethodMode::Update),
        [FuncMode::Query] => Ok(MethodMode::Query),
        [FuncMode::CompositeQuery] => Ok(MethodMode::CompositeQuery),
        [FuncMode::Oneway] => Ok(MethodMode::Oneway),
        _ => Err("official Candid checker returned more than one function mode".to_string()),
    }
}

fn compare_field_label_provenance(
    left: &FieldLabelProvenance,
    right: &FieldLabelProvenance,
) -> std::cmp::Ordering {
    left.origin
        .cmp(&right.origin)
        .then(left.path.cmp(&right.path))
        .then(left.container.cmp(&right.container))
        .then(left.id.cmp(&right.id))
        .then(source_label_order(&left.label, &right.label))
        .then(left.docs.cmp(&right.docs))
}

fn compare_source_method_info(
    left: &SourceMethodInfo,
    right: &SourceMethodInfo,
) -> std::cmp::Ordering {
    left.origin
        .cmp(&right.origin)
        .then(left.path.cmp(&right.path))
        .then(left.service.cmp(&right.service))
        .then(left.name.cmp(&right.name))
        .then(left.docs.cmp(&right.docs))
}

fn compare_source_function_argument_info(
    left: &SourceFunctionArgumentInfo,
    right: &SourceFunctionArgumentInfo,
) -> std::cmp::Ordering {
    left.origin
        .cmp(&right.origin)
        .then(left.path.cmp(&right.path))
        .then(left.function.cmp(&right.function))
        .then(left.direction.cmp(&right.direction))
        .then(left.position.cmp(&right.position))
        .then(left.name.cmp(&right.name))
}

fn source_label_order(left: &SourceLabel, right: &SourceLabel) -> std::cmp::Ordering {
    let rank = |label: &SourceLabel| match label {
        SourceLabel::Named { .. } => 0,
        SourceLabel::Numeric => 1,
        SourceLabel::Positional => 2,
    };
    rank(left)
        .cmp(&rank(right))
        .then_with(|| match (left, right) {
            (SourceLabel::Named { name: left }, SourceLabel::Named { name: right }) => {
                left.cmp(right)
            }
            _ => std::cmp::Ordering::Equal,
        })
}
