use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileOptions {
    /// Preserve optional names, comments, raw source, and label spelling in a
    /// sidecar. This never changes the Contract or its identities.
    pub include_source_info: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            include_source_info: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation {
    pub(super) contract: Contract,
    pub(super) source_info: Option<SourceInfo>,
}

impl Compilation {
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn source_info(&self) -> Option<&SourceInfo> {
        self.source_info.as_ref()
    }

    pub fn into_parts(self) -> (Contract, Option<SourceInfo>) {
        (self.contract, self.source_info)
    }

    pub fn try_from_raw(
        raw_contract: RawContract,
        raw_source_info: Option<SerializedSourceInfo>,
        limits: &crate::Limits,
    ) -> Result<Self, crate::ContractValidationError> {
        let context = crate::RuntimeContext::new(limits.clone());
        Self::try_from_raw_with_context(raw_contract, raw_source_info, &context)
    }

    pub fn try_from_raw_with_context(
        raw_contract: RawContract,
        raw_source_info: Option<SerializedSourceInfo>,
        context: &crate::RuntimeContext,
    ) -> Result<Self, crate::ContractValidationError> {
        let mut budget = context.budget();
        Self::try_from_raw_with_budget(raw_contract, raw_source_info, &mut budget)
    }

    pub(crate) fn try_from_raw_with_budget(
        raw_contract: RawContract,
        raw_source_info: Option<SerializedSourceInfo>,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<Self, crate::ContractValidationError> {
        let (contract, mapping) = Contract::from_raw_with_mapping_and_budget(raw_contract, budget)?;
        let source_info = raw_source_info
            .map(SourceInfo::from_raw_unchecked)
            .map(|mut source_info| {
                remap_source_info(&mut source_info, &mapping, budget)?;
                source_info.validate_with_budget(&contract, budget)?;
                Ok::<SourceInfo, crate::ContractValidationError>(source_info)
            })
            .transpose()?;
        Ok(Self {
            contract,
            source_info,
        })
    }

    /// Parse, validate, and canonicalize a Compilation JSON document under
    /// caller-supplied limits.
    ///
    /// `max_input_bytes` is enforced before the document is decoded, so an
    /// oversized sidecar is rejected without being materialized.
    pub fn from_json_with_limits(
        input: &str,
        limits: &crate::Limits,
    ) -> Result<Self, crate::ContractJsonError> {
        Self::from_json_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    /// Bounded parse: the byte gate, decode, and validation share one budget.
    pub fn from_json_with_context(
        input: &str,
        context: &crate::RuntimeContext,
    ) -> Result<Self, crate::ContractJsonError> {
        let mut budget = context.budget();
        let raw: RawCompilation = crate::budget::decode_bounded(&mut budget, input.len(), || {
            serde_json::from_str(input)
        })?;
        Self::try_from_raw_with_budget(raw.contract, raw.source_info, &mut budget)
            .map_err(crate::ContractJsonError::InvalidContract)
    }

    /// Bounded parse from bytes.
    pub fn from_slice_with_limits(
        input: &[u8],
        limits: &crate::Limits,
    ) -> Result<Self, crate::ContractJsonError> {
        Self::from_slice_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    pub fn from_slice_with_context(
        input: &[u8],
        context: &crate::RuntimeContext,
    ) -> Result<Self, crate::ContractJsonError> {
        let mut budget = context.budget();
        let raw: RawCompilation = crate::budget::decode_bounded(&mut budget, input.len(), || {
            serde_json::from_slice(input)
        })?;
        Self::try_from_raw_with_budget(raw.contract, raw.source_info, &mut budget)
            .map_err(crate::ContractJsonError::InvalidContract)
    }

    /// Serialize validated canonical JSON under caller-supplied limits.
    ///
    /// Like [`Contract::to_json_pretty_with_limits`], this revalidates and
    /// charges the rendered length against `max_canonicalization_work`, so it
    /// may require raising that limit in addition to whichever structural
    /// limit gated construction.
    pub fn to_json_pretty_with_limits(
        &self,
        limits: &crate::Limits,
    ) -> Result<String, crate::ContractValidationError> {
        self.to_json_pretty_with_context(&crate::RuntimeContext::new(limits.clone()))
    }

    pub fn to_json_pretty_with_context(
        &self,
        context: &crate::RuntimeContext,
    ) -> Result<String, crate::ContractValidationError> {
        let mut budget = context.budget();
        // Validate without recanonicalizing, and render the stored artifact.
        // A Compilation's Contract is already canonical, and its sidecar's
        // type references are indexed against exactly that arena — rendering a
        // recanonicalized copy could desynchronize the two. The sidecar is not
        // revalidated here either: provenance authentication rederives the
        // whole bundle from source, which is construction-time work, not
        // serialization work.
        crate::validate::validate_contract_with_budget(&self.contract, &mut budget)?;
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        let json = serde_json::to_string_pretty(&CompilationRef {
            contract: &self.contract,
            source_info: &self.source_info,
        })
        .map_err(|error| {
            crate::ContractValidationError::single(
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

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct CompilationRef<'a> {
    contract: &'a Contract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_info: &'a Option<SourceInfo>,
}

impl Serialize for Compilation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        CompilationRef {
            contract: &self.contract,
            source_info: &self.source_info,
        }
        .serialize(serializer)
    }
}

/// Unvalidated Compilation data.
///
/// Kept private deliberately: [`Compilation::from_json_with_context`] is the
/// decode path, so this shape is not frozen as public surface. [`Compilation`]
/// itself does not implement [`Deserialize`] because a trait impl cannot
/// accept a resource policy:
///
/// ```compile_fail
/// let _: candid_core::Compilation = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompilation {
    contract: RawContract,
    #[serde(default)]
    source_info: Option<SerializedSourceInfo>,
}

fn remap_source_info(
    source_info: &mut SourceInfo,
    mapping: &[TypeRef],
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(), crate::ContractValidationError> {
    // Remapping walks attacker-sized collections before any validation stage
    // runs, so it must bound them itself rather than inherit later checks.
    crate::source::observe_remapped_collections(source_info, budget)?;
    let map = |reference: TypeRef| {
        mapping.get(reference as usize).copied().ok_or_else(|| {
            crate::ContractValidationError::single(
                "source_type_ref_out_of_bounds",
                "$",
                format!("source sidecar type reference {reference} is outside the input arena"),
            )
        })
    };
    for declaration in &mut source_info.declarations {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        declaration.ty = map(declaration.ty)?;
    }
    for field in &mut source_info.field_labels {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        field.container = map(field.container)?;
    }
    for method in &mut source_info.methods {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        method.service = map(method.service)?;
    }
    for argument in &mut source_info.function_arguments {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        argument.function = map(argument.function)?;
    }
    Ok(())
}
