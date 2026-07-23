//! A canonical, validated Candid Contract graph.
//!
//! This crate deliberately delegates DID parsing and semantic checking to the
//! official `candid_parser` engine. It projects the checked result into a
//! host-neutral JSON model; it does not implement a second Candid parser or
//! codec.

// Two invariants bracket the supported pointer width. Portable wire values
// (diagnostic counts, span offsets, limit overrides) are fixed-width `u64`,
// and the crate widens `usize` counters into them with plain casts that are
// exact only while `usize` fits in 64 bits (so no target wider than 64 bits).
// The `InteractiveV1` default limit values exceed a 16-bit `usize` (so no
// target narrower than 32 bits). That leaves 32- and 64-bit targets — which
// covers every std platform, `wasm32` included. Refuse to compile elsewhere
// with a clear message rather than silently truncating or overflowing a
// literal.
#[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
compile_error!("candid-core supports only 32- and 64-bit targets: portable u64 wire values must represent every usize exactly (usize must not exceed 64 bits), and the InteractiveV1 default limits require a usize of at least 32 bits");

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
pub use limits::{
    CancellationToken, Limits, LimitsConfig, LimitsConfigError, LimitsProfile, RuntimeContext,
    LIMITS_CONFIG_VERSION,
};
pub use model::{
    Actor, Contract, ContractDraft, ContractIdentities, ContractJsonError, ContractValidationError,
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
