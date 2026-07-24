//! Pins the crate root's public surface, per feature.
//!
//! This file compiles under every supported feature combination, including
//! `--no-default-features`. Each block names exactly the items its feature is
//! responsible for exporting, so moving an item across a feature boundary — in
//! either direction — fails to compile here rather than silently changing what
//! a downstream `default-features = false` consumer can reach.

use candid_core::{
    Actor, CancellationToken, Contract, ContractDraft, ContractEnvelope, ContractIdentities,
    ContractJsonError, ContractValidationError, ContractViolation, Declaration, Diagnostic,
    DiagnosticPhase, Field, Limits, LimitsConfig, LimitsConfigError, LimitsProfile, MethodMode,
    PrimitiveType, ProducerInfo, RawContract, RelatedLocation, ResourceLimitInfo, RuntimeContext,
    ServiceMethod, Severity, SourceSpan, TypeNode, TypeRef, CANONICALIZATION_PROFILE,
    CONTRACT_FORMAT, FORMAT_VERSION, LIMITS_CONFIG_VERSION, SEMANTICS_PROFILE,
};

fn assert_public_type<T: 'static>() {}

/// The base surface: everything a `default-features = false` consumer gets.
#[test]
fn model_api_remains_available_at_the_crate_root() {
    assert_public_type::<Actor>();
    assert_public_type::<CancellationToken>();
    assert_public_type::<Contract>();
    assert_public_type::<ContractDraft>();
    assert_public_type::<ContractEnvelope>();
    assert_public_type::<ContractIdentities>();
    assert_public_type::<ContractJsonError>();
    assert_public_type::<ContractValidationError>();
    assert_public_type::<ContractViolation>();
    assert_public_type::<Declaration>();
    assert_public_type::<Diagnostic>();
    assert_public_type::<DiagnosticPhase>();
    assert_public_type::<Field>();
    assert_public_type::<Limits>();
    assert_public_type::<LimitsConfig>();
    assert_public_type::<LimitsConfigError>();
    assert_public_type::<LimitsProfile>();
    assert_public_type::<MethodMode>();
    assert_public_type::<PrimitiveType>();
    assert_public_type::<ProducerInfo>();
    assert_public_type::<RawContract>();
    assert_public_type::<RelatedLocation>();
    assert_public_type::<ResourceLimitInfo>();
    assert_public_type::<RuntimeContext>();
    assert_public_type::<ServiceMethod>();
    assert_public_type::<Severity>();
    assert_public_type::<SourceSpan>();
    assert_public_type::<TypeNode>();
    assert_public_type::<TypeRef>();

    assert_eq!(CONTRACT_FORMAT, "candid-core");
    assert_eq!(FORMAT_VERSION, 1);
    assert_eq!(SEMANTICS_PROFILE, "candid-1");
    assert_eq!(CANONICALIZATION_PROFILE, "candid-core-canon-1");
    assert_eq!(LIMITS_CONFIG_VERSION, 1);
}

/// Producer metadata is identical in every configuration: `candid` and
/// `candid_parser` are optional dependencies, but the versions reported here
/// come from this package's manifest, not from a linked crate.
#[test]
fn producer_metadata_is_feature_independent() {
    let producer = ProducerInfo::current();
    assert_eq!(producer.name, "candid-core");
    assert_eq!(producer.version, env!("CARGO_PKG_VERSION"));
    assert!(
        !producer.candid_version.is_empty() && !producer.candid_parser_version.is_empty(),
        "producer must report both engine versions with defaults disabled"
    );
}

/// The `compiler` surface: source compilation, logical source resolution, and
/// the provenance sidecar.
#[cfg(feature = "compiler")]
mod compiler_surface {
    use super::assert_public_type;
    use candid_core::{
        Compilation, CompileError, CompileOptions, FieldLabelProvenance, MemoryResolver,
        RawSourceInfo, ResolveError, ResolvedSource, RuntimeContext, SourceActorInfo,
        SourceDeclaration, SourceFileInfo, SourceFunctionArgumentDirection,
        SourceFunctionArgumentInfo, SourceId, SourceImportInfo, SourceImportKind, SourceInfo,
        SourceLabel, SourceMethodInfo, SourceOrigin, SourceResolver, SOURCE_INFO_VERSION,
    };

