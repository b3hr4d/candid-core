//! A canonical, validated Candid Contract graph.
//!
//! This crate deliberately delegates DID parsing and semantic checking to the
//! official `candid_parser` engine. It projects the checked result into a
//! host-neutral JSON model; it does not implement a second Candid parser or
//! codec.

mod canonical;
mod compile;
mod diagnostics;
mod model;
mod validate;

pub use compile::{
    compile_did, compile_did_file, compile_did_file_with_options, compile_did_with_options,
    Compilation, CompileOptions,
};
pub use diagnostics::{CompileError, Diagnostic, DiagnosticPhase, Severity, SourceSpan};
pub use model::{
    Actor, Contract, ContractJsonError, ContractValidationError, ContractViolation, Declaration,
    Field, FieldLabelProvenance, MethodMode, PrimitiveType, ServiceMethod, SourceActorInfo,
    SourceDeclaration, SourceFileInfo, SourceFunctionArgumentDirection, SourceFunctionArgumentInfo,
    SourceInfo, SourceLabel, SourceMethodInfo, SourceOrigin, TypeNode, TypeRef, CONTRACT_VERSION,
    SOURCE_INFO_VERSION,
};
