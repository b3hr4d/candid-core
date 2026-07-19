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
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

mod artifact;
mod diagnostics;
mod loading;
mod lower;
mod materialize;
mod nesting;

pub use artifact::{Compilation, CompileOptions};
use diagnostics::{
    budget_error, candid_error, candid_file_error, lower_error, source_info_compile_error,
};
use loading::{accept_source, load_source_units_with_resolver, SourceUnit};
use lower::lower_checked;
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
            "DID source contains imports; compile it with compile_did_file so candid_parser can resolve them",
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
    let mut budget = context.budget();
    let (source_units, entry_id) =
        load_source_units_with_resolver(entry, resolver, context, &mut budget)?;
    check_programs_type_depth(source_units.iter().map(|unit| &unit.program), &mut budget)?;
    let materialized = MaterializedBundle::new(&source_units, &entry_id, &mut budget)?;
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
    let (environment, actor, _) = check_file(&materialized.entry).map_err(candid_file_error)?;
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

pub(crate) fn rederive_source_bundle_with_budget(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    context: &RuntimeContext,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<Compilation, CompileError> {
    let (source_units, entry_id) =
        load_source_units_with_resolver(entry, resolver, context, budget)?;
    check_programs_type_depth(source_units.iter().map(|unit| &unit.program), budget)?;
    let entry_unit = source_units
        .iter()
        .find(|unit| unit.name == entry_id.as_str())
        .ok_or_else(|| {
            CompileError::single(
                "did_source_not_found",
                DiagnosticPhase::Load,
                "entry source is missing from the resolved bundle",
            )
        })?;
    let entry_program = parse_program(&entry_unit.source, Some(entry_unit.name.clone()), budget)?;
    let mut merged = IDLMergedProg::new(entry_program);
    for unit in source_units
        .iter()
        .filter(|unit| unit.name != entry_id.as_str())
    {
        let program = parse_program(&unit.source, Some(unit.name.clone()), budget)?;
        merged
            .merge(unit.include_actor, unit.name.clone(), program)
            .map_err(|error| {
                candid_error(
                    candid_parser::Error::Custom(error),
                    DiagnosticPhase::TypeCheck,
                    None,
                )
            })?;
    }
    let program = IDLProg {
        decs: merged.decs(),
        actor: merged.resolve_actor().map_err(|error| {
            candid_error(
                candid_parser::Error::Custom(error),
                DiagnosticPhase::TypeCheck,
                None,
            )
        })?,
    };
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
    let mut environment = TypeEnv::new();
    let actor = check_prog(&mut environment, &program)
        .map_err(|error| candid_error(error, DiagnosticPhase::TypeCheck, None))?;
    budget
        .checkpoint()
        .map_err(|error| budget_error(error, DiagnosticPhase::TypeCheck, "Candid type checking"))?;
    lower_checked(
        &source_units,
        &environment,
        actor.as_ref(),
        CompileOptions {
            include_source_info: true,
        },
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
