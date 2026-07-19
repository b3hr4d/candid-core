use super::*;

pub(super) fn lower_checked(
    source_units: &[SourceUnit],
    environment: &TypeEnv,
    actor_type: Option<&Type>,
    options: CompileOptions,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<Compilation, CompileError> {
    let limits = budget.limits().clone();
    check_type_depth(environment, actor_type, budget)?;
    let mut lowerer = Lowerer::new(environment);
    let declaration_names: Vec<_> = environment.0.keys().cloned().collect();
    for name in &declaration_names {
        budget
            .checkpoint()
            .map_err(|error| budget_error(error, DiagnosticPhase::Lower, "Contract lowering"))?;
        lowerer.lower_named(name).map_err(lower_error)?;
    }

    let actor = match actor_type {
        None => None,
        Some(actor_type) => {
            let reference = lowerer.lower_type(actor_type).map_err(lower_error)?;
            let node = lowerer.nodes[reference as usize].as_ref().ok_or_else(|| {
                lower_error("actor type was not fully lowered into the Contract arena")
            })?;
            match node {
                TypeNode::Service { .. } => Some(Actor::Service { service: reference }),
                TypeNode::Class { .. } => Some(Actor::Class { class: reference }),
                _ => return Err(lower_error(
                    "official Candid checker returned an actor that is neither service nor class",
                )),
            }
        }
    };

    let mut raw_source_info = RawSourceInfo::default();
    if options.include_source_info {
        let actor_service = actor_service_ref(actor.as_ref(), &lowerer).map_err(lower_error)?;
        collect_source_units_info(
            source_units,
            actor_service,
            environment,
            &mut lowerer,
            &mut raw_source_info,
        )
        .map_err(lower_error)?;
        for (resource, limit, observed) in [
            (
                "source_declarations",
                limits.max_declarations,
                raw_source_info.declarations.len(),
            ),
            (
                "source_actors",
                limits.max_sources,
                raw_source_info.actors.len(),
            ),
            (
                "source_field_labels",
                limits.max_fields,
                raw_source_info.field_labels.len(),
            ),
            (
                "source_methods",
                limits.max_methods,
                raw_source_info.methods.len(),
            ),
            (
                "source_function_arguments",
                limits.max_function_values,
                raw_source_info.function_arguments.len(),
            ),
        ] {
            budget.observe(resource, limit, observed).map_err(|error| {
                budget_error(error, DiagnosticPhase::Lower, "provenance collection")
            })?;
        }
    }

    let LoweredGraph { types, named_refs } = lowerer.finish().map_err(lower_error)?;
    let declarations = declaration_names
        .iter()
        .map(|name| {
            Ok(Declaration {
                name: name.clone(),
                ty: *named_refs
                    .get(name)
                    .ok_or_else(|| format!("missing lowered declaration {name}"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(lower_error)?;

    // Structural validation needs a syntactically valid placeholder. The
    // canonicalizer then computes the real identities.
    let raw_contract = Contract::new_unchecked(types, declarations, actor);
    crate::validate::validate_structure_with_budget(&raw_contract, budget)
        .map_err(|error| lower_error(format!("lowered Contract violated an invariant: {error}")))?;
    let canonicalized =
        canonical::canonicalize_with_mapping_unchecked_with_budget(&raw_contract, budget)
            .map_err(|error| lower_error(format!("canonicalization failed: {error}")))?;

    let source_info = if options.include_source_info {
        // Remapping and sorting the collected provenance is proportional to the
        // bundle, so it must remain interruptible.
        budget
            .checkpoint()
            .map_err(|error| budget_error(error, DiagnosticPhase::Lower, "provenance remapping"))?;
        let mut field_labels: Vec<_> = raw_source_info
            .field_labels
            .into_iter()
            .map(|label| FieldLabelProvenance {
                origin: label.origin,
                path: label.path,
                container: canonicalized.old_to_new[label.container as usize],
                id: label.id,
                label: label.label,
                docs: label.docs,
            })
            .collect();
        field_labels.sort_by(compare_field_label_provenance);

        let mut methods: Vec<_> = raw_source_info
            .methods
            .into_iter()
            .map(|method| SourceMethodInfo {
                origin: method.origin,
                path: method.path,
                service: canonicalized.old_to_new[method.service as usize],
                name: method.name,
                docs: method.docs,
            })
            .collect();
        methods.sort_by(compare_source_method_info);

        let mut function_arguments: Vec<_> = raw_source_info
            .function_arguments
            .into_iter()
            .map(|argument| SourceFunctionArgumentInfo {
                origin: argument.origin,
                path: argument.path,
                function: canonicalized.old_to_new[argument.function as usize],
                direction: argument.direction,
                position: argument.position,
                name: argument.name,
            })
            .collect();
        function_arguments.sort_by(compare_source_function_argument_info);

        let mut declarations: Vec<_> = raw_source_info
            .declarations
            .into_iter()
            .map(|declaration| SourceDeclaration {
                source: declaration.source,
                name: declaration.name,
                ty: canonicalized.old_to_new[declaration.ty as usize],
                docs: declaration.docs,
            })
            .collect();
        declarations.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.name.cmp(&right.name))
                .then(left.ty.cmp(&right.ty))
        });
        let mut actors: Vec<_> = raw_source_info
            .actors
            .into_iter()
            .map(|actor| SourceActorInfo {
                source: actor.source,
                docs: actor.docs,
            })
            .collect();
        actors.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.docs.cmp(&right.docs))
        });
        budget
            .checkpoint()
            .map_err(|error| budget_error(error, DiagnosticPhase::Lower, "provenance remapping"))?;
        let mut sources: Vec<_> = source_units
            .iter()
            .map(|unit| SourceFileInfo {
                name: unit.name.clone(),
                source: unit.source.clone(),
            })
            .collect();
        sources.sort_by(|left, right| left.name.cmp(&right.name));
        let mut imports = source_imports(source_units);
        imports.sort();
        let source_bundle_id = crate::source::source_bundle_id(&sources, &imports);
        let source_info = SourceInfo {
            source_info_version: SOURCE_INFO_VERSION,
            contract_id: canonicalized.contract.contract_id().to_string(),
            source_bundle_id,
            sources,
            imports,
            declarations,
            field_labels,
            methods,
            function_arguments,
            actors,
        };
        crate::source::validate_source_info_structure_with_budget(
            &source_info,
            &canonicalized.contract,
            budget,
        )
        .map_err(source_info_compile_error)?;
        Some(source_info)
    } else {
        None
    };

    Ok(Compilation {
        contract: canonicalized.contract,
        source_info,
    })
}

fn check_type_depth(
    environment: &TypeEnv,
    actor_type: Option<&Type>,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(), CompileError> {
    let limits = budget.limits().clone();
    let mut roots: Vec<Type> = environment.0.values().cloned().collect();
    roots.extend(actor_type.cloned());
    let mut pending: Vec<_> = roots
        .into_iter()
        .map(|ty| (ty, 0usize, BTreeSet::<String>::new()))
        .collect();

    while let Some((ty, depth, active_names)) = pending.pop() {
        budget.checkpoint().map_err(|error| {
            budget_error(error, DiagnosticPhase::Lower, "checked type traversal")
        })?;
        if depth > limits.max_type_depth {
            return Err(CompileError::resource_limit(
                "type_depth",
                limits.max_type_depth,
                depth,
                format!(
                    "checked Candid type depth {depth} exceeds limit {}",
                    limits.max_type_depth
                ),
            ));
        }
        let next_depth = depth.saturating_add(1);
        match ty.as_ref() {
            TypeInner::Var(name) => {
                if !active_names.contains(name) {
                    let mut next_names = active_names;
                    next_names.insert(name.clone());
                    let resolved = environment
                        .find_type(name)
                        .map_err(|error| lower_error(error.to_string()))?
                        .clone();
                    pending.push((resolved, depth, next_names));
                }
            }
            TypeInner::Opt(inner) | TypeInner::Vec(inner) => {
                pending.push((inner.clone(), next_depth, active_names));
            }
            TypeInner::Record(fields) | TypeInner::Variant(fields) => {
                for field in fields {
                    pending.push((field.ty.clone(), next_depth, active_names.clone()));
                }
            }
            TypeInner::Func(function) => {
                for ty in function.args.iter().chain(&function.rets) {
                    pending.push((ty.clone(), next_depth, active_names.clone()));
                }
            }
            TypeInner::Service(methods) => {
                for (_, ty) in methods {
                    pending.push((ty.clone(), next_depth, active_names.clone()));
                }
            }
            TypeInner::Class(init, service) => {
                pending.push((service.clone(), next_depth, active_names.clone()));
                for ty in init {
                    pending.push((ty.clone(), next_depth, active_names.clone()));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn source_imports(source_units: &[SourceUnit]) -> Vec<SourceImportInfo> {
    let mut imports = Vec::new();
    for unit in source_units {
        for import in &unit.imports {
            imports.push(SourceImportInfo {
                from: unit.name.clone(),
                import: import.import.clone(),
                to: import.target.as_str().to_string(),
                kind: import.kind,
            });
        }
    }
    imports
}
struct Lowerer<'a> {
    environment: &'a TypeEnv,
    nodes: Vec<Option<TypeNode>>,
    named_refs: BTreeMap<String, TypeRef>,
    primitive_refs: BTreeMap<PrimitiveType, TypeRef>,
    composite_refs: HashMap<Type, TypeRef>,
}

struct LoweredGraph {
    types: Vec<TypeNode>,
    named_refs: BTreeMap<String, TypeRef>,
}

impl<'a> Lowerer<'a> {
    fn new(environment: &'a TypeEnv) -> Self {
        Self {
            environment,
            nodes: Vec::new(),
            named_refs: BTreeMap::new(),
            primitive_refs: BTreeMap::new(),
            composite_refs: HashMap::new(),
        }
    }

    fn finish(self) -> Result<LoweredGraph, String> {
        let mut types = Vec::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.into_iter().enumerate() {
            types.push(node.ok_or_else(|| format!("type node {index} was left incomplete"))?);
        }
        Ok(LoweredGraph {
            types,
            named_refs: self.named_refs,
        })
    }

    fn lower_named(&mut self, name: &str) -> Result<TypeRef, String> {
        let mut pending = Vec::new();
        let reference = self.ensure_named(name, &mut pending)?;
        self.drain_pending(&mut pending)?;
        Ok(reference)
    }

    fn ensure_named(
        &mut self,
        name: &str,
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<TypeRef, String> {
        if let Some(reference) = self.named_refs.get(name) {
            return Ok(*reference);
        }
        let terminal = self.terminal_name(name)?;
        if terminal != name {
            let reference = self.ensure_named(&terminal, pending)?;
            self.named_refs.insert(name.to_string(), reference);
            return Ok(reference);
        }

        let ty = self
            .environment
            .find_type(&terminal)
            .map_err(|error| error.to_string())?
            .clone();
        if primitive_from_type(ty.as_ref()).is_some() {
            let reference = self.ensure_type(&ty, pending)?;
            self.named_refs.insert(terminal, reference);
            return Ok(reference);
        }
        if let Some(reference) = self.composite_refs.get(&ty) {
            self.named_refs.insert(terminal, *reference);
            return Ok(*reference);
        }

        let reference = self.reserve()?;
        self.named_refs.insert(terminal, reference);
        self.composite_refs.insert(ty.clone(), reference);
        pending.push((reference, ty));
        Ok(reference)
    }

    fn terminal_name(&self, name: &str) -> Result<String, String> {
        let mut seen = BTreeSet::new();
        let mut current = name.to_string();
        loop {
            if !seen.insert(current.clone()) {
                return Err(format!("unproductive alias cycle reached {current}"));
            }
            let ty = self
                .environment
                .find_type(&current)
                .map_err(|error| error.to_string())?;
            if let TypeInner::Var(next) = ty.as_ref() {
                current = next.clone();
            } else {
                return Ok(current);
            }
        }
    }

    fn lower_type(&mut self, ty: &Type) -> Result<TypeRef, String> {
        let mut pending = Vec::new();
        let reference = self.ensure_type(ty, &mut pending)?;
        self.drain_pending(&mut pending)?;
        Ok(reference)
    }

    fn ensure_type(
        &mut self,
        ty: &Type,
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<TypeRef, String> {
        if let TypeInner::Var(name) = ty.as_ref() {
            return self.ensure_named(name, pending);
        }
        if let Some(primitive) = primitive_from_type(ty.as_ref()) {
            if let Some(reference) = self.primitive_refs.get(&primitive) {
                return Ok(*reference);
            }
            let reference = self.reserve()?;
            self.primitive_refs.insert(primitive, reference);
            self.nodes[reference as usize] = Some(TypeNode::Primitive { primitive });
            return Ok(reference);
        }
        if let Some(reference) = self.composite_refs.get(ty) {
            return Ok(*reference);
        }
        let reference = self.reserve()?;
        self.composite_refs.insert(ty.clone(), reference);
        pending.push((reference, ty.clone()));
        Ok(reference)
    }

    fn drain_pending(&mut self, pending: &mut Vec<(TypeRef, Type)>) -> Result<(), String> {
        while let Some((reference, ty)) = pending.pop() {
            if self.nodes[reference as usize].is_none() {
                self.fill_one(reference, &ty, pending)?;
            }
        }
        Ok(())
    }

    fn reserve(&mut self) -> Result<TypeRef, String> {
        let reference = u32::try_from(self.nodes.len())
            .map_err(|_| "Contract type arena exceeds u32 references")?;
        self.nodes.push(None);
        Ok(reference)
    }

    fn fill_one(
        &mut self,
        reference: TypeRef,
        ty: &Type,
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<(), String> {
        let node = match ty.as_ref() {
            TypeInner::Opt(inner) => TypeNode::Opt {
                inner: self.ensure_type(inner, pending)?,
            },
            TypeInner::Vec(inner) => TypeNode::Vec {
                inner: self.ensure_type(inner, pending)?,
            },
            TypeInner::Record(fields) => TypeNode::Record {
                fields: self.lower_fields(fields, pending)?,
            },
            TypeInner::Variant(fields) => TypeNode::Variant {
                fields: self.lower_fields(fields, pending)?,
            },
            TypeInner::Func(function) => TypeNode::Func {
                args: function
                    .args
                    .iter()
                    .map(|argument| self.ensure_type(argument, pending))
                    .collect::<Result<Vec<_>, _>>()?,
                results: function
                    .rets
                    .iter()
                    .map(|result| self.ensure_type(result, pending))
                    .collect::<Result<Vec<_>, _>>()?,
                mode: lower_mode(&function.modes)?,
            },
            TypeInner::Service(methods) => {
                let mut lowered = methods
                    .iter()
                    .map(|(name, function)| {
                        Ok(ServiceMethod {
                            name: name.clone(),
                            id: candid_parser::candid::idl_hash(name),
                            function: self.ensure_type(function, pending)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                lowered.sort_by(|left, right| {
                    left.id
                        .cmp(&right.id)
                        .then(left.name.cmp(&right.name))
                        .then(left.function.cmp(&right.function))
                });
                TypeNode::Service { methods: lowered }
            }
            TypeInner::Class(init, service) => TypeNode::Class {
                init: init
                    .iter()
                    .map(|argument| self.ensure_type(argument, pending))
                    .collect::<Result<Vec<_>, _>>()?,
                service: self.ensure_type(service, pending)?,
            },
            TypeInner::Var(name) => {
                return Err(format!("unresolved alias {name} leaked into lowering"))
            }
            TypeInner::Unknown => {
                return Err("unknown Candid type leaked into lowering".to_string())
            }
            TypeInner::Knot(_) => {
                return Err("Rust-only recursive knot leaked into DID lowering".to_string())
            }
            TypeInner::Future => {
                return Err("unsupported non-DID future type leaked into lowering".to_string())
            }
            primitive if primitive_from_type(primitive).is_some() => {
                return Err("primitive type reached composite lowering".to_string())
            }
            other => {
                return Err(format!(
                    "unsupported Candid type leaked into lowering: {other:?}"
                ))
            }
        };
        self.nodes[reference as usize] = Some(node);
        Ok(())
    }

    fn lower_fields(
        &mut self,
        fields: &[candid_parser::candid::types::Field],
        pending: &mut Vec<(TypeRef, Type)>,
    ) -> Result<Vec<Field>, String> {
        let mut lowered = Vec::with_capacity(fields.len());
        for field in fields {
            let id = field.id.get_id();
            lowered.push(Field {
                id,
                ty: self.ensure_type(&field.ty, pending)?,
            });
        }
        lowered.sort_by(|left, right| left.id.cmp(&right.id).then(left.ty.cmp(&right.ty)));
        Ok(lowered)
    }
}

#[derive(Default)]
struct RawSourceInfo {
    declarations: Vec<RawDeclarationInfo>,
    field_labels: Vec<RawFieldLabelInfo>,
    methods: Vec<RawMethodInfo>,
    function_arguments: Vec<RawFunctionArgumentInfo>,
    actors: Vec<RawActorInfo>,
}

struct RawDeclarationInfo {
    source: String,
    name: String,
    ty: TypeRef,
    docs: Vec<String>,
}

struct RawActorInfo {
    source: String,
    docs: Vec<String>,
}

struct RawFieldLabelInfo {
    origin: SourceOrigin,
    path: String,
    container: TypeRef,
    id: u32,
    label: SourceLabel,
    docs: Vec<String>,
}

struct RawMethodInfo {
    origin: SourceOrigin,
    path: String,
    service: TypeRef,
    name: String,
    docs: Vec<String>,
}

struct RawFunctionArgumentInfo {
    origin: SourceOrigin,
    path: String,
    function: TypeRef,
    direction: SourceFunctionArgumentDirection,
    position: u32,
    name: String,
}

/// Walk the parser's source AST only for provenance. Every semantic type ref
/// comes from `ast_to_type` plus the checked environment, never from a
/// handwritten interpretation of Candid grammar or type rules.
fn actor_service_ref(
    actor: Option<&Actor>,
    lowerer: &Lowerer<'_>,
) -> Result<Option<TypeRef>, String> {
    match actor {
        None => Ok(None),
        Some(Actor::Service { service }) => Ok(Some(*service)),
        Some(Actor::Class { class }) => match lowerer.nodes[*class as usize].as_ref() {
            Some(TypeNode::Class { service, .. }) => Ok(Some(*service)),
            _ => Err("actor class was not fully lowered".to_string()),
        },
    }
}

fn collect_source_units_info(
    source_units: &[SourceUnit],
    actor_service: Option<TypeRef>,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    for unit in source_units {
        for declaration in &unit.program.decs {
            if let Dec::TypD(binding) = declaration {
                let origin = SourceOrigin::Declaration {
                    source: unit.name.clone(),
                    name: binding.id.clone(),
                };
                let ty = lowerer.lower_named(&binding.id)?;
                output.declarations.push(RawDeclarationInfo {
                    source: unit.name.clone(),
                    name: binding.id.clone(),
                    ty,
                    docs: binding.docs.clone(),
                });
                collect_type_source_info(
                    &binding.typ,
                    &origin,
                    &format!("type:{}", binding.id),
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
        if unit.include_actor {
            if let Some(actor) = &unit.program.actor {
                let origin = SourceOrigin::Actor {
                    source: unit.name.clone(),
                };
                output.actors.push(RawActorInfo {
                    source: unit.name.clone(),
                    docs: actor.docs.clone(),
                });
                collect_actor_source_info(
                    &actor.typ,
                    &origin,
                    actor_service,
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
    }
    Ok(())
}

fn collect_actor_source_info(
    ast: &IDLType,
    origin: &SourceOrigin,
    actor_service: Option<TypeRef>,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    match ast {
        IDLType::ServT(methods) => {
            if let Some(service) = actor_service {
                collect_service_source_info(
                    methods,
                    origin,
                    "actor",
                    service,
                    environment,
                    lowerer,
                    output,
                )?;
            }
        }
        IDLType::ClassT(init, service) => {
            for (position, argument) in init.iter().enumerate() {
                collect_type_source_info(
                    &argument.typ,
                    origin,
                    &format!("actor.init[{position}].type"),
                    environment,
                    lowerer,
                    output,
                )?;
            }
            if let IDLType::ServT(methods) = service.as_ref() {
                if let Some(service) = actor_service {
                    collect_service_source_info(
                        methods,
                        origin,
                        "actor.service",
                        service,
                        environment,
                        lowerer,
                        output,
                    )?;
                }
            }
        }
        IDLType::VarT(_) => {}
        other => {
            collect_type_source_info(other, origin, "actor", environment, lowerer, output)?;
        }
    }
    Ok(())
}

fn collect_type_source_info(
    ast: &IDLType,
    origin: &SourceOrigin,
    path: &str,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    let mut pending = vec![(ast, path.to_string())];
    while let Some((ast, path)) = pending.pop() {
        match ast {
            IDLType::RecordT(fields) | IDLType::VariantT(fields) => {
                let container = lower_ast_type(ast, environment, lowerer)?;
                for (position, field) in fields.iter().enumerate() {
                    let field_path = format!("{path}.fields[{position}]");
                    output.field_labels.push(RawFieldLabelInfo {
                        origin: origin.clone(),
                        path: field_path.clone(),
                        container,
                        id: field.label.get_id(),
                        label: lower_source_label(&field.label),
                        docs: field.docs.clone(),
                    });
                }
                for (position, field) in fields.iter().enumerate().rev() {
                    pending.push((&field.typ, format!("{path}.fields[{position}].type")));
                }
            }
            IDLType::FuncT(function) => {
                let function_ref = lower_ast_type(ast, environment, lowerer)?;
                for (position, argument) in function.args.iter().enumerate() {
                    record_function_argument_name(
                        origin,
                        &format!("{path}.args[{position}]"),
                        function_ref,
                        SourceFunctionArgumentDirection::Argument,
                        position,
                        argument.name.as_deref(),
                        output,
                    )?;
                }
                for (position, result) in function.rets.iter().enumerate() {
                    record_function_argument_name(
                        origin,
                        &format!("{path}.results[{position}]"),
                        function_ref,
                        SourceFunctionArgumentDirection::Result,
                        position,
                        result.name.as_deref(),
                        output,
                    )?;
                }
                for (position, result) in function.rets.iter().enumerate().rev() {
                    pending.push((&result.typ, format!("{path}.results[{position}].type")));
                }
                for (position, argument) in function.args.iter().enumerate().rev() {
                    pending.push((&argument.typ, format!("{path}.args[{position}].type")));
                }
            }
            IDLType::ServT(methods) => {
                let service = lower_ast_type(ast, environment, lowerer)?;
                for (position, method) in methods.iter().enumerate() {
                    let method_path = format!("{path}.methods[{position}]");
                    output.methods.push(RawMethodInfo {
                        origin: origin.clone(),
                        path: method_path,
                        service,
                        name: method.id.clone(),
                        docs: method.docs.clone(),
                    });
                }
                for (position, method) in methods.iter().enumerate().rev() {
                    pending.push((&method.typ, format!("{path}.methods[{position}].function")));
                }
            }
            IDLType::ClassT(init, service) => {
                pending.push((service, format!("{path}.service")));
                for (position, argument) in init.iter().enumerate().rev() {
                    pending.push((&argument.typ, format!("{path}.init[{position}].type")));
                }
            }
            IDLType::OptT(inner) | IDLType::VecT(inner) => {
                pending.push((inner, format!("{path}.inner")));
            }
            IDLType::PrimT(_) | IDLType::VarT(_) | IDLType::PrincipalT => {}
        }
    }
    Ok(())
}

fn collect_service_source_info(
    methods: &[candid_parser::syntax::Binding],
    origin: &SourceOrigin,
    path: &str,
    service: TypeRef,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    for (position, method) in methods.iter().enumerate() {
        let method_path = format!("{path}.methods[{position}]");
        output.methods.push(RawMethodInfo {
            origin: origin.clone(),
            path: method_path.clone(),
            service,
            name: method.id.clone(),
            docs: method.docs.clone(),
        });
        collect_type_source_info(
            &method.typ,
            origin,
            &format!("{method_path}.function"),
            environment,
            lowerer,
            output,
        )?;
    }
    Ok(())
}

fn lower_ast_type(
    ast: &IDLType,
    environment: &TypeEnv,
    lowerer: &mut Lowerer<'_>,
) -> Result<TypeRef, String> {
    let ty = ast_to_type(environment, ast).map_err(|error| error.to_string())?;
    lowerer.lower_type(&ty)
}

fn record_function_argument_name(
    origin: &SourceOrigin,
    path: &str,
    function: TypeRef,
    direction: SourceFunctionArgumentDirection,
    position: usize,
    name: Option<&str>,
    output: &mut RawSourceInfo,
) -> Result<(), String> {
    let Some(name) = name else {
        return Ok(());
    };
    output.function_arguments.push(RawFunctionArgumentInfo {
        origin: origin.clone(),
        path: path.to_string(),
        function,
        direction,
        position: u32::try_from(position).map_err(|_| "function argument position exceeds u32")?,
        name: name.to_string(),
    });
    Ok(())
}

fn lower_source_label(label: &Label) -> SourceLabel {
    match label {
        Label::Named(name) => SourceLabel::Named { name: name.clone() },
        Label::Id(_) => SourceLabel::Numeric,
        Label::Unnamed(_) => SourceLabel::Positional,
    }
}

fn primitive_from_type(ty: &TypeInner) -> Option<PrimitiveType> {
    Some(match ty {
        TypeInner::Null => PrimitiveType::Null,
        TypeInner::Bool => PrimitiveType::Bool,
        TypeInner::Nat => PrimitiveType::Nat,
        TypeInner::Int => PrimitiveType::Int,
        TypeInner::Nat8 => PrimitiveType::Nat8,
        TypeInner::Nat16 => PrimitiveType::Nat16,
        TypeInner::Nat32 => PrimitiveType::Nat32,
        TypeInner::Nat64 => PrimitiveType::Nat64,
        TypeInner::Int8 => PrimitiveType::Int8,
        TypeInner::Int16 => PrimitiveType::Int16,
        TypeInner::Int32 => PrimitiveType::Int32,
        TypeInner::Int64 => PrimitiveType::Int64,
        TypeInner::Float32 => PrimitiveType::Float32,
        TypeInner::Float64 => PrimitiveType::Float64,
        TypeInner::Text => PrimitiveType::Text,
        TypeInner::Reserved => PrimitiveType::Reserved,
        TypeInner::Empty => PrimitiveType::Empty,
        TypeInner::Principal => PrimitiveType::Principal,
        _ => return None,
    })
}

fn lower_mode(modes: &[FuncMode]) -> Result<MethodMode, String> {
    match modes {
        [] => Ok(MethodMode::Update),
        [FuncMode::Query] => Ok(MethodMode::Query),
        [FuncMode::CompositeQuery] => Ok(MethodMode::CompositeQuery),
        [FuncMode::Oneway] => Ok(MethodMode::Oneway),
        _ => Err("official Candid checker returned more than one function mode".to_string()),
    }
}

fn compare_field_label_provenance(
    left: &FieldLabelProvenance,
    right: &FieldLabelProvenance,
) -> std::cmp::Ordering {
    left.origin
        .cmp(&right.origin)
        .then(left.path.cmp(&right.path))
        .then(left.container.cmp(&right.container))
        .then(left.id.cmp(&right.id))
        .then(source_label_order(&left.label, &right.label))
        .then(left.docs.cmp(&right.docs))
}

fn compare_source_method_info(
    left: &SourceMethodInfo,
    right: &SourceMethodInfo,
) -> std::cmp::Ordering {
    left.origin
        .cmp(&right.origin)
        .then(left.path.cmp(&right.path))
        .then(left.service.cmp(&right.service))
        .then(left.name.cmp(&right.name))
        .then(left.docs.cmp(&right.docs))
}

fn compare_source_function_argument_info(
    left: &SourceFunctionArgumentInfo,
    right: &SourceFunctionArgumentInfo,
) -> std::cmp::Ordering {
    left.origin
        .cmp(&right.origin)
        .then(left.path.cmp(&right.path))
        .then(left.function.cmp(&right.function))
        .then(left.direction.cmp(&right.direction))
        .then(left.position.cmp(&right.position))
        .then(left.name.cmp(&right.name))
}

fn source_label_order(left: &SourceLabel, right: &SourceLabel) -> std::cmp::Ordering {
    let rank = |label: &SourceLabel| match label {
        SourceLabel::Named { .. } => 0,
        SourceLabel::Numeric => 1,
        SourceLabel::Positional => 2,
    };
    rank(left)
        .cmp(&rank(right))
        .then_with(|| match (left, right) {
            (SourceLabel::Named { name: left }, SourceLabel::Named { name: right }) => {
                left.cmp(right)
            }
            _ => std::cmp::Ordering::Equal,
        })
}
