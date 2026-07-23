//! A canonical, validated Candid Contract graph.
//!
//! This crate deliberately delegates DID parsing and semantic checking to the
//! official `candid_parser` engine. It projects the checked result into a
//! host-neutral JSON model; it does not implement a second Candid parser or
//! codec.

#[cfg(not(target_os = "unknown"))]
mod bounded;
mod budget;
mod canonical;
mod compile;
mod diagnostics;
mod envelope;
mod limits;
mod model;
mod resolver;
mod source;
mod validate;
mod value;

pub use compile::{
    compile_did, compile_did_file, compile_did_file_with_context, compile_did_file_with_options,
    compile_did_with_context, compile_did_with_options, compile_with_resolver, Compilation,
    CompileOptions,
};
pub use diagnostics::{
    CompileError, Diagnostic, DiagnosticPhase, RelatedLocation, ResourceLimitInfo, Severity,
    SourceSpan,
};
pub use envelope::ContractEnvelope;
pub use limits::{CancellationToken, Limits, RuntimeContext};
pub use model::{
    Actor, Contract, ContractIdentities, ContractJsonError, ContractValidationError,
    ContractViolation, Declaration, Field, FieldLabelProvenance, MethodMode, PrimitiveType,
    ProducerInfo, RawContract, RawSourceInfo, ServiceMethod, SourceActorInfo, SourceDeclaration,
    SourceFileInfo, SourceFunctionArgumentDirection, SourceFunctionArgumentInfo, SourceImportInfo,
    SourceImportKind, SourceInfo, SourceLabel, SourceMethodInfo, SourceOrigin, TypeNode, TypeRef,
    CANONICALIZATION_PROFILE, CONTRACT_FORMAT, FORMAT_VERSION, SEMANTICS_PROFILE,
    SOURCE_INFO_VERSION,
};
pub use resolver::{
    MemoryResolver, ResolveError, ResolvedSource, SourceId, SourceResolver, WorkspaceResolver,
};
pub use value::{
    validate_host_value, validate_host_value_with_context, ContractMethodRef, ContractTypeRef,
    HostFieldValue, HostValue, HostValueJsonError, HostValueValidationError, HostValueViolation,
};
