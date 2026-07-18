use crate::budget::Budget;
use crate::canonical::domain_hash;
use crate::limits::Limits;
use crate::model::{
    Contract, ContractValidationError, ContractViolation, SourceImportInfo, SourceInfo,
    SourceOrigin, TypeNode, SOURCE_INFO_VERSION,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

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
    let mut budget = Budget::from_limits(limits);
    validate_source_info_with_budget(source_info, contract, &mut budget)
}

pub(crate) fn validate_source_info_with_budget(
    source_info: &SourceInfo,
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<(), ContractValidationError> {
    validate_source_info_header(source_info, contract, budget)?;
    preflight_source_info_resources(source_info, budget.limits())?;

    validate_source_bundle_ids(source_info)?;
    validate_source_bundle_identity(source_info)?;
    let bundle = SourceBundleResolver::new(source_info)?;
    let compilation = crate::compile::rederive_source_bundle_with_budget(
        bundle.entry.as_str(),
        &bundle,
        &crate::RuntimeContext::new(budget.limits().clone()),
        budget,
    )
    .map_err(rederivation_error)?;

    validate_source_info_structure_with_budget(source_info, contract, budget)?;
    if compilation.contract().contract_id() != contract.contract_id() {
        return Err(ContractValidationError::single(
            "source_contract_rederivation_mismatch",
            "$.contract_id",
            "the embedded source bundle does not rederive the bound Contract",
        ));
    }
    let expected = compilation.source_info().ok_or_else(|| {
        ContractValidationError::single(
            "source_info_rederivation_missing",
            "$",
            "the compiler did not produce source provenance for the embedded bundle",
        )
    })?;
    compare_rederived_source_info(source_info, expected)
}

pub(crate) fn validate_source_info_structure_with_budget(
    source_info: &SourceInfo,
    contract: &Contract,
    budget: &mut Budget<'_>,
) -> Result<(), ContractValidationError> {
    validate_source_info_header(source_info, contract, budget)?;
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let limits = budget.limits().clone();
    budget
        .observe("sources", limits.max_sources, source_info.sources.len())
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    budget
        .observe(
            "import_edges",
            limits.max_import_edges,
            source_info.imports.len(),
        )
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    for (resource, limit, observed) in [
        (
            "source_declarations",
            limits.max_declarations,
            source_info.declarations.len(),
        ),
        (
            "source_actors",
            limits.max_sources,
            source_info.actors.len(),
        ),
        (
            "source_field_labels",
            limits.max_fields,
            source_info.field_labels.len(),
        ),
        (
            "source_methods",
            limits.max_methods,
            source_info.methods.len(),
        ),
        (
            "source_function_arguments",
            limits.max_function_values,
            source_info.function_arguments.len(),
        ),
    ] {
        budget
            .observe(resource, limit, observed)
            .map_err(crate::budget::BudgetError::into_contract_error)?;
    }
    budget
        .observe(
            "source_string_bytes",
            limits.max_string_bytes,
            source_info_string_bytes(source_info),
        )
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let bundle_bytes = source_info
        .sources
        .iter()
        .map(|source| source.source.len())
        .fold(0usize, usize::saturating_add);
    budget
        .observe("bundle_bytes", limits.max_bundle_bytes, bundle_bytes)
        .map_err(crate::budget::BudgetError::into_contract_error)?;
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

    for (index, source) in source_info.sources.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        validate_source_id(&source.name, "sources", index, "name")?;
    }
    for (index, import) in source_info.imports.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        validate_source_id(&import.from, "imports", index, "from")?;
        validate_source_id(&import.to, "imports", index, "to")?;
    }

    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let mut sources = source_info.sources.clone();
    sources.sort_by(|left, right| left.name.cmp(&right.name));
    let mut imports = source_info.imports.clone();
    imports.sort();
    let expected_bundle_id = source_bundle_id(&sources, &imports);
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
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
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        validate_source_name(&source_names, &declaration.source, "declaration", index)?;
        validate_ref(contract, declaration.ty, "declaration", index)?;
    }
    for (index, actor) in source_info.actors.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        validate_source_name(&source_names, &actor.source, "actor", index)?;
    }
    for (index, field) in source_info.field_labels.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
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
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
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
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
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

