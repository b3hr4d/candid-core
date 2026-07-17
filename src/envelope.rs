use crate::{Contract, ContractValidationError, Limits};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
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
        let limits = &context.limits;
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
            .observe("extension_bytes", limits.max_value_bytes, bytes)
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContractEnvelope {
    contract: Contract,
    #[serde(default)]
    extensions: BTreeMap<String, Value>,
}

impl<'de> Deserialize<'de> for ContractEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawContractEnvelope::deserialize(deserializer)?;
        let envelope = Self {
            contract: raw.contract,
            extensions: raw.extensions,
        };
        envelope
            .validate(&Limits::default())
            .map_err(D::Error::custom)?;
        Ok(envelope)
    }
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
