use crate::budget::Budget;
use crate::canonical::domain_hash;
use crate::limits::Limits;
use crate::model::{
    Contract, ContractValidationError, ContractViolation, SourceImportInfo, SourceInfo,
    SourceOrigin, TypeNode, TypeRef, SOURCE_INFO_VERSION,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::btree_map::Entry;
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
    preflight_source_info_resources(source_info, budget)?;

    validate_source_bundle_ids(source_info, budget)?;
    validate_source_bundle_identity(source_info, budget)?;
    let bundle = SourceBundleResolver::new(source_info)?;
    let compilation = crate::compile::rederive_source_bundle_with_budget(
        bundle.entry.as_str(),
        &bundle,
        // The nested context must inherit the caller's cancellation token so a
        // resolver can still observe cancellation during rederivation.
        &crate::RuntimeContext::new(budget.limits().clone())
            .with_cancellation(budget.cancellation()),
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
    observe_source_info_collections(source_info, budget)?;
    // Element counts above bound the traversals below. Charge the O(records)
    // documentation count before the O(entries) byte pass so this holds without
    // depending on the caller having run the preflight first. The byte total
    // includes the per-entry cost, so the high-water mark is unaffected.
    budget
        .observe(
            "source_string_bytes",
            limits.max_string_bytes,
            source_info_doc_entries(source_info),
        )
        .map_err(crate::budget::BudgetError::into_contract_error)?;
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
    // Bundle canonicality and identity are verified by
    // `validate_source_bundle_identity` before this structural pass runs on the
    // presented (untrusted) path, and the compiler emits canonical, correctly
    // identified sidecars by construction. Re-serializing and re-hashing the
    // whole bundle here was pure redundant work — it ran up to twice more per
    // presented-sidecar validation — so only the cheap canonical-order
    // invariant is asserted in debug builds. Recomputing the bundle hash, even
    // in an assertion, would repeat the uninterruptible serialize/hash pass
    // that `validate_source_bundle_identity` checkpoints before performing.
    // This changes no error code or precedence: a tampered presented bundle is
    // still rejected by `validate_source_bundle_identity`, which runs first.
    debug_assert!(
        source_bundle_is_canonical(source_info),
        "bundle canonical order must be established before the structural pass",
    );

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
    // Target existence is checked against a per-container index rather than a
    // linear scan. `max_fields` bounds both the field-label count and one
    // aggregate's field count independently, so a bare `fields.iter().any(...)`
    // per label is `O(max_fields^2)` of uncharged, uninterruptible work — and
    // duplicate labels, which are permitted by design, each re-pay a full scan.
    // Building each referenced container's field-ID set once turns the pass
    // linearithmic; charging that build and every lookup against
    // `provenance_work` bounds the total and makes it interruptible.
    let mut container_field_ids: BTreeMap<TypeRef, BTreeSet<u32>> = BTreeMap::new();
    for (index, field) in source_info.field_labels.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        validate_origin(&source_names, &field.origin, "field_labels", index)?;
        if field.path.is_empty() {
            return Err(empty_path("field_labels", index));
        }
        let field_ids = match container_field_ids.entry(field.container) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let fields = match contract.types.get(field.container as usize) {
                    Some(TypeNode::Record { fields }) | Some(TypeNode::Variant { fields }) => {
                        fields
                    }
                    _ => return Err(source_field_target_mismatch(index)),
                };
                charge_provenance_work(budget, fields.len())?;
                entry.insert(fields.iter().map(|candidate| candidate.id).collect())
            }
        };
        charge_provenance_work(budget, 1)?;
        if !field_ids.contains(&field.id) {
            return Err(source_field_target_mismatch(index));
        }
    }
    let mut service_method_names: BTreeMap<TypeRef, BTreeSet<&str>> = BTreeMap::new();
    for (index, method) in source_info.methods.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        validate_origin(&source_names, &method.origin, "methods", index)?;
        if method.path.is_empty() {
            return Err(empty_path("methods", index));
        }
        let method_names = match service_method_names.entry(method.service) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let methods = match contract.types.get(method.service as usize) {
                    Some(TypeNode::Service { methods }) => methods,
                    _ => return Err(source_method_target_mismatch(index)),
                };
                charge_provenance_work(budget, methods.len())?;
                entry.insert(
                    methods
                        .iter()
                        .map(|candidate| candidate.name.as_str())
                        .collect(),
                )
            }
        };
        charge_provenance_work(budget, 1)?;
        if !method_names.contains(method.name.as_str()) {
            return Err(source_method_target_mismatch(index));
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

/// Element counts for every provenance collection, in stable reporting order.
///
/// Each entry is `O(1)`, so this is safe to evaluate before any collection has
/// been bounded.
fn source_info_collection_counts(
    source_info: &SourceInfo,
    limits: &Limits,
) -> [(&'static str, usize, usize); 7] {
    [
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
    ]
}

fn observe_source_info_collections(
    source_info: &SourceInfo,
    budget: &mut Budget<'_>,
) -> Result<(), ContractValidationError> {
    let limits = budget.limits().clone();
    for (resource, limit, observed) in source_info_collection_counts(source_info, &limits) {
        budget
            .observe(resource, limit, observed)
            .map_err(crate::budget::BudgetError::into_contract_error)?;
    }
    Ok(())
}

/// Bounds the four collections that reference remapping rewrites.
///
/// Remapping runs before validation, so it must bound the collections it walks
/// itself. It deliberately excludes `sources` and `import_edges`: loading
/// charges those cumulatively, and seeding a high-water mark here would be
/// added to that later charge and reject bundles that are within their limit.
pub(crate) fn observe_remapped_collections(
    source_info: &SourceInfo,
    budget: &mut Budget<'_>,
) -> Result<(), ContractValidationError> {
    let limits = budget.limits().clone();
    for (resource, limit, observed) in [
        (
            "source_declarations",
            limits.max_declarations,
            source_info.declarations.len(),
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
    Ok(())
}

fn preflight_source_info_resources(
    source_info: &SourceInfo,
    budget: &Budget<'_>,
) -> Result<(), ContractValidationError> {
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let limits = budget.limits();
    // Element counts are O(1) and bound the traversals that follow, so they are
    // enforced before any per-entry work is done.
    for (resource, limit, observed) in source_info_collection_counts(source_info, limits) {
        if observed > limit {
            return Err(ContractValidationError::resource_limit(
                resource, limit, observed,
            ));
        }
    }
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    // Counting documentation entries is O(records) while summing their bytes is
    // O(entries), and each entry costs at least one unit. Checking the cheap
    // lower bound first keeps an inflated sidecar from driving the full pass.
    let doc_entries = source_info_doc_entries(source_info);
    if doc_entries > limits.max_string_bytes {
        return Err(ContractValidationError::resource_limit(
            "source_string_bytes",
            limits.max_string_bytes,
            doc_entries,
        ));
    }
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    let string_bytes = source_info_string_bytes(source_info);
    if string_bytes > limits.max_string_bytes {
        return Err(ContractValidationError::resource_limit(
            "source_string_bytes",
            limits.max_string_bytes,
            string_bytes,
        ));
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
    // Bound each logical source ID individually, and last, so this new terminal
    // check never preempts an existing byte/count limit. The counts above cap
    // these loops, and running in the preflight still rejects an oversized path
    // before the bundle is hashed, resolved, or rederived. Import spellings are
    // paths too, so they share the bound.
    let max_id_bytes = limits.max_source_id_bytes;
    for source in &source_info.sources {
        check_source_id_bytes(&source.name, max_id_bytes)?;
    }
    for import in &source_info.imports {
        check_source_id_bytes(&import.from, max_id_bytes)?;
        check_source_id_bytes(&import.import, max_id_bytes)?;
        check_source_id_bytes(&import.to, max_id_bytes)?;
    }
    Ok(())
}

/// Verifies canonical ordering in place.
///
/// A stable sort of an already-sorted slice is the identity, so checking
/// adjacent pairs is equivalent to the previous clone-sort-compare while
/// allocating nothing.
fn source_bundle_is_canonical(source_info: &SourceInfo) -> bool {
    source_info
        .sources
        .windows(2)
        .all(|pair| pair[0].name <= pair[1].name)
        && source_info
            .imports
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
}

fn validate_source_bundle_identity(
    source_info: &SourceInfo,
    budget: &Budget<'_>,
) -> Result<(), ContractValidationError> {
    // The bundle bytes are already bounded (preflight enforced `bundle_bytes`),
    // but hashing them is not itself interruptible, so poll cancellation and the
    // deadline before starting the whole-bundle serialize/hash pass.
    budget
        .checkpoint()
        .map_err(crate::budget::BudgetError::into_contract_error)?;
    if !source_bundle_is_canonical(source_info) {
        return Err(ContractValidationError::single(
            "non_canonical_source_bundle",
            "$",
            "sources and imports must be sorted canonically",
        ));
    }
    let expected = source_bundle_id(&source_info.sources, &source_info.imports);
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

fn validate_source_bundle_ids(
    source_info: &SourceInfo,
    budget: &Budget<'_>,
) -> Result<(), ContractValidationError> {
    for (index, source) in source_info.sources.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
        parse_canonical_source_id(&source.name, "sources", index, "name")?;
    }
    for (index, import) in source_info.imports.iter().enumerate() {
        budget
            .checkpoint()
            .map_err(crate::budget::BudgetError::into_contract_error)?;
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

/// Total documentation entries across every provenance collection.
///
/// Documentation is the one provenance category whose cardinality is not
/// implied by a collection length: a single record may carry any number of
/// entries. This is `O(records)` because each `docs.len()` is `O(1)`, so it is
/// a cheap lower bound on [`source_info_string_bytes`], which is
/// `O(entries)`.
fn source_info_doc_entries(source_info: &SourceInfo) -> usize {
    let mut entries = 0usize;
    let mut add = |count: usize| entries = entries.saturating_add(count);
    for declaration in &source_info.declarations {
        add(declaration.docs.len());
    }
    for actor in &source_info.actors {
        add(actor.docs.len());
    }
    for field in &source_info.field_labels {
        add(field.docs.len());
    }
    for method in &source_info.methods {
        add(method.docs.len());
    }
    entries
}

/// Units of the string budget consumed by a documentation block.
///
/// Every entry costs one unit before its content, so a block of empty entries
/// cannot inflate the sidecar for free. The per-entry cost is a fixed constant
/// rather than the platform's `String` footprint, so the reported `observed`
/// value stays identical on every target.
fn docs_string_units(docs: &[String]) -> usize {
    docs.iter()
        .fold(docs.len(), |units, doc| units.saturating_add(doc.len()))
}

fn source_info_string_bytes(source_info: &SourceInfo) -> usize {
    let mut bytes = source_info
        .contract_id
        .len()
        .saturating_add(source_info.source_bundle_id.len());
    let mut add = |amount: usize| bytes = bytes.saturating_add(amount);
    for source in &source_info.sources {
        add(source.name.len());
    }
    for import in &source_info.imports {
        add(import.from.len());
        add(import.import.len());
        add(import.to.len());
    }
    for declaration in &source_info.declarations {
        add(declaration.source.len());
        add(declaration.name.len());
        add(docs_string_units(&declaration.docs));
    }
    for actor in &source_info.actors {
        add(actor.source.len());
        add(docs_string_units(&actor.docs));
    }
    for field in &source_info.field_labels {
        add(origin_string_bytes(&field.origin));
        add(field.path.len());
        if let crate::SourceLabel::Named { name } = &field.label {
            add(name.len());
        }
        add(docs_string_units(&field.docs));
    }
    for method in &source_info.methods {
        add(origin_string_bytes(&method.origin));
        add(method.path.len());
        add(method.name.len());
        add(docs_string_units(&method.docs));
    }
    for argument in &source_info.function_arguments {
        add(origin_string_bytes(&argument.origin));
        add(argument.path.len());
        add(argument.name.len());
    }
    bytes
}

fn origin_string_bytes(origin: &SourceOrigin) -> usize {
    match origin {
        SourceOrigin::Declaration { source, name } => source.len().saturating_add(name.len()),
        SourceOrigin::Actor { source } => source.len(),
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

fn check_source_id_bytes(value: &str, limit: usize) -> Result<(), ContractValidationError> {
    if value.len() > limit {
        return Err(ContractValidationError::resource_limit(
            "source_id_bytes",
            limit,
            value.len(),
        ));
    }
    Ok(())
}

fn source_field_target_mismatch(index: usize) -> ContractValidationError {
    ContractValidationError::single(
        "source_field_target_mismatch",
        format!("$.field_labels[{index}]"),
        "field provenance must target an existing aggregate field ID",
    )
}

fn source_method_target_mismatch(index: usize) -> ContractValidationError {
    ContractValidationError::single(
        "source_method_target_mismatch",
        format!("$.methods[{index}]"),
        "method provenance must target an existing service method",
    )
}

/// Charge provenance target-resolution work against its own budget counter.
///
/// A dedicated `provenance_work` resource keeps this off `canonicalization_work`
/// so that rederiving a large graph and then indexing its provenance cannot
/// jointly overflow one counter and reject a bundle neither pass would reject
/// alone. Every field-ID set built and every membership test is charged, so the
/// fan-out and duplicate-label work is bounded and interruptible on one budget.
fn charge_provenance_work(
    budget: &mut Budget<'_>,
    amount: usize,
) -> Result<(), ContractValidationError> {
    let limit = budget.limits().max_provenance_work;
    budget
        .charge("provenance_work", limit, amount)
        .map(|_| ())
        .map_err(crate::budget::BudgetError::into_contract_error)
}