fn validate_source_info_header(
    source_info: &SourceInfo,
    contract: &Contract,
    budget: &Budget<'_>,
) -> Result<(), ContractValidationError> {
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
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
    Ok(())
}

fn preflight_source_info_resources(
    source_info: &SourceInfo,
    limits: &Limits,
) -> Result<(), ContractValidationError> {
    for (resource, limit, observed) in [
        ("sources", limits.max_sources, source_info.sources.len()),
        (
            "import_edges",
            limits.max_import_edges,
            source_info.imports.len(),
        ),
        (
            "source_declarations",
            limits.max_declarations,
            source_info.declarations.len(),
        ),
        (
            "source_actors",
            limits.max_sources,
            source_info.actors.len(),
        ),
        (
            "source_field_labels",
            limits.max_fields,
            source_info.field_labels.len(),
        ),
        (
            "source_methods",
            limits.max_methods,
            source_info.methods.len(),
        ),
        (
            "source_function_arguments",
            limits.max_function_values,
            source_info.function_arguments.len(),
        ),
        (
            "source_string_bytes",
            limits.max_string_bytes,
            source_info_string_bytes(source_info),
        ),
    ] {
        if observed > limit {
            return Err(ContractValidationError::resource_limit(
                resource, limit, observed,
            ));
        }
    }
    let bundle_bytes = source_info
        .sources
        .iter()
        .map(|source| source.source.len())
        .fold(0usize, usize::saturating_add);
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
    Ok(())
}

fn validate_source_bundle_identity(
    source_info: &SourceInfo,
) -> Result<(), ContractValidationError> {
    let mut sources = source_info.sources.clone();
    sources.sort_by(|left, right| left.name.cmp(&right.name));
    let mut imports = source_info.imports.clone();
    imports.sort();
    if source_info.sources != sources || source_info.imports != imports {
        return Err(ContractValidationError::single(
            "non_canonical_source_bundle",
            "$",
            "sources and imports must be sorted canonically",
        ));
    }
    let expected = source_bundle_id(&sources, &imports);
    if source_info.source_bundle_id != expected {
        return Err(ContractValidationError::single(
            "source_bundle_id_mismatch",
            "$.source_bundle_id",
            format!(
                "expected {expected}, found {}",
                source_info.source_bundle_id
            ),
        ));
    }
    Ok(())
}

fn validate_source_bundle_ids(source_info: &SourceInfo) -> Result<(), ContractValidationError> {
    for (index, source) in source_info.sources.iter().enumerate() {
        parse_canonical_source_id(&source.name, "sources", index, "name")?;
    }
    for (index, import) in source_info.imports.iter().enumerate() {
        parse_canonical_source_id(&import.from, "imports", index, "from")?;
        parse_canonical_source_id(&import.to, "imports", index, "to")?;
    }
    Ok(())
}

fn compare_rederived_source_info(
    presented: &SourceInfo,
    expected: &SourceInfo,
) -> Result<(), ContractValidationError> {
    for (path, matches) in [
        ("$.sources", presented.sources == expected.sources),
        ("$.imports", presented.imports == expected.imports),
        (
            "$.declarations",
            presented.declarations == expected.declarations,
        ),
        (
            "$.field_labels",
            presented.field_labels == expected.field_labels,
        ),
        ("$.methods", presented.methods == expected.methods),
        (
            "$.function_arguments",
            presented.function_arguments == expected.function_arguments,
        ),
        ("$.actors", presented.actors == expected.actors),
    ] {
        if !matches {
            return Err(ContractValidationError::single(
                "source_info_provenance_mismatch",
                path,
                "presented provenance does not match provenance rederived from the embedded source bundle",
            ));
        }
    }
    Ok(())
}

