use super::*;
// Imported here rather than through `super::*` so the parent module never
// needs a filesystem-only import in a `compiler`-only build.
use candid_parser::syntax::pretty_print;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

pub(super) struct MaterializedBundle {
    pub(super) root: PathBuf,
    pub(super) entry: PathBuf,
    /// Logical source ID of every materialized `{index}.did`, in unit order.
    /// This is the map back across the `check_file` boundary: upstream errors
    /// only ever name the numeric files, never the logical sources.
    logical_names: Vec<String>,
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
            logical_names: units.iter().map(|unit| unit.name.clone()).collect(),
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

impl MaterializedBundle {
    /// Rewrite materialized file identities in an upstream message back to
    /// their logical source IDs.
    ///
    /// Every rewrite is anchored to a complete upstream message template,
    /// never to a bare quoted `"N.did"`. Candid permits text field labels and
    /// method names such as `"0.did"`, and upstream messages render user
    /// labels raw-quoted (only their *content* is escaped), so a bare quoted
    /// pattern can match rendered user text. The template words, in contrast,
    /// cannot appear unquoted inside rendered user content, and escaping
    /// prevents rendered content from reproducing an unescaped quoted name
    /// directly behind them. `candid_parser` 0.4.0 (pinned exactly) embeds a
    /// materialized identity in exactly four templates, audited at that pin:
    /// `Cannot import {file:?}` and `Cannot open {file:?}` (typing.rs), and
    /// `Imported service file "{name}" has no main service` / `… has a
    /// service constructor` (syntax/mod.rs). The path template is computed
    /// from the bundle's actual root (`{:?}` on the same `PathBuf`, so Windows
    /// escaping matches by construction).
    ///
    /// Replacements cannot cascade: the inserted logical ID is `{:?}`-escaped,
    /// so any quote it contains is `\"` and can never complete a later
    /// template, and canonical IDs always contain `:/` so an inserted ID can
    /// never itself spell `N.did` directly after a template's opening quote.
    ///
    /// Returns the rewritten message plus the logical IDs that were
    /// referenced, in unit order without duplicates.
    pub(super) fn map_materialized_names(&self, message: String) -> (String, Vec<&str>) {
        let mut message = message;
        let mut referenced = Vec::new();
        for (index, name) in self.logical_names.iter().enumerate() {
            let logical = format!("{name:?}");
            let quoted_name = format!("\"{index}.did\"");
            let mut hit = false;
            for template in [
                format!("Cannot import {quoted_name}"),
                format!("Imported service file {quoted_name} has no main service"),
                format!("Imported service file {quoted_name} has a service constructor"),
            ] {
                if message.contains(&template) {
                    message = message.replace(&template, &template.replace(&quoted_name, &logical));
                    hit = true;
                }
            }
            let path_template = format!("Cannot open {:?}", self.root.join(format!("{index}.did")));
            if message.contains(&path_template) {
                message = message.replace(&path_template, &format!("Cannot open {logical}"));
                hit = true;
            }
            if hit {
                referenced.push(name.as_str());
            }
        }
        (message, referenced)
    }

    /// Test-only constructor so span/message policy tests can exercise the
    /// mapping without touching the filesystem.
    #[cfg(test)]
    pub(super) fn for_tests(root: PathBuf, logical_names: Vec<String>) -> Self {
        Self {
            entry: root.join("0.did"),
            root,
            logical_names,
        }
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

#[cfg(test)]
mod name_mapping_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A root that never exists on disk, so the bundle's `Drop` cleanup is a
    /// no-op instead of deleting a directory some other process may own.
    fn bundle_with_names(names: &[&str]) -> MaterializedBundle {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "candid-core-map-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        MaterializedBundle::for_tests(root, names.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn quoted_numeric_names_map_to_logical_ids() {
        let bundle = bundle_with_names(&["memory:/entry.did", "memory:/lib.did"]);
        let (message, referenced) = bundle.map_materialized_names(
            "Imported service file \"1.did\" has no main service".to_string(),
        );
        assert_eq!(
            message,
            "Imported service file \"memory:/lib.did\" has no main service"
        );
        assert_eq!(referenced, vec!["memory:/lib.did"]);
    }

    #[test]
    fn full_temp_paths_map_to_logical_ids() {
        let bundle = bundle_with_names(&["memory:/entry.did"]);
        let path = bundle.root.join("0.did");
        let (message, referenced) = bundle.map_materialized_names(format!("Cannot open {path:?}"));
        assert_eq!(message, "Cannot open \"memory:/entry.did\"");
        assert_eq!(referenced, vec!["memory:/entry.did"]);
    }

    #[test]
    fn index_ten_never_matches_index_one() {
        let names: Vec<String> = (0..11)
            .map(|index| format!("memory:/u{index}.did"))
            .collect();
        let bundle = bundle_with_names(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let (message, referenced) =
            bundle.map_materialized_names("Cannot import \"10.did\"".to_string());
        assert_eq!(message, "Cannot import \"memory:/u10.did\"");
        assert_eq!(referenced, vec!["memory:/u10.did"]);
    }

    #[test]
    fn unrelated_text_is_left_untouched() {
        let bundle = bundle_with_names(&["memory:/entry.did", "memory:/lib.did"]);
        let original = "Unbound type identifier T2; method '0.did' is odd; see 3.did".to_string();
        let (message, referenced) = bundle.map_materialized_names(original.clone());
        // "0.did"/"3.did" appear without the exact quoted spelling candid_parser
        // uses for file identities, so nothing may be rewritten.
        assert_eq!(message, original);
        assert!(referenced.is_empty());
    }

    #[test]
    fn quoted_user_labels_are_never_rewritten() {
        // Candid permits text field labels, and upstream renders them
        // raw-quoted: `not a service type: record { "0.did" : nat }`. A bare
        // quoted pattern would rewrite the user's own label into a source ID
        // and fabricate a span; the template anchor must leave it alone.
        let bundle = bundle_with_names(&["memory:/entry.did", "memory:/lib.did"]);
        for original in [
            "not a service type: record { \"0.did\" : nat }",
            "not a function type: record { \"1.did\" : nat }",
            "not a service type: RecordT([TypeField { label: Named(\"1.did\"), .. }])",
        ] {
            let (message, referenced) = bundle.map_materialized_names(original.to_string());
            assert_eq!(message, original);
            assert!(referenced.is_empty(), "no span may be fabricated");
        }
    }

    #[test]
    fn inserted_logical_ids_never_complete_a_later_template() {
        // A source ID may legally contain a double quote. Its `{:?}`-escaped
        // insertion must not let a later index's template match inside the
        // replacement text and splice two identities together.
        let bundle = bundle_with_names(&["memory:/entry.did", "memory:/a\"2.did", "memory:/b.did"]);
        let (message, referenced) = bundle.map_materialized_names(
            "Imported service file \"1.did\" has no main service".to_string(),
        );
        assert_eq!(
            message,
            "Imported service file \"memory:/a\\\"2.did\" has no main service"
        );
        assert_eq!(referenced, vec!["memory:/a\"2.did"]);
    }

    #[test]
    fn multiple_references_are_reported_in_unit_order() {
        let bundle = bundle_with_names(&["memory:/entry.did", "memory:/lib.did"]);
        let (message, referenced) = bundle.map_materialized_names(
            "Cannot import \"1.did\" while Cannot import \"0.did\"".to_string(),
        );
        assert_eq!(
            message,
            "Cannot import \"memory:/lib.did\" while Cannot import \"memory:/entry.did\""
        );
        assert_eq!(referenced, vec!["memory:/entry.did", "memory:/lib.did"]);
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
