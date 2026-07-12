use crate::canonical::domain_hash;
use crate::limits::Limits;
use crate::model::{
    Contract, ContractValidationError, SourceImportInfo, SourceInfo, SourceOrigin, TypeNode,
    SOURCE_INFO_VERSION,
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize)]
struct SourceBundlePayload<'a> {
    sources: &'a [crate::model::SourceFileInfo],
    imports: &'a [SourceImportInfo],
}

pub(crate) fn source_bundle_id(
    sources: &[crate::model::SourceFileInfo],
    imports: &[SourceImportInfo],
) -> String {
    domain_hash(
        "candid-core:source-bundle:v1",
        &SourceBundlePayload { sources, imports },
    )
}

pub(crate) fn validate_source_info(
    source_info: &SourceInfo,
    contract: &Contract,
    limits: &Limits,
) -> Result<(), ContractValidationError> {
    if source_info.source_info_version != SOURCE_INFO_VERSION {
        return Err(ContractValidationError::single(
            "unsupported_source_info_version",
            "$.source_info_version",
            format!(
                "expected {SOURCE_INFO_VERSION}, found {}",
                source_info.source_info_version
            ),
        ));
    }
    if source_info.contract_id != contract.contract_id() {
        return Err(ContractValidationError::single(
            "source_contract_id_mismatch",
            "$.contract_id",
            format!(
                "expected {}, found {}",
                contract.contract_id(),
                source_info.contract_id
            ),
        ));
    }
    if source_info.sources.len() > limits.max_sources {
        return Err(ContractValidationError::resource_limit(
            "sources",
            limits.max_sources,
            source_info.sources.len(),
        ));
    }
    if source_info.imports.len() > limits.max_import_edges {
        return Err(ContractValidationError::resource_limit(
            "import_edges",
            limits.max_import_edges,
            source_info.imports.len(),
        ));
    }
    let bundle_bytes = source_info
        .sources
        .iter()
        .map(|source| source.source.len())
        .sum::<usize>();
    if bundle_bytes > limits.max_bundle_bytes {
        return Err(ContractValidationError::resource_limit(
            "bundle_bytes",
            limits.max_bundle_bytes,
            bundle_bytes,
        ));
    }
    if let Some(source) = source_info
        .sources
        .iter()
        .find(|source| source.source.len() > limits.max_source_bytes)
    {
        return Err(ContractValidationError::resource_limit(
            "source_bytes",
            limits.max_source_bytes,
            source.source.len(),
        ));
    }

    let mut sources = source_info.sources.clone();
    sources.sort_by(|left, right| left.name.cmp(&right.name));
    let mut imports = source_info.imports.clone();
    imports.sort();
    let expected_bundle_id = source_bundle_id(&sources, &imports);
    if source_info.sources != sources || source_info.imports != imports {
        return Err(ContractValidationError::single(
            "non_canonical_source_bundle",
            "$",
            "sources and imports must be sorted canonically",
        ));
    }
    if source_info.source_bundle_id != expected_bundle_id {
        return Err(ContractValidationError::single(
            "source_bundle_id_mismatch",
            "$.source_bundle_id",
            format!(
                "expected {expected_bundle_id}, found {}",
                source_info.source_bundle_id
            ),
        ));
    }

    let mut source_names = BTreeSet::new();
    for (index, source) in source_info.sources.iter().enumerate() {
        if source.name.is_empty() || !source_names.insert(source.name.as_str()) {
            return Err(ContractValidationError::single(
                "invalid_source_id",
                format!("$.sources[{index}].name"),
                "source logical IDs must be non-empty and unique",
            ));
        }
    }
    for (index, import) in source_info.imports.iter().enumerate() {
        if !source_names.contains(import.from.as_str())
            || !source_names.contains(import.to.as_str())
        {
            return Err(ContractValidationError::single(
                "import_source_missing",
                format!("$.imports[{index}]"),
                "both import endpoints must exist in the source bundle",
            ));
        }
    }

    for (index, declaration) in source_info.declarations.iter().enumerate() {
        validate_source_name(&source_names, &declaration.source, "declaration", index)?;
        validate_ref(contract, declaration.ty, "declaration", index)?;
    }
    for (index, actor) in source_info.actors.iter().enumerate() {
        validate_source_name(&source_names, &actor.source, "actor", index)?;
    }
    for (index, field) in source_info.field_labels.iter().enumerate() {
        validate_origin(&source_names, &field.origin, "field_labels", index)?;
        if field.path.is_empty() {
            return Err(empty_path("field_labels", index));
        }
        match contract.types.get(field.container as usize) {
            Some(TypeNode::Record { fields }) | Some(TypeNode::Variant { fields })
                if fields.iter().any(|candidate| candidate.id == field.id) => {}
            _ => {
                return Err(ContractValidationError::single(
                    "source_field_target_mismatch",
                    format!("$.field_labels[{index}]"),
                    "field provenance must target an existing aggregate field ID",
                ));
            }
        }
    }
    for (index, method) in source_info.methods.iter().enumerate() {
        validate_origin(&source_names, &method.origin, "methods", index)?;
        if method.path.is_empty() {
            return Err(empty_path("methods", index));
        }
        match contract.types.get(method.service as usize) {
            Some(TypeNode::Service { methods })
                if methods
                    .iter()
                    .any(|candidate| candidate.name == method.name) => {}
            _ => {
                return Err(ContractValidationError::single(
                    "source_method_target_mismatch",
                    format!("$.methods[{index}]"),
                    "method provenance must target an existing service method",
                ));
            }
        }
    }
    for (index, argument) in source_info.function_arguments.iter().enumerate() {
        validate_origin(&source_names, &argument.origin, "function_arguments", index)?;
        if argument.path.is_empty() {
            return Err(empty_path("function_arguments", index));
        }
        let count = match contract.types.get(argument.function as usize) {
            Some(TypeNode::Func { args, .. })
                if matches!(
                    argument.direction,
                    crate::model::SourceFunctionArgumentDirection::Argument
                ) =>
            {
                args.len()
            }
            Some(TypeNode::Func { results, .. }) => results.len(),
            _ => {
                return Err(ContractValidationError::single(
                    "source_function_target_mismatch",
                    format!("$.function_arguments[{index}]"),
                    "function provenance must target a function node",
                ));
            }
        };
        if argument.position as usize >= count {
            return Err(ContractValidationError::single(
                "source_function_position_out_of_bounds",
                format!("$.function_arguments[{index}].position"),
                format!("position {} is outside {count} value(s)", argument.position),
            ));
        }
    }
    Ok(())
}

