use super::*;

pub(super) struct MaterializedBundle {
    pub(super) root: PathBuf,
    pub(super) entry: PathBuf,
}

impl MaterializedBundle {
    pub(super) fn new(
        units: &[SourceUnit],
        entry: &crate::SourceId,
        budget: &mut crate::budget::Budget<'_>,
    ) -> Result<Self, CompileError> {
        budget.checkpoint().map_err(|error| {
            budget_error(error, DiagnosticPhase::Load, "source materialization")
        })?;
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed);
        let root = std::env::temp_dir().join(format!("candid-core-{}-{id}", std::process::id()));
        let indexes = units
            .iter()
            .enumerate()
            .map(|(index, unit)| {
                let id = crate::SourceId::parse(&unit.name)
                    .map_err(crate::ResolveError::into_compile_error)?;
                Ok((id, index))
            })
            .collect::<Result<BTreeMap<_, _>, CompileError>>()?;
        let entry_index = indexes.get(entry).copied().ok_or_else(|| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                "entry source is missing from the resolved bundle",
            )
        })?;
        create_private_dir(&root).map_err(|error| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                format!("cannot create isolated source bundle: {error}"),
            )
        })?;
        let bundle = Self {
            entry: root.join(format!("{entry_index}.did")),
            root,
        };
        for (index, unit) in units.iter().enumerate() {
            budget.checkpoint().map_err(|error| {
                budget_error(error, DiagnosticPhase::Load, "source materialization")
            })?;
            let path = bundle.root.join(format!("{index}.did"));
            let source = materialized_source(unit, &indexes, budget)?;
            fs::write(&path, source).map_err(|error| {
                CompileError::single(
                    "did_materialize_error",
                    DiagnosticPhase::Load,
                    format!("cannot materialize source {:?}: {error}", unit.name),
                )
            })?;
        }
        Ok(bundle)
    }
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn materialized_source(
    unit: &SourceUnit,
    indexes: &BTreeMap<crate::SourceId, usize>,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<String, CompileError> {
    let mut source = String::new();
    for import in &unit.imports {
        let target = indexes.get(&import.target).copied().ok_or_else(|| {
            CompileError::single(
                "did_materialize_error",
                DiagnosticPhase::Load,
                format!(
                    "resolved import {:?} is missing from the source bundle",
                    import.target.as_str()
                ),
            )
        })?;
        match import.kind {
            SourceImportKind::Type => source.push_str(&format!("import \"{target}.did\";\n")),
            SourceImportKind::Service => {
                source.push_str(&format!("import service \"{target}.did\";\n"));
            }
        }
    }
    let program = parse_program(&unit.source, Some(unit.name.clone()), budget)?;
    source.push_str(&pretty_print(&IDLMergedProg::new(program)));
    Ok(source)
}

impl Drop for MaterializedBundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn materialized_bundle_root_is_private_and_self_cleaning() {
        use std::os::unix::fs::PermissionsExt;

        let source = "service : {};";
        let entry = crate::SourceId::parse("memory:/private.did").unwrap();
        let limits = crate::Limits::default();
        let mut budget = crate::budget::Budget::from_limits(&limits);
        let unit = SourceUnit {
            name: entry.as_str().to_string(),
            source: source.to_string(),
            program: parse_program(source, Some(entry.as_str().to_string()), &mut budget).unwrap(),
            imports: Vec::new(),
            include_actor: true,
        };
        let bundle = MaterializedBundle::new(&[unit], &entry, &mut budget).unwrap();
        let root = bundle.root.clone();
        let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        drop(bundle);
        assert!(!root.exists());
    }
}
