use crate::canonical;
use crate::diagnostics::{CompileError, Diagnostic, DiagnosticPhase, SourceSpan};
use crate::limits::RuntimeContext;
use crate::model::{
    Actor, Contract, Declaration, Field, FieldLabelProvenance, MethodMode, PrimitiveType,
    RawContract, RawSourceInfo as SerializedSourceInfo, ServiceMethod, SourceActorInfo,
    SourceDeclaration, SourceFileInfo, SourceFunctionArgumentDirection, SourceFunctionArgumentInfo,
    SourceImportInfo, SourceImportKind, SourceInfo, SourceLabel, SourceMethodInfo, SourceOrigin,
    TypeNode, TypeRef, SOURCE_INFO_VERSION,
};
use candid_parser::candid::types::{FuncMode, Label, Type, TypeEnv, TypeInner};
#[cfg(feature = "filesystem-compiler")]
use candid_parser::check_file;
use candid_parser::check_prog;
use candid_parser::syntax::{Dec, IDLMergedProg, IDLProg, IDLType};
use candid_parser::token::{Token, Tokenizer};
use candid_parser::typing::ast_to_type;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(feature = "filesystem-compiler")]
use std::path::Path;

mod artifact;
/// The filesystem-free imported-bundle backend shared by `compile_with_resolver`
/// and provenance rederivation.
mod bundle;
mod diagnostics;
/// Holds the two compilation backends to each other: the promoted in-memory
/// `bundle` one and the native materialized/`check_file` one below.
#[cfg(all(test, feature = "filesystem-compiler"))]
mod differential;
mod loading;
mod lower;
/// Materialization writes the resolved bundle into a private temporary
/// directory so the official file checker can read it back, so it is
/// `filesystem-compiler` surface, not `compiler` surface.
#[cfg(feature = "filesystem-compiler")]
mod materialize;
mod nesting;

pub use artifact::{Compilation, CompileOptions};
use bundle::compile_resolved_bundle;
#[cfg(feature = "filesystem-compiler")]
use diagnostics::candid_file_error;
use diagnostics::{budget_error, candid_error, lower_error, source_info_compile_error};
use loading::{accept_source, load_source_units_with_resolver, SourceUnit};
use lower::lower_checked;
#[cfg(feature = "filesystem-compiler")]
use materialize::MaterializedBundle;
use nesting::{check_programs_type_depth, check_source_nesting};

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
    let mut budget = context.budget();
    accept_source("memory:/inline.did", source.len(), &mut budget)?;
    check_source_nesting(source, &mut budget)?;
    let program = parse_program(source, Some("memory:/inline.did".to_string()), &mut budget)?;
    check_programs_type_depth(std::iter::once(&program), &mut budget)?;
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
            "DID source contains imports; supply the bundle through a SourceResolver and compile it with compile_with_resolver, or use compile_did_file to read it from a native workspace",
        );
        error.diagnostics[0].notes = imports
            .into_iter()
            .map(|path| format!("import: {path}"))
            .collect();
        return Err(error);
    }

    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
    let mut environment = TypeEnv::new();
    let actor = check_prog(&mut environment, &program)
        .map_err(|error| candid_error(error, DiagnosticPhase::TypeCheck, None))?;
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
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
        &mut budget,
    )
}

/// Compile a DID file through `candid_parser::check_file`, including its
/// official filesystem import-resolution path.
///
/// Requires the `filesystem-compiler` feature. A host with no filesystem
/// compiles a self-contained source with [`compile_did`], or a logical source
/// bundle it already holds with [`compile_with_resolver`].
#[cfg(feature = "filesystem-compiler")]
pub fn compile_did_file(path: impl AsRef<Path>) -> Result<Compilation, CompileError> {
    compile_did_file_with_options(path, CompileOptions::default())
}

#[cfg(feature = "filesystem-compiler")]
pub fn compile_did_file_with_options(
    path: impl AsRef<Path>,
    options: CompileOptions,
) -> Result<Compilation, CompileError> {
    compile_did_file_with_context(path, options, &RuntimeContext::default())
}

#[cfg(feature = "filesystem-compiler")]
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
    compile_materialized_bundle(entry, &resolver, options, context)
}