fn rederivation_error(error: crate::CompileError) -> ContractValidationError {
    ContractValidationError {
        violations: error
            .diagnostics
            .into_iter()
            .map(|diagnostic| ContractViolation {
                code: diagnostic.code,
                path: "$".to_string(),
                message: format!(
                    "the embedded source bundle could not rederive provenance: {}",
                    diagnostic.message
                ),
                resource_limit: diagnostic.resource_limit,
            })
            .collect(),
    }
}

struct SourceBundleResolver {
    entry: crate::SourceId,
    sources: BTreeMap<crate::SourceId, String>,
    imports: BTreeMap<(crate::SourceId, String), crate::SourceId>,
}

impl SourceBundleResolver {
    fn new(source_info: &SourceInfo) -> Result<Self, ContractValidationError> {
        let mut sources = BTreeMap::new();
        for (index, source) in source_info.sources.iter().enumerate() {
            let id = parse_canonical_source_id(&source.name, "sources", index, "name")?;
            if sources.insert(id, source.source.clone()).is_some() {
                return Err(ContractValidationError::single(
                    "invalid_source_id",
                    format!("$.sources[{index}].name"),
                    "source logical IDs must be unique",
                ));
            }
        }

        let mut imports = BTreeMap::new();
        let mut imported = BTreeSet::new();
        for (index, import) in source_info.imports.iter().enumerate() {
            let from = parse_canonical_source_id(&import.from, "imports", index, "from")?;
            let to = parse_canonical_source_id(&import.to, "imports", index, "to")?;
            if !sources.contains_key(&from) || !sources.contains_key(&to) {
                return Err(ContractValidationError::single(
                    "import_source_missing",
                    format!("$.imports[{index}]"),
                    "both import endpoints must exist in the source bundle",
                ));
            }
            let key = (from, import.import.clone());
            if imports.get(&key).is_some_and(|existing| existing != &to) {
                return Err(ContractValidationError::single(
                    "ambiguous_source_import",
                    format!("$.imports[{index}]"),
                    "one source import spelling cannot resolve to multiple targets",
                ));
            }
            imports.insert(key, to.clone());
            imported.insert(to);
        }

        let mut roots = sources.keys().filter(|id| !imported.contains(*id));
        let entry = roots.next().cloned().ok_or_else(|| {
            ContractValidationError::single(
                "source_bundle_entry_missing",
                "$.sources",
                "the source bundle has no unique entry source",
            )
        })?;
        if roots.next().is_some() {
            return Err(ContractValidationError::single(
                "source_bundle_entry_ambiguous",
                "$.sources",
                "the source bundle has more than one possible entry source",
            ));
        }
        Ok(Self {
            entry,
            sources,
            imports,
        })
    }
}

impl crate::SourceResolver for SourceBundleResolver {
    fn identify(
        &self,
        from: Option<&crate::SourceId>,
        import: &str,
    ) -> Result<crate::SourceId, crate::ResolveError> {
        match from {
            None => {
                let entry = crate::SourceId::parse(import)?;
                if entry == self.entry {
                    Ok(entry)
                } else {
                    Err(bundle_resolve_error(
                        "did_source_not_found",
                        format!("source bundle entry {import:?} is not recorded"),
                    ))
                }
            }
            Some(from) => self
                .imports
                .get(&(from.clone(), import.to_string()))
                .cloned()
                .ok_or_else(|| {
                    bundle_resolve_error(
                        "did_source_not_found",
                        format!(
                            "import {import:?} from {:?} is not recorded in the source bundle",
                            from.as_str()
                        ),
                    )
                }),
        }
    }

