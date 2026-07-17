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
        let (contract, mapping) = Contract::from_raw_with_mapping(raw_contract, limits)?;
        let source_info = raw_source_info
            .map(SourceInfo::from_raw_unchecked)
            .map(|mut source_info| {
                remap_source_info(&mut source_info, &mapping)?;
                source_info.validate(&contract, limits)?;
                Ok::<SourceInfo, crate::ContractValidationError>(source_info)
            })
            .transpose()?;
        Ok(Self {
            contract,
            source_info,
        })
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompilation {
    contract: RawContract,
    #[serde(default)]
    source_info: Option<SerializedSourceInfo>,
}

impl<'de> Deserialize<'de> for Compilation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawCompilation::deserialize(deserializer)?;
        let limits = crate::Limits::default();
        Self::try_from_raw(raw.contract, raw.source_info, &limits).map_err(D::Error::custom)
    }
}

impl TryFrom<(RawContract, Option<SerializedSourceInfo>)> for Compilation {
    type Error = crate::ContractValidationError;

    fn try_from(
        (contract, source_info): (RawContract, Option<SerializedSourceInfo>),
    ) -> Result<Self, Self::Error> {
        Self::try_from_raw(contract, source_info, &crate::Limits::default())
    }
}

fn remap_source_info(
    source_info: &mut SourceInfo,
    mapping: &[TypeRef],
) -> Result<(), crate::ContractValidationError> {
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
        declaration.ty = map(declaration.ty)?;
    }
    for field in &mut source_info.field_labels {
        field.container = map(field.container)?;
    }
    for method in &mut source_info.methods {
        method.service = map(method.service)?;
    }
    for argument in &mut source_info.function_arguments {
        argument.function = map(argument.function)?;
    }
    Ok(())
}
