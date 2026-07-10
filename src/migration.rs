use crate::model::{
    Actor, Contract, ContractJsonError, ContractValidationError, Declaration, RawContract,
    TypeNode, CONTRACT_VERSION,
};
use crate::Limits;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyContractV1 {
    contract_version: u32,
    fingerprint: String,
    types: Vec<TypeNode>,
    #[serde(default)]
    declarations: Vec<Declaration>,
    #[serde(default)]
    actor: Option<Actor>,
}

/// Explicitly migrate the pre-profile Contract v1 JSON shape.
///
/// Ordinary [`Contract::from_json`] never invokes this migration implicitly.
pub fn migrate_legacy_v1_json(input: &str, limits: &Limits) -> Result<Contract, ContractJsonError> {
    if input.len() > limits.max_input_bytes {
        return Err(ContractJsonError::InvalidContract(
            ContractValidationError::resource_limit(
                "input_bytes",
                limits.max_input_bytes,
                input.len(),
            ),
        ));
    }
    let legacy: LegacyContractV1 = serde_json::from_str(input)
        .map_err(|error| ContractJsonError::MalformedJson(error.to_string()))?;
    if legacy.contract_version != CONTRACT_VERSION {
        return Err(ContractJsonError::InvalidContract(
            ContractValidationError::single(
                "unsupported_legacy_contract_version",
                "$.contract_version",
                format!(
                    "expected legacy Contract version {CONTRACT_VERSION}, found {}",
                    legacy.contract_version
                ),
            ),
        ));
    }
    let expected_legacy_fingerprint = legacy.fingerprint;
    let raw = RawContract::new(legacy.types, legacy.declarations, legacy.actor);
    let contract = Contract::build_raw(raw, limits).map_err(ContractJsonError::InvalidContract)?;
    if contract.fingerprint() != expected_legacy_fingerprint {
        return Err(ContractJsonError::InvalidContract(
            ContractValidationError::single(
                "legacy_fingerprint_mismatch",
                "$.fingerprint",
                format!(
                    "expected {}, found {expected_legacy_fingerprint}",
                    contract.fingerprint()
                ),
            ),
        ));
    }
    Ok(contract)
}