    fn load(
        &self,
        id: &crate::SourceId,
        limits: &Limits,
    ) -> Result<crate::ResolvedSource, crate::ResolveError> {
        let source = self.sources.get(id).ok_or_else(|| {
            bundle_resolve_error(
                "did_source_not_found",
                format!(
                    "source {:?} is not recorded in the source bundle",
                    id.as_str()
                ),
            )
        })?;
        if source.len() > limits.max_source_bytes {
            return Err(crate::ResolveError {
                code: "resource_limit_exceeded".to_string(),
                message: format!(
                    "source {:?} uses {} bytes; limit is {}",
                    id.as_str(),
                    source.len(),
                    limits.max_source_bytes
                ),
                resource_limit: Some(crate::ResourceLimitInfo {
                    resource: "source_bytes".to_string(),
                    limit: limits.max_source_bytes,
                    observed: source.len(),
                }),
            });
        }
        Ok(crate::ResolvedSource {
            id: id.clone(),
            source: source.clone(),
            digest: format!("sha256:{}", hex::encode(Sha256::digest(source.as_bytes()))),
        })
    }
}

fn parse_canonical_source_id(
    value: &str,
    collection: &str,
    index: usize,
    field: &str,
) -> Result<crate::SourceId, ContractValidationError> {
    let id = crate::SourceId::parse(value).map_err(|error| {
        ContractValidationError::single(
            "invalid_source_id",
            format!("$.{collection}[{index}].{field}"),
            error.message,
        )
    })?;
    if id.as_str() != value {
        return Err(ContractValidationError::single(
            "invalid_source_id",
            format!("$.{collection}[{index}].{field}"),
            format!(
                "source logical ID {value:?} is not canonical; expected {:?}",
                id.as_str()
            ),
        ));
    }
    Ok(id)
}

fn bundle_resolve_error(code: &str, message: String) -> crate::ResolveError {
    crate::ResolveError {
        code: code.to_string(),
        message,
        resource_limit: None,
    }
}

fn source_info_string_bytes(source_info: &SourceInfo) -> usize {
    let mut bytes = source_info
        .contract_id
        .len()
        .saturating_add(source_info.source_bundle_id.len());
    let mut add = |value: &str| bytes = bytes.saturating_add(value.len());
    for source in &source_info.sources {
        add(&source.name);
    }
    for import in &source_info.imports {
        add(&import.from);
        add(&import.import);
        add(&import.to);
    }
    for declaration in &source_info.declarations {
        add(&declaration.source);
        add(&declaration.name);
        for doc in &declaration.docs {
            add(doc);
        }
    }
    for actor in &source_info.actors {
        add(&actor.source);
        for doc in &actor.docs {
            add(doc);
        }
    }
    for field in &source_info.field_labels {
        add_origin_strings(&field.origin, &mut add);
        add(&field.path);
        if let crate::SourceLabel::Named { name } = &field.label {
            add(name);
        }
        for doc in &field.docs {
            add(doc);
        }
    }
    for method in &source_info.methods {
        add_origin_strings(&method.origin, &mut add);
        add(&method.path);
        add(&method.name);
        for doc in &method.docs {
            add(doc);
        }
    }
    for argument in &source_info.function_arguments {
        add_origin_strings(&argument.origin, &mut add);
        add(&argument.path);
        add(&argument.name);
    }
    bytes
}

fn add_origin_strings(origin: &SourceOrigin, add: &mut impl FnMut(&str)) {
    match origin {
        SourceOrigin::Declaration { source, name } => {
            add(source);
            add(name);
        }
        SourceOrigin::Actor { source } => add(source),
    }
}

fn validate_source_id(
    value: &str,
    collection: &str,
    index: usize,
    field: &str,
) -> Result<(), ContractValidationError> {
    match crate::SourceId::parse(value) {
        Ok(source_id) if source_id.as_str() == value => Ok(()),
        Ok(source_id) => Err(ContractValidationError::single(
            "invalid_source_id",
            format!("$.{collection}[{index}].{field}"),
            format!(
                "source logical ID {value:?} is not canonical; expected {:?}",
                source_id.as_str()
            ),
        )),
        Err(error) => Err(ContractValidationError::single(
            "invalid_source_id",
            format!("$.{collection}[{index}].{field}"),
            error.message,
        )),
    }
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