    #[test]
    fn compiler_api_remains_available_at_the_crate_root() {
        assert_public_type::<Compilation>();
        assert_public_type::<CompileError>();
        assert_public_type::<CompileOptions>();
        assert_public_type::<FieldLabelProvenance>();
        assert_public_type::<MemoryResolver>();
        assert_public_type::<RawSourceInfo>();
        assert_public_type::<ResolveError>();
        assert_public_type::<ResolvedSource>();
        assert_public_type::<SourceActorInfo>();
        assert_public_type::<SourceDeclaration>();
        assert_public_type::<SourceFileInfo>();
        assert_public_type::<SourceFunctionArgumentDirection>();
        assert_public_type::<SourceFunctionArgumentInfo>();
        assert_public_type::<SourceId>();
        assert_public_type::<SourceImportInfo>();
        assert_public_type::<SourceImportKind>();
        assert_public_type::<SourceInfo>();
        assert_public_type::<SourceLabel>();
        assert_public_type::<SourceMethodInfo>();
        assert_public_type::<SourceOrigin>();
        assert_public_type::<Box<dyn SourceResolver>>();

        assert_eq!(SOURCE_INFO_VERSION, 1);
    }

    /// Signatures, not just names: coercing each entry point to a function
    /// pointer pins its argument and return types too.
    #[test]
    fn compiler_entry_point_signatures_are_unchanged() {
        let _: fn(&str) -> Result<Compilation, CompileError> = candid_core::compile_did;
        let _: fn(&str, CompileOptions) -> Result<Compilation, CompileError> =
            candid_core::compile_did_with_options;
        let _: fn(&str, CompileOptions, &RuntimeContext) -> Result<Compilation, CompileError> =
            candid_core::compile_did_with_context;
    }
}

/// The `filesystem-compiler` surface: the native-only additions.
#[cfg(feature = "filesystem-compiler")]
mod filesystem_compiler_surface {
    use super::assert_public_type;
    use candid_core::{
        Compilation, CompileError, CompileOptions, RuntimeContext, SourceResolver,
        WorkspaceResolver,
    };
    use std::path::Path;

    #[test]
    fn filesystem_compiler_api_remains_available_at_the_crate_root() {
        assert_public_type::<WorkspaceResolver>();
    }

    #[test]
    fn filesystem_entry_point_signatures_are_unchanged() {
        // `compile_did_file` takes `impl AsRef<Path>`, which cannot be named
        // by turbofish; the closure coercion pins one concrete instantiation
        // plus the exact return type.
        let _: fn(&Path) -> Result<Compilation, CompileError> =
            |path| candid_core::compile_did_file(path);
        let _: fn(&Path, CompileOptions) -> Result<Compilation, CompileError> =
            |path, options| candid_core::compile_did_file_with_options(path, options);
        let _: fn(&Path, CompileOptions, &RuntimeContext) -> Result<Compilation, CompileError> =
            |path, options, context| {
                candid_core::compile_did_file_with_context(path, options, context)
            };
        let _: fn(
            &str,
            &dyn SourceResolver,
            CompileOptions,
            &RuntimeContext,
        ) -> Result<Compilation, CompileError> = candid_core::compile_with_resolver;
    }
}

/// The `host-value` surface.
#[cfg(feature = "host-value")]
mod host_value_surface {
    use super::assert_public_type;
    use candid_core::{
        Contract, ContractMethodRef, ContractTypeRef, HostFieldValue, HostValue,
        HostValueJsonError, HostValueValidationError, HostValueViolation, RuntimeContext,
    };

    #[test]
    fn host_value_api_remains_available_at_the_crate_root() {
        assert_public_type::<ContractMethodRef>();
        assert_public_type::<ContractTypeRef>();
        assert_public_type::<HostFieldValue>();
        assert_public_type::<HostValue>();
        assert_public_type::<HostValueJsonError>();
        assert_public_type::<HostValueValidationError>();
        assert_public_type::<HostValueViolation>();
    }

    #[test]
    fn host_value_entry_point_signatures_are_unchanged() {
        let _: fn(
            &Contract,
            &ContractTypeRef,
            &HostValue,
            &candid_core::Limits,
        ) -> Result<(), HostValueValidationError> = candid_core::validate_host_value;
        let _: fn(
            &Contract,
            &ContractTypeRef,
            &HostValue,
            &RuntimeContext,
        ) -> Result<(), HostValueValidationError> = candid_core::validate_host_value_with_context;
    }
}
