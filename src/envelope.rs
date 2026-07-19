use crate::{Contract, ContractJsonError, ContractValidationError, Limits, RawContract};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A strict semantic Contract plus namespaced, non-semantic ecosystem metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractEnvelope {
    contract: Contract,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extensions: BTreeMap<String, Value>,
}

impl ContractEnvelope {
    pub fn new(contract: Contract) -> Self {
        Self {
            contract,
            extensions: BTreeMap::new(),
        }
    }

    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn extensions(&self) -> &BTreeMap<String, Value> {
        &self.extensions
    }

    pub fn insert_extension(
        &mut self,
        name: impl Into<String>,
        value: Value,
        limits: &Limits,
    ) -> Result<(), ContractValidationError> {
        self.insert_extension_with_context(name, value, &crate::RuntimeContext::new(limits.clone()))
    }

    pub fn insert_extension_with_context(
        &mut self,
        name: impl Into<String>,
        value: Value,
        context: &crate::RuntimeContext,
    ) -> Result<(), ContractValidationError> {
        let name = name.into();
        let previous = self.extensions.insert(name.clone(), value);
        if let Err(error) = self.validate_with_context(context) {
            match previous {
                Some(previous) => {
                    self.extensions.insert(name, previous);
                }
                None => {
                    self.extensions.remove(&name);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn validate(&self, limits: &Limits) -> Result<(), ContractValidationError> {
        self.validate_with_context(&crate::RuntimeContext::new(limits.clone()))
    }

    pub fn validate_with_context(
        &self,
        context: &crate::RuntimeContext,
    ) -> Result<(), ContractValidationError> {
        let mut budget = context.budget();
        crate::validate::validate_contract_with_budget(&self.contract, &mut budget)?;
        self.validate_extensions_with_budget(&mut budget)
    }

    /// Validate only the extension map, on a budget the caller already owns.
    ///
    /// Split out so a bounded parse can validate the Contract and the
    /// extensions on one budget instead of two independent allowances.
    pub(crate) fn validate_extensions_with_budget(
        &self,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<(), ContractValidationError> {
        let max_value_bytes = budget.limits().max_value_bytes;
        let mut bytes = 0usize;
        for (name, value) in &self.extensions {
            budget
                .checkpoint()
                .map_err(crate::budget::BudgetError::into_contract_error)?;
            if !valid_extension_name(name) {
                return Err(ContractValidationError::single(
                    "invalid_extension_name",
                    "$.extensions",
                    format!(
                        "extension {name:?} must be a reverse-domain name followed by /v<integer>"
                    ),
                ));
            }
            bytes = bytes
                .saturating_add(name.len())
                .saturating_add(serde_json::to_vec(value).map_or(usize::MAX, |value| value.len()));
        }
        budget
            .observe("extension_bytes", max_value_bytes, bytes)
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        Ok(())
    }

    /// Parse, validate, and canonicalize an envelope JSON document under
    /// caller-supplied limits.
    pub fn from_json_with_limits(input: &str, limits: &Limits) -> Result<Self, ContractJsonError> {
        Self::from_json_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    /// Bounded parse: `max_input_bytes` is enforced before decoding, and the
    /// nested Contract and the extension map are validated on one budget.
    pub fn from_json_with_context(
        input: &str,
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractJsonError> {
        let mut budget = context.budget();
        let raw: RawContractEnvelope =
            crate::budget::decode_bounded(&mut budget, input.len(), || {
                serde_json::from_str(input)
            })?;
        Self::from_raw_with_budget(raw, &mut budget)
    }

    /// Bounded parse from bytes.
    pub fn from_slice_with_limits(
        input: &[u8],
        limits: &Limits,
    ) -> Result<Self, ContractJsonError> {
        Self::from_slice_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    pub fn from_slice_with_context(
        input: &[u8],
        context: &crate::RuntimeContext,
    ) -> Result<Self, ContractJsonError> {
        let mut budget = context.budget();
        let raw: RawContractEnvelope =
            crate::budget::decode_bounded(&mut budget, input.len(), || {
                serde_json::from_slice(input)
            })?;
        Self::from_raw_with_budget(raw, &mut budget)
    }

    fn from_raw_with_budget(
        raw: RawContractEnvelope,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<Self, ContractJsonError> {
        let (contract, _) = Contract::from_raw_with_mapping_and_budget(raw.contract, budget)
            .map_err(ContractJsonError::InvalidContract)?;
        let envelope = Self {
            contract,
            extensions: raw.extensions,
        };
        envelope
            .validate_extensions_with_budget(budget)
            .map_err(ContractJsonError::InvalidContract)?;
        Ok(envelope)
    }

    /// Serialize the envelope under caller-supplied limits.
    pub fn to_json_pretty_with_limits(
        &self,
        limits: &Limits,
    ) -> Result<String, ContractValidationError> {
        self.to_json_pretty_with_context(&crate::RuntimeContext::new(limits.clone()))
    }

    /// Like [`Contract::to_json_pretty_with_context`], this revalidates and
    /// charges the rendered length against `max_canonicalization_work`.
    pub fn to_json_pretty_with_context(
        &self,
        context: &crate::RuntimeContext,
    ) -> Result<String, ContractValidationError> {
        let mut budget = context.budget();
        crate::validate::validate_contract_with_budget(&self.contract, &mut budget)?;
        self.validate_extensions_with_budget(&mut budget)?;
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        let json = serde_json::to_string_pretty(self).map_err(|error| {
            ContractValidationError::single(
                "contract_json_serialization_failed",
                "$",
                error.to_string(),
            )
        })?;
        let max_work = budget.limits().max_canonicalization_work;
        budget
            .charge("canonicalization_work", max_work, json.len())
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        Ok(json)
    }
}

/// Unvalidated envelope data.
///
/// `contract` is a [`RawContract`], not a [`Contract`]: the nested Contract is
/// validated on the envelope's own budget rather than on an independent
/// default-limited one. Kept private — [`ContractEnvelope::from_json_with_context`]
/// is the decode path, and the envelope carries no [`Deserialize`] impl:
///
/// ```compile_fail
/// let _: candid_core::ContractEnvelope = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContractEnvelope {
    contract: RawContract,
    #[serde(default)]
    extensions: BTreeMap<String, Value>,
}

fn valid_extension_name(name: &str) -> bool {
    let Some((namespace, version)) = name.rsplit_once("/v") else {
        return false;
    };
    namespace.contains('.')
        && namespace
            .split('.')
            .all(|segment| !segment.is_empty() && segment.bytes().all(is_name_byte))
        && !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_digit())
        && !version.starts_with('0')
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
}
