use candid_core::{
    Actor, Contract, ContractIdentities, ContractJsonError, ContractValidationError,
    ContractViolation, Declaration, Field, FieldLabelProvenance, MethodMode, PrimitiveType,
    ProducerInfo, RawContract, RawSourceInfo, ServiceMethod, SourceActorInfo, SourceDeclaration,
    SourceFileInfo, SourceFunctionArgumentDirection, SourceFunctionArgumentInfo, SourceImportInfo,
    SourceImportKind, SourceInfo, SourceLabel, SourceMethodInfo, SourceOrigin, TypeNode, TypeRef,
    CANONICALIZATION_PROFILE, CONTRACT_FORMAT, FORMAT_VERSION, SEMANTICS_PROFILE,
    SOURCE_INFO_VERSION,
};

fn assert_public_type<T: 'static>() {}

#[test]
fn model_api_remains_available_at_the_crate_root() {
    assert_public_type::<Actor>();
    assert_public_type::<Contract>();
    assert_public_type::<ContractIdentities>();
    assert_public_type::<ContractJsonError>();
    assert_public_type::<ContractValidationError>();
    assert_public_type::<ContractViolation>();
    assert_public_type::<Declaration>();
    assert_public_type::<Field>();
    assert_public_type::<FieldLabelProvenance>();
    assert_public_type::<MethodMode>();
    assert_public_type::<PrimitiveType>();
    assert_public_type::<ProducerInfo>();
    assert_public_type::<RawContract>();
    assert_public_type::<RawSourceInfo>();
    assert_public_type::<ServiceMethod>();
    assert_public_type::<SourceActorInfo>();
    assert_public_type::<SourceDeclaration>();
    assert_public_type::<SourceFileInfo>();
    assert_public_type::<SourceFunctionArgumentDirection>();
    assert_public_type::<SourceFunctionArgumentInfo>();
    assert_public_type::<SourceImportInfo>();
    assert_public_type::<SourceImportKind>();
    assert_public_type::<SourceInfo>();
    assert_public_type::<SourceLabel>();
    assert_public_type::<SourceMethodInfo>();
    assert_public_type::<SourceOrigin>();
    assert_public_type::<TypeNode>();
    assert_public_type::<TypeRef>();

    assert_eq!(CONTRACT_FORMAT, "candid-core");
    assert_eq!(FORMAT_VERSION, 1);
    assert_eq!(SEMANTICS_PROFILE, "candid-1");
    assert_eq!(CANONICALIZATION_PROFILE, "candid-core-canon-1");
    assert_eq!(SOURCE_INFO_VERSION, 1);
}
