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
use candid_parser::syntax::{Dec, IDLProg, IDLType};
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
    /// sidecar. This never changes the Contract or its fingerprint.
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
    include_actor: bool,
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
    if context.limits.deadline_exceeded() {
        return Err(CompileError::single(
            "operation_deadline_exceeded",
            DiagnosticPhase::Load,
            "compilation deadline has elapsed",
        ));
    }
    if source.len() > context.limits.max_source_bytes {
        return Err(CompileError::resource_limit(
            "source_bytes",
            context.limits.max_source_bytes,
            source.len(),
            format!(
                "inline source uses {} bytes; limit is {}",
                source.len(),
                context.limits.max_source_bytes
            ),
        ));
    }
    let program = parse_program(source, Some("memory:/inline.did".to_string()))?;
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

fn load_source_units_with_resolver(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    context: &RuntimeContext,
) -> Result<(Vec<SourceUnit>, crate::SourceId), CompileError> {
    struct Pending {
        from: Option<crate::SourceId>,
        import: String,
        include_actor: bool,
        depth: usize,
        ancestors: Vec<crate::SourceId>,
    }

    let limits = &context.limits;
    let mut units = Vec::<SourceUnit>::new();
    let mut indexes = BTreeMap::<crate::SourceId, usize>::new();
    let mut pending = vec![Pending {
        from: None,
        import: entry.to_string(),
        include_actor: true,
        depth: 0,
        ancestors: Vec::new(),
    }];
    let mut entry_id = None;
    let mut total_bytes = 0usize;
    let mut import_edges = 0usize;

    while let Some(request) = pending.pop() {
        if limits.deadline_exceeded() {
            return Err(CompileError::single(
                "operation_deadline_exceeded",
                DiagnosticPhase::Load,
                "source resolution deadline has elapsed",
            ));
        }
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
        let source_id = resolver
            .identify(request.from.as_ref(), &request.import)
            .map_err(crate::ResolveError::into_compile_error)?;
        if request.ancestors.contains(&source_id) {
            return Err(CompileError::single(
                "did_import_cycle",
                DiagnosticPhase::Load,
                format!("import cycle reached {:?}", source_id.as_str()),
            ));
        }
        if entry_id.is_none() {
            entry_id = Some(source_id.clone());
        }
        if let Some(index) = indexes.get(&source_id).copied() {
            units[index].include_actor |= request.include_actor;
            continue;
        }
        let resolved = resolver
            .load(&source_id, limits)
            .map_err(crate::ResolveError::into_compile_error)?;
        if resolved.id != source_id {
            return Err(CompileError::single(
                "did_resolver_identity_mismatch",
                DiagnosticPhase::Load,
                format!(
                    "resolver identified {:?} but loaded {:?}",
                    source_id.as_str(),
                    resolved.id.as_str()
                ),
            ));
        }
        resolved
            .verify()
            .map_err(crate::ResolveError::into_compile_error)?;
        if units.len() >= limits.max_sources {
            return Err(CompileError::resource_limit(
                "sources",
                limits.max_sources,
                units.len() + 1,
                format!("source count exceeds limit {}", limits.max_sources),
            ));
        }
        total_bytes = total_bytes.saturating_add(resolved.source.len());
        if total_bytes > limits.max_bundle_bytes {
            return Err(CompileError::resource_limit(
                "bundle_bytes",
                limits.max_bundle_bytes,
                total_bytes,
                format!(
                    "source bundle uses {total_bytes} bytes; limit is {}",
                    limits.max_bundle_bytes
                ),
            ));
        }
        let program = parse_program(&resolved.source, Some(resolved.id.as_str().to_string()))?;
        let imports: Vec<_> = program
            .decs
            .iter()
            .filter_map(|declaration| match declaration {
                Dec::ImportType(import) => Some((import.clone(), false)),
                Dec::ImportServ(import) => Some((import.clone(), true)),
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
        let index = units.len();
        indexes.insert(source_id.clone(), index);
        units.push(SourceUnit {
            name: resolved.id.as_str().to_string(),
            source: resolved.source,
            program,
            include_actor: request.include_actor,
        });
        let mut ancestors = request.ancestors;
        ancestors.push(resolved.id.clone());
        for (import, imports_actor) in imports.into_iter().rev() {
            pending.push(Pending {
                from: Some(resolved.id.clone()),
                import,
                include_actor: imports_actor,
                depth: request.depth + 1,
                ancestors: ancestors.clone(),
            });
        }
    }
    Ok((
        units,
        entry_id.expect("entry resolution creates at least one source"),
    ))
}

struct MaterializedBundle {
    root: PathBuf,
    entry: PathBuf,
}

impl MaterializedBundle {
    fn new(units: &[SourceUnit], entry: &crate::SourceId) -> Result<Self, CompileError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "candid-contract-runtime-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|error| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                format!("cannot create isolated source bundle: {error}"),
            )
        })?;
        let bundle = Self {
            entry: root.join(entry.path()),
            root,
        };
        for unit in units {
            let id =
                crate::SourceId::parse(&unit.name).expect("resolver-produced source IDs are valid");
            let path = bundle.root.join(id.path());
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    CompileError::single(
                        "did_materialize_error",
                        DiagnosticPhase::Load,
                        format!("cannot create isolated source directory: {error}"),
                    )
                })?;
            }
            fs::write(&path, &unit.source).map_err(|error| {
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
    // canonicalizer then computes the real fingerprint.
    let raw_contract = Contract::new_unchecked(types, declarations, actor);
    crate::validate::validate_structure_with_limits(&raw_contract, &context.limits)
        .map_err(|error| lower_error(format!("lowered Contract violated an invariant: {error}")))?;
    let canonicalized =
        canonical::canonicalize_with_mapping_unchecked_and_limits(&raw_contract, &context.limits)
            .map_err(|error| lower_error(format!("canonicalization failed: {error}")))?;

    let source_info = options.include_source_info.then(|| {
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
            .expect("compiler-generated SourceInfo must validate");
        source_info
    });

    Ok(Compilation {
        contract: canonicalized.contract,
        source_info,
    })
}

fn source_imports(source_units: &[SourceUnit]) -> Vec<SourceImportInfo> {
    let mut imports = Vec::new();
    for unit in source_units {
        let from = crate::SourceId::parse(&unit.name)
            .expect("resolver-produced source IDs are already normalized");
        for declaration in &unit.program.decs {
            let (import, kind) = match declaration {
                Dec::ImportType(import) => (import, SourceImportKind::Type),
                Dec::ImportServ(import) => (import, SourceImportKind::Service),
                Dec::TypD(_) => continue,
            };
            let to = crate::resolver::resolve_source_id(Some(&from), import)
                .expect("all imports were accepted by the resolver");
            imports.push(SourceImportInfo {
                from: unit.name.clone(),
                import: import.clone(),
                to: to.as_str().to_string(),
                kind,
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
        if let Some(reference) = self.named_refs.get(name) {
            return Ok(*reference);
        }
        let terminal = self.terminal_name(name)?;
        if terminal != name {
            let reference = self.lower_named(&terminal)?;
            self.named_refs.insert(name.to_string(), reference);
            return Ok(reference);
        }

        let ty = self
            .environment
            .find_type(&terminal)
            .map_err(|error| error.to_string())?
            .clone();
        if primitive_from_type(ty.as_ref()).is_some() {
            let reference = self.lower_type(&ty)?;
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
        self.fill(reference, &ty)?;
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
        if let TypeInner::Var(name) = ty.as_ref() {
            return self.lower_named(name);
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
        self.fill(reference, ty)?;
        Ok(reference)
    }

    fn reserve(&mut self) -> Result<TypeRef, String> {
        let reference = u32::try_from(self.nodes.len())
            .map_err(|_| "Contract type arena exceeds u32 references")?;
        self.nodes.push(None);
        Ok(reference)
    }

    fn fill(&mut self, reference: TypeRef, ty: &Type) -> Result<(), String> {
        let node = match ty.as_ref() {
            TypeInner::Opt(inner) => TypeNode::Opt {
                inner: self.lower_type(inner)?,
            },
            TypeInner::Vec(inner) => TypeNode::Vec {
                inner: self.lower_type(inner)?,
            },
            TypeInner::Record(fields) => TypeNode::Record {
                fields: self.lower_fields(fields)?,
            },
            TypeInner::Variant(fields) => TypeNode::Variant {
                fields: self.lower_fields(fields)?,
            },
            TypeInner::Func(function) => TypeNode::Func {
                args: function
                    .args
                    .iter()
                    .map(|argument| self.lower_type(argument))
                    .collect::<Result<Vec<_>, _>>()?,
                results: function
                    .rets
                    .iter()
                    .map(|result| self.lower_type(result))
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
                            function: self.lower_type(function)?,
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
                    .map(|argument| self.lower_type(argument))
                    .collect::<Result<Vec<_>, _>>()?,
                service: self.lower_type(service)?,
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
    ) -> Result<Vec<Field>, String> {
        let mut lowered = Vec::with_capacity(fields.len());
        for field in fields {
            let id = field.id.get_id();
            lowered.push(Field {
                id,
                ty: self.lower_type(&field.ty)?,
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
                collect_type_source_info(
                    &field.typ,
                    origin,
                    &format!("{field_path}.type"),
                    environment,
                    lowerer,
                    output,
                )?;
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
                collect_type_source_info(
                    &argument.typ,
                    origin,
                    &format!("{path}.args[{position}].type"),
                    environment,
                    lowerer,
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
                collect_type_source_info(
                    &result.typ,
                    origin,
                    &format!("{path}.results[{position}].type"),
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
        IDLType::ServT(methods) => {
            let service = lower_ast_type(ast, environment, lowerer)?;
            collect_service_source_info(
                methods,
                origin,
                path,
                service,
                environment,
                lowerer,
                output,
            )?;
        }
        IDLType::ClassT(init, service) => {
            for (position, argument) in init.iter().enumerate() {
                collect_type_source_info(
                    &argument.typ,
                    origin,
                    &format!("{path}.init[{position}].type"),
                    environment,
                    lowerer,
                    output,
                )?;
            }
            collect_type_source_info(
                service,
                origin,
                &format!("{path}.service"),
                environment,
                lowerer,
                output,
            )?;
        }
        IDLType::OptT(inner) | IDLType::VecT(inner) => {
            collect_type_source_info(
                inner,
                origin,
                &format!("{path}.inner"),
                environment,
                lowerer,
                output,
            )?;
        }
        IDLType::PrimT(_) | IDLType::VarT(_) | IDLType::PrincipalT => {}
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