/// The native materialized backend: resolve the bundle, write it into a
/// private temporary directory under numeric names, and hand the entry to
/// `candid_parser::check_file`.
///
/// [`compile_did_file`] deliberately keeps this path rather than routing
/// through the promoted in-memory [`compile_with_resolver`]. Native file
/// compilation keeps the official file checker as its authority, and with it
/// the filesystem diagnostic mapping (`Cannot open`/`Cannot import` classified
/// as [`DiagnosticPhase::Load`], numeric materialized names mapped back to
/// logical source IDs, rewritten offsets suppressed) and the source snapshot
/// the workspace resolver already took. Issue #21 promotes the in-memory
/// backend; it does not retire this one.
#[cfg(feature = "filesystem-compiler")]
fn compile_materialized_bundle(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    options: CompileOptions,
    context: &RuntimeContext,
) -> Result<Compilation, CompileError> {
    let mut budget = context.budget();
    let (source_units, entry_id) =
        load_source_units_with_resolver(entry, resolver, context, &mut budget)?;
    check_programs_type_depth(source_units.iter().map(|unit| &unit.program), &mut budget)?;
    let materialized = MaterializedBundle::new(&source_units, &entry_id, &mut budget)?;
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
    let (environment, actor, _) =
        check_file(&materialized.entry).map_err(|error| candid_file_error(error, &materialized))?;
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
    lower_checked(
        &source_units,
        &environment,
        actor.as_ref(),
        options,
        &mut budget,
    )
}

/// Compile an immutable logical source bundle with no filesystem access.
///
/// This is the platform primitive for imported bundles, and it requires only
/// the `compiler` feature. The resolver supplies every source as data; the
/// bundle is merged into one virtual program and type-checked in memory
/// through the official `candid_parser` merged-program APIs. Nothing is
/// materialized, no directory is opened, and no ambient authority is used, so
/// it works on `wasm32-unknown-unknown` — in a browser — where
/// `compile_did_file` and `WorkspaceResolver` need the `filesystem-compiler`
/// feature and, even with it, have no filesystem to reach.
///
/// [`crate::MemoryResolver`] covers a bundle the host already holds; a custom
/// [`crate::SourceResolver`] covers one the host can produce synchronously.
/// Resolution stays synchronous and host-supplied: this function never fetches
/// anything itself.
///
/// ```
/// use candid_core::{compile_with_resolver, CompileOptions, MemoryResolver, RuntimeContext};
///
/// let resolver = MemoryResolver::new()
///     .with_source("memory:/entry.did", "import \"types.did\";\nservice : { get: () -> (Item) query };")?
///     .with_source("memory:/types.did", "type Item = record { id: nat };")?;
/// let compilation = compile_with_resolver(
///     "memory:/entry.did",
///     &resolver,
///     CompileOptions::default(),
///     &RuntimeContext::default(),
/// )?;
/// assert_eq!(compilation.source_info().unwrap().sources().len(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn compile_with_resolver(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    options: CompileOptions,
    context: &RuntimeContext,
) -> Result<Compilation, CompileError> {
    let mut budget = context.budget();
    compile_resolved_bundle(entry, resolver, options, context, &mut budget)
}

/// Rederive a presented `SourceInfo`'s embedded bundle on the caller's budget.
///
/// Deliberately the same backend [`compile_with_resolver`] uses, with
/// provenance forced on: authentication compares a presented sidecar against
/// what compilation produces, so the two must be one code path rather than two
/// that agree today.
pub(crate) fn rederive_source_bundle_with_budget(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    context: &RuntimeContext,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<Compilation, CompileError> {
    compile_resolved_bundle(
        entry,
        resolver,
        CompileOptions {
            include_source_info: true,
        },
        context,
        budget,
    )
}

pub(super) fn parse_program(
    source: &str,
    source_name: Option<String>,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<IDLProg, CompileError> {
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::Parse, "Candid parsing"))?;
    let program = source
        .parse::<IDLProg>()
        .map_err(|error| candid_error(error, DiagnosticPhase::Parse, source_name))?;
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::Parse, "Candid parsing"))?;
    Ok(program)
}
