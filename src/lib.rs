//! A canonical, validated Candid Contract graph.
//!
//! This crate deliberately delegates DID parsing and semantic checking to the
//! official `candid_parser` engine. It projects the checked result into a
//! host-neutral JSON model; it does not implement a second Candid parser or
//! codec.
//!
//! # Feature surfaces
//!
//! One package, four build surfaces. Every feature is enabled by default, so
//! an existing `candid-core = "0.1"` dependency is unchanged.
//!
//! | Feature | Adds | Dependencies it pulls in |
//! | --- | --- | --- |
//! | *(base)* `default-features = false` | `Contract`, `ContractDraft`, `RawContract`, validation, canonicalization, identities, `Limits`/`RuntimeContext`, `Diagnostic`, `ContractEnvelope` | `serde`, `serde_json`, `sha2`, `hex` |
//! | `host-value` | `HostValue` and graph-directed value validation | `ic_principal` |
//! | `compiler` | `compile_did`, `Compilation`, `SourceId`/`SourceResolver`/`MemoryResolver`, `SourceInfo` provenance | `candid`, `candid_parser` |
//! | `filesystem-compiler` (implies `compiler`) | `WorkspaceResolver`, `compile_did_file`, `compile_with_resolver`, the `candid-core` binary | `cap-std` |
//!
//! Items outside the enabled set are absent at compile time rather than
//! present as failing stubs, so a build error names the feature to turn on.
//!
//! Cargo unifies features across a dependency graph: if anything in a build
//! also depends on `candid-core` with defaults, the whole surface is compiled
//! once for every consumer in that build. Feature selection bounds what a
//! given dependency graph *must* contain, not what a mixed graph produces.

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

// `bounded` reads a capped byte count off a `std::io::Read`; only the native
// filesystem resolver uses it, and only where a real filesystem exists.
#[cfg(all(feature = "filesystem-compiler", not(target_os = "unknown")))]
mod bounded;
mod budget;
mod canonical;
#[cfg(feature = "compiler")]
mod compile;
mod diagnostics;
mod envelope;
mod limits;
mod model;
mod name_hash;
#[cfg(feature = "compiler")]
mod resolver;
#[cfg(feature = "compiler")]
mod source;
mod validate;
#[cfg(feature = "host-value")]
mod value;

#[cfg(feature = "compiler")]
pub use compile::{
    compile_did, compile_did_with_context, compile_did_with_options, Compilation, CompileOptions,
};
#[cfg(feature = "filesystem-compiler")]
pub use compile::{
    compile_did_file, compile_did_file_with_context, compile_did_file_with_options,
    compile_with_resolver,
};
#[cfg(feature = "compiler")]
pub use diagnostics::CompileError;
pub use diagnostics::{
    Diagnostic, DiagnosticPhase, RelatedLocation, ResourceLimitInfo, Severity, SourceSpan,
};
pub use envelope::ContractEnvelope;
pub use limits::{
    CancellationToken, Limits, LimitsConfig, LimitsConfigError, LimitsProfile, RuntimeContext,
    LIMITS_CONFIG_VERSION,
};
pub use model::{
    Actor, Contract, ContractDraft, ContractIdentities, ContractJsonError, ContractValidationError,
    ContractViolation, Declaration, Field, MethodMode, PrimitiveType, ProducerInfo, RawContract,
    ServiceMethod, TypeNode, TypeRef, CANONICALIZATION_PROFILE, CONTRACT_FORMAT, FORMAT_VERSION,
    SEMANTICS_PROFILE,
};
// Provenance is compiler surface: a presented sidecar is authenticated by
// recompiling its embedded bundle, which is compiler logic.
#[cfg(feature = "compiler")]
pub use model::{
    FieldLabelProvenance, RawSourceInfo, SourceActorInfo, SourceDeclaration, SourceFileInfo,
    SourceFunctionArgumentDirection, SourceFunctionArgumentInfo, SourceImportInfo,
    SourceImportKind, SourceInfo, SourceLabel, SourceMethodInfo, SourceOrigin, SOURCE_INFO_VERSION,
};
#[cfg(feature = "filesystem-compiler")]
pub use resolver::WorkspaceResolver;
#[cfg(feature = "compiler")]
pub use resolver::{MemoryResolver, ResolveError, ResolvedSource, SourceId, SourceResolver};
#[cfg(feature = "host-value")]
pub use value::{
    validate_host_value, validate_host_value_with_context, ContractMethodRef, ContractTypeRef,
    HostFieldValue, HostValue, HostValueJsonError, HostValueValidationError, HostValueViolation,
};

// A disabled feature must remove its API, not replace it with something that
// compiles and then fails at run time. `tests/model_public_api.rs` proves each
// surface is *present* when its feature is on; these doctests are the other
// direction, and they only exist in the configurations where the surface
// should be gone.
#[cfg(not(feature = "compiler"))]
mod compiler_surface_is_absent_without_its_feature {
    //! ```compile_fail
    //! let _ = candid_core::compile_did("service : {};");
    //! ```
    //!
    //! ```compile_fail
    //! let _: candid_core::MemoryResolver = candid_core::MemoryResolver::new();
    //! ```
    //!
    //! ```compile_fail
    //! fn takes(_: candid_core::SourceInfo) {}
    //! ```
}

#[cfg(all(feature = "compiler", not(feature = "filesystem-compiler")))]
mod filesystem_surface_is_absent_without_its_feature {
    //! ```compile_fail
    //! let _ = candid_core::compile_did_file("service.did");
    //! ```
    //!
    //! ```compile_fail
    //! let _ = candid_core::WorkspaceResolver::new(".");
    //! ```
}

#[cfg(not(feature = "host-value"))]
mod host_value_surface_is_absent_without_its_feature {
    //! ```compile_fail
    //! let _ = candid_core::HostValue::null();
    //! ```
    //!
    //! ```compile_fail
    //! fn takes(_: candid_core::HostValueValidationError) {}
    //! ```
}