fn validate_ref(
    contract: &Contract,
    reference: u32,
    collection: &str,
    index: usize,
) -> Result<(), ContractValidationError> {
    if reference as usize >= contract.types.len() {
        return Err(ContractValidationError::single(
            "source_type_ref_out_of_bounds",
            format!("$.{collection}[{index}]"),
            format!("type reference {reference} is outside the Contract arena"),
        ));
    }
    Ok(())
}

fn validate_source_name(
    sources: &BTreeSet<&str>,
    source: &str,
    collection: &str,
    index: usize,
) -> Result<(), ContractValidationError> {
    if !sources.contains(source) {
        return Err(ContractValidationError::single(
            "source_origin_missing",
            format!("$.{collection}[{index}]"),
            format!("source {source:?} is not present in the source bundle"),
        ));
    }
    Ok(())
}

fn validate_origin(
    sources: &BTreeSet<&str>,
    origin: &SourceOrigin,
    collection: &str,
    index: usize,
) -> Result<(), ContractValidationError> {
    let source = match origin {
        SourceOrigin::Declaration { source, .. } | SourceOrigin::Actor { source } => source,
    };
    validate_source_name(sources, source, collection, index)
}

fn empty_path(collection: &str, index: usize) -> ContractValidationError {
    ContractValidationError::single(
        "empty_source_occurrence_path",
        format!("$.{collection}[{index}].path"),
        "source occurrence paths must not be empty",
    )
}
