//! The filesystem-free imported-bundle backend.
//!
//! One resolver load, one merge through the official `candid_parser`
//! merged-program APIs, one `check_prog`, one lowering. The promoted public
//! [`super::compile_with_resolver`] entry point and the `SourceInfo`
//! authentication path both call [`compile_resolved_bundle`], so browser-side
//! compilation and provenance rederivation cannot drift apart: there is one
//! implementation, not two that must be kept in agreement.
//!
//! Nothing here touches a filesystem, materializes a source, or needs a
//! capability crate, which is why this module — and with it
//! `compile_with_resolver` — is `compiler` surface rather than
//! `filesystem-compiler` surface.

use super::*;

/// Load, bound, check, and lower one immutable logical source bundle.
///
/// The caller owns the budget so a nested operation (provenance
/// authentication) charges the same counters as the operation that contains
/// it, instead of starting from a fresh allowance.
pub(super) fn compile_resolved_bundle(
    entry: &str,
    resolver: &dyn crate::SourceResolver,
    options: CompileOptions,
    context: &RuntimeContext,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<Compilation, CompileError> {
    let (source_units, entry_id) =
        load_source_units_with_resolver(entry, resolver, context, budget)?;
    check_programs_type_depth(source_units.iter().map(|unit| &unit.program), budget)?;
    let (environment, actor) = check_source_units(&source_units, &entry_id, budget)?;
    lower_checked(&source_units, &environment, actor.as_ref(), options, budget)
}

/// Merge an already-loaded bundle into one virtual program and type-check it.
///
/// This reproduces `candid_parser::check_file`'s own tail — merge every
/// import into an [`IDLMergedProg`], resolve the actor, then `check_decs` plus
/// `check_actor`, which is exactly what [`check_prog`] performs — over sources
/// the resolver already supplied as data. The loader has already decided
/// import kind and actor inclusion (`import` versus `import service`, with a
/// repeated target keeping actor inclusion if *any* edge to it was a service
/// import), so this pass only replays that decision.
fn check_source_units(
    source_units: &[SourceUnit],
    entry_id: &crate::SourceId,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(TypeEnv, Option<Type>), CompileError> {
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
    budget.checkpoint().map_err(|error| {
        budget_error(error, DiagnosticPhase::TypeCheck, "Candid import merging")
    })?;
    let mut merged = IDLMergedProg::new(clone_program(&entry_unit.program));
    for unit in source_units
        .iter()
        .filter(|unit| unit.name != entry_id.as_str())
    {
        budget.checkpoint().map_err(|error| {
            budget_error(error, DiagnosticPhase::TypeCheck, "Candid import merging")
        })?;
        merged
            .merge(
                unit.include_actor,
                unit.name.clone(),
                clone_program(&unit.program),
            )
            // `merge` fails for exactly one reason: this unit was reached by an
            // `import service` edge and declares no main service. The failing
            // source is known here without inspecting the message, so it is
            // named as a source-scoped location — the location the native
            // materialized path can only recover by mapping numeric file names
            // back afterwards. No offsets: the report carries none.
            .map_err(|error| {
                merged_program_error(error, Some(SourceSpan::source_only(&unit.name)))
            })?;
    }
    let actor = merged.resolve_actor().map_err(|error| {
        let span = service_import_span(&error.to_string(), source_units, entry_id);
        merged_program_error(error, span)
    })?;
    let program = IDLProg {
        decs: merged.decs(),
        actor,
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
    Ok((environment, actor))
}

/// Convert a merge or actor-resolution failure, optionally scoping it to one
/// logical source.
///
/// These errors carry no report labels — `candid_parser::Error::report` gives
/// `Custom` and `CandidError` a message and nothing else — so there are no
/// offsets to publish or suppress, and the span is exactly the source scope
/// the caller could prove.
fn merged_program_error(
    error: impl Into<candid_parser::Error>,
    span: Option<SourceSpan>,
) -> CompileError {
    let mut compile_error = candid_error(error.into(), DiagnosticPhase::TypeCheck, None);
    if let Some(span) = span {
        for diagnostic in &mut compile_error.diagnostics {
            diagnostic.span = Some(span.clone());
        }
    }
    compile_error
}

/// Scope an actor-resolution failure to one logical source when upstream's own
/// message identifies it unambiguously.
///
/// `IDLMergedProg::resolve_actor` names a source in exactly one template at
/// this pin (`candid_parser` 0.4.0, `syntax/mod.rs`): `Imported service file
/// "{name}" has a service constructor`, where `{name}` is the merge name —
/// this compiler's logical source ID. The comparison is against the *whole*
/// message and against a name the compiler itself supplied, so no rendered
/// user text can produce a match and no span is fabricated. Every other
/// resolve failure (`Unbound type identifier …`, `not a service type: …`,
/// `Duplicate imported method name: …`) names no single source and gets none.
fn service_import_span(
    message: &str,
    source_units: &[SourceUnit],
    entry_id: &crate::SourceId,
) -> Option<SourceSpan> {
    source_units
        .iter()
        .filter(|unit| unit.include_actor && unit.name != entry_id.as_str())
        .find(|unit| {
            message
                == format!(
                    "Imported service file \"{}\" has a service constructor",
                    unit.name
                )
        })
        .map(|unit| SourceSpan::source_only(&unit.name))
}

/// Rebuild an owned `IDLProg` from the one the loader already parsed.
///
/// `IDLMergedProg` consumes programs by value and upstream's `IDLProg` is not
/// `Clone`, but every part it is built from is. Cloning the parsed tree is
/// what keeps this backend at exactly one parse per source: re-parsing — what
/// the provenance path used to do — repeats tokenizing work the loader has
/// already performed and can only ever reproduce this same tree.
fn clone_program(program: &IDLProg) -> IDLProg {
    IDLProg {
        decs: program.decs.iter().map(clone_declaration).collect(),
        actor: program.actor.clone(),
    }
}

fn clone_declaration(declaration: &Dec) -> Dec {
    match declaration {
        Dec::TypD(binding) => Dec::TypD(binding.clone()),
        Dec::ImportType(import) => Dec::ImportType(import.clone()),
        Dec::ImportServ(import) => Dec::ImportServ(import.clone()),
    }
}
