//! Boundary and adversarial coverage for SourceInfo provenance accounting.
//!
//! Every provenance category is exercised at exactly its limit (accepted) and
//! one entry over it (rejected with stable resource metadata). The oversized
//! cases present a tampered sidecar on purpose: a small source bundle must not
//! be able to carry an arbitrarily large derived sidecar, and the limit has to
//! fire before rederivation or remapping walks the inflated collection.

use candid_core::{
    compile_did, compile_with_resolver, CancellationToken, Compilation, CompileOptions, Limits,
    MemoryResolver, RawContract, RawSourceInfo, RuntimeContext, SourceInfo,
};

const ROOT: &str = r#"import "types.did";
/// Root service documentation.
/// Second root line.
service : {
  /// Ping documentation.
  /// Ping second line.
  ping: (name: text, tag: nat) -> (item: Item) query;
  /// Read documentation.
  read: (id: nat) -> (label: text);
};"#;

const TYPES: &str = r#"/// Item documentation.
/// Item second line.
type Item = record {
  /// Identifier documentation.
  id: nat;
  /// Label documentation.
  label: text;
};"#;

/// A two-source bundle that populates every provenance collection.
fn bundle() -> Compilation {
    let mut resolver = MemoryResolver::new();
    resolver.insert("root.did", ROOT).unwrap();
    resolver.insert("types.did", TYPES).unwrap();
    compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .unwrap()
}

fn raw_of(compilation: &Compilation) -> RawSourceInfo {
    serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
        .unwrap()
}

/// Duplicates the final entry until `items` reaches `target`.
fn grow<T: Clone>(items: &mut Vec<T>, target: usize) {
    let template = items
        .last()
        .expect("fixture must populate this collection")
        .clone();
    while items.len() < target {
        items.push(template.clone());
    }
}

fn accept(raw: RawSourceInfo, compilation: &Compilation, limits: Limits) {
    SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::new(limits),
    )
    .unwrap_or_else(|error| panic!("expected acceptance at the exact limit: {error:#?}"));
}

/// Returns `(resource, limit, observed)` from the first violation.
fn reject(raw: RawSourceInfo, compilation: &Compilation, limits: Limits) -> (String, usize, usize) {
    let error = SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::new(limits),
    )
    .expect_err("oversized provenance must be rejected");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded", "{error:#?}");
    let info = violation
        .resource_limit
        .as_ref()
        .expect("resource limit failures must retain metadata");
    (info.resource.clone(), info.limit, info.observed)
}

/// Smallest `max_provenance_work` under which the same validation succeeds.
///
/// `provenance_work` is charged cumulatively across the rederived and presented
/// structural passes, so lowering only this counter isolates the target-scan
/// work from every other limit.
fn minimum_provenance_work(validate: impl Fn(&Limits) -> bool) -> usize {
    let ceiling = Limits::default().max_provenance_work;
    assert!(
        validate(&Limits::default()),
        "the probe must succeed at the default ceiling"
    );
    let (mut low, mut high) = (0usize, ceiling);
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = Limits {
            max_provenance_work: mid,
            ..Limits::default()
        };
        if validate(&limits) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

#[test]
fn sources_are_accepted_at_the_limit_and_rejected_one_over() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let observed = raw.sources.len();
    assert_eq!(observed, 2);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_sources: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.sources, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_sources: observed,
                ..Limits::default()
            }
        ),
        ("sources".to_string(), observed, observed + 1)
    );
}

#[test]
fn import_edges_are_accepted_at_the_limit_and_rejected_one_over() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let observed = raw.imports.len();
    assert_eq!(observed, 1);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_import_edges: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.imports, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_import_edges: observed,
                ..Limits::default()
            }
        ),
        ("import_edges".to_string(), observed, observed + 1)
    );
}

#[test]
fn source_declarations_are_accepted_at_the_limit_and_rejected_one_over() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let observed = raw.declarations.len();
    assert_eq!(observed, 1);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_declarations: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.declarations, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_declarations: observed,
                ..Limits::default()
            }
        ),
        ("source_declarations".to_string(), observed, observed + 1)
    );
}

#[test]
fn source_actors_are_accepted_at_the_limit_and_rejected_one_over() {
    // `source_actors` shares `max_sources`, and a compiled bundle never has more
    // actors than sources, so a single-source bundle is required to place the
    // actor count exactly at the shared limit.
    let compilation = compile_did("service : {};").unwrap();
    let raw = raw_of(&compilation);
    let observed = raw.actors.len();
    assert_eq!(observed, 1);
    assert_eq!(raw.sources.len(), 1);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_sources: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.actors, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_sources: observed,
                ..Limits::default()
            }
        ),
        ("source_actors".to_string(), observed, observed + 1)
    );
}

#[test]
fn source_field_labels_are_accepted_at_the_limit_and_rejected_one_over() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let observed = raw.field_labels.len();
    assert_eq!(observed, 2);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_fields: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.field_labels, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_fields: observed,
                ..Limits::default()
            }
        ),
        ("source_field_labels".to_string(), observed, observed + 1)
    );
}

#[test]
fn source_methods_are_accepted_at_the_limit_and_rejected_one_over() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let observed = raw.methods.len();
    assert_eq!(observed, 2);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_methods: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.methods, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_methods: observed,
                ..Limits::default()
            }
        ),
        ("source_methods".to_string(), observed, observed + 1)
    );
}

#[test]
fn source_function_arguments_are_accepted_at_the_limit_and_rejected_one_over() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    // Every argument and result is named so the provenance count matches the
    // structural function-value count that shares `max_function_values`.
    let observed = raw.function_arguments.len();
    assert_eq!(observed, 5);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_function_values: observed,
            ..Limits::default()
        },
    );

    let mut inflated = raw;
    grow(&mut inflated.function_arguments, observed + 1);
    assert_eq!(
        reject(
            inflated,
            &compilation,
            Limits {
                max_function_values: observed,
                ..Limits::default()
            }
        ),
        (
            "source_function_arguments".to_string(),
            observed,
            observed + 1
        )
    );
}

#[test]
fn source_string_bytes_are_accepted_at_the_limit_and_rejected_one_under() {
    let compilation = bundle();
    let raw = raw_of(&compilation);

    // Learn the genuine totals from the failure metadata rather than restating
    // the accounting rule in the test. The cheap documentation-cardinality
    // bound reports first, so a second probe above it reveals the full total.
    let (resource, _, doc_entries) = reject(
        raw.clone(),
        &compilation,
        Limits {
            max_string_bytes: 0,
            ..Limits::default()
        },
    );
    assert_eq!(resource, "source_string_bytes");
    assert!(doc_entries > 0);

    let (resource, _, observed) = reject(
        raw.clone(),
        &compilation,
        Limits {
            max_string_bytes: doc_entries,
            ..Limits::default()
        },
    );
    assert_eq!(resource, "source_string_bytes");
    assert!(observed > doc_entries);

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_string_bytes: observed,
            ..Limits::default()
        },
    );

    assert_eq!(
        reject(
            raw,
            &compilation,
            Limits {
                max_string_bytes: observed - 1,
                ..Limits::default()
            }
        ),
        ("source_string_bytes".to_string(), observed - 1, observed)
    );
}

#[test]
fn empty_documentation_entries_cannot_inflate_a_sidecar_for_free() {
    // Documentation cardinality is the one provenance category not implied by a
    // collection length. Without a per-entry cost, a sidecar carrying millions
    // of empty strings would contribute zero string bytes and pass every count
    // limit while still forcing the traversal and allocation.
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let genuine_docs: usize = raw
        .declarations
        .iter()
        .map(|entry| entry.docs.len())
        .chain(raw.actors.iter().map(|entry| entry.docs.len()))
        .chain(raw.field_labels.iter().map(|entry| entry.docs.len()))
        .chain(raw.methods.iter().map(|entry| entry.docs.len()))
        .sum();

    let padding = 4096;
    let mut inflated = raw;
    let replaced = inflated.declarations[0].docs.len();
    inflated.declarations[0].docs = vec![String::new(); padding];
    let expected_entries = genuine_docs - replaced + padding;

    let limit = 1024;
    assert!(limit < expected_entries);
    let (resource, reported_limit, observed) = reject(
        inflated,
        &compilation,
        Limits {
            max_string_bytes: limit,
            ..Limits::default()
        },
    );
    assert_eq!(resource, "source_string_bytes");
    assert_eq!(reported_limit, limit);
    assert_eq!(observed, expected_entries);
}

#[test]
fn provenance_limits_are_enforced_before_reference_remapping() {
    // `Compilation::try_from_raw_with_context` remaps sidecar references before
    // validation runs. The remap must bound the collections it walks, so an
    // oversized sidecar has to fail on its limit rather than on a reference
    // error discovered part-way through the walk.
    let compilation = bundle();
    let mut inflated = raw_of(&compilation);
    let observed = inflated.field_labels.len();
    grow(&mut inflated.field_labels, observed + 1);
    for label in &mut inflated.field_labels[observed..] {
        label.container = u32::MAX;
    }

    let error = Compilation::try_from_raw_with_context(
        RawContract::from(compilation.contract()),
        Some(inflated),
        &RuntimeContext::new(Limits {
            max_fields: observed,
            ..Limits::default()
        }),
    )
    .expect_err("oversized provenance must be rejected");
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded", "{error:#?}");
    let info = violation.resource_limit.as_ref().unwrap();
    assert_eq!(info.resource, "source_field_labels");
    assert_eq!(info.limit, observed);
    assert_eq!(info.observed, observed + 1);
}

#[test]
fn provenance_validation_observes_cancellation() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let token = CancellationToken::new();
    token.cancel();
    let context = RuntimeContext::new(Limits::default()).with_cancellation(token);

    let error = SourceInfo::try_from_raw_with_context(raw, compilation.contract(), &context)
        .expect_err("cancellation must abort provenance validation");
    assert_eq!(error.violations[0].code, "operation_cancelled");
}

#[test]
fn provenance_validation_observes_elapsed_deadlines() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 1;
    let context = RuntimeContext::new(Limits {
        deadline_unix_ms: Some(elapsed),
        ..Limits::default()
    });

    let error = SourceInfo::try_from_raw_with_context(raw, compilation.contract(), &context)
        .expect_err("an elapsed deadline must abort provenance validation");
    assert_eq!(error.violations[0].code, "operation_deadline_exceeded");
}

#[test]
fn reference_remapping_observes_cancellation() {
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let token = CancellationToken::new();
    token.cancel();
    let context = RuntimeContext::new(Limits::default()).with_cancellation(token);

    let error = Compilation::try_from_raw_with_context(
        RawContract::from(compilation.contract()),
        Some(raw),
        &context,
    )
    .expect_err("cancellation must abort remapping");
    assert_eq!(error.violations[0].code, "operation_cancelled");
}

#[test]
fn provenance_target_resolution_is_charged_at_the_exact_boundary() {
    // Resolving every field-label and method provenance entry against its target
    // now charges `provenance_work`. The minimum that admits a valid bundle is
    // exactly the work the two structural passes charge; one unit under it must
    // fail closed with that resource, proving the traversal is charged rather
    // than free.
    let compilation = bundle();
    let raw = raw_of(&compilation);

    let min = minimum_provenance_work(|limits| {
        SourceInfo::try_from_raw_with_context(
            raw.clone(),
            compilation.contract(),
            &RuntimeContext::new(limits.clone()),
        )
        .is_ok()
    });
    assert!(min > 0, "a bundle with provenance must charge some work");

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_provenance_work: min,
            ..Limits::default()
        },
    );
    assert_eq!(
        reject(
            raw,
            &compilation,
            Limits {
                max_provenance_work: min - 1,
                ..Limits::default()
            }
        ),
        ("provenance_work".to_string(), min - 1, min)
    );
}

#[test]
fn field_label_fan_out_onto_one_container_is_charged_and_bounded() {
    // The load-bearing regression: a tampered sidecar that aims many field
    // labels at one aggregate used to run an uncharged, uninterruptible
    // `O(labels * fields)` scan before the rederivation mismatch rejected it.
    // Duplicate labels are permitted by design, so each re-paid a full scan.
    // Now the extra labels each charge `provenance_work`, so a budget that
    // admits the genuine bundle rejects the inflated one before the scan runs
    // to completion — and does so deterministically.
    let compilation = bundle();
    let genuine = raw_of(&compilation);

    let min_valid = minimum_provenance_work(|limits| {
        SourceInfo::try_from_raw_with_context(
            genuine.clone(),
            compilation.contract(),
            &RuntimeContext::new(limits.clone()),
        )
        .is_ok()
    });

    // Concentrate extra (duplicate) labels on the same real container. The count
    // stays well under `max_fields`, so `provenance_work`, not the count limit,
    // is what fails.
    let mut inflated = genuine;
    let observed = inflated.field_labels.len();
    grow(&mut inflated.field_labels, observed + 64);

    let limits = Limits {
        max_provenance_work: min_valid,
        ..Limits::default()
    };
    let (resource, limit, over) = reject(inflated.clone(), &compilation, limits.clone());
    assert_eq!(resource, "provenance_work");
    assert_eq!(limit, min_valid);
    assert!(
        over > min_valid,
        "the extra labels must consume charged work beyond the genuine bundle"
    );

    // Determinism: the same hostile sidecar fails identically on a second run.
    let repeat = reject(inflated, &compilation, limits);
    assert_eq!(repeat, (resource, limit, over));
}

#[test]
fn tampered_source_bundle_id_is_still_rejected() {
    // The redundant bundle-identity re-hash was removed from the structural
    // pass because `validate_source_bundle_identity` performs it first on the
    // presented path. This guards that the surviving check still rejects a
    // tampered `source_bundle_id`.
    let compilation = bundle();
    let mut raw = raw_of(&compilation);
    raw.source_bundle_id.push('0');

    let error = SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::default(),
    )
    .expect_err("a tampered source_bundle_id must be rejected");
    assert_eq!(error.violations[0].code, "source_bundle_id_mismatch");
}

#[test]
fn source_id_length_is_bounded_at_the_limit_and_one_over() {
    // A logical source ID is otherwise bounded only cumulatively by
    // `max_string_bytes`. The sidecar path enforces the per-ID limit in the
    // preflight, before the bundle is hashed or rederived.
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let longest = raw
        .sources
        .iter()
        .map(|source| source.name.len())
        .max()
        .expect("the bundle has sources");

    accept(
        raw.clone(),
        &compilation,
        Limits {
            max_source_id_bytes: longest,
            ..Limits::default()
        },
    );
    assert_eq!(
        reject(
            raw,
            &compilation,
            Limits {
                max_source_id_bytes: longest - 1,
                ..Limits::default()
            }
        ),
        ("source_id_bytes".to_string(), longest - 1, longest)
    );
}

#[test]
fn remapping_does_not_double_count_cumulative_source_charges() {
    // `sources` and `import_edges` are charged cumulatively while rederiving the
    // bundle. Bounding them again before remapping would seed a high-water mark
    // that the later charge adds to, rejecting a bundle that is within its
    // limit, so remapping must only bound the collections it rewrites.
    let compilation = bundle();
    let raw = raw_of(&compilation);
    let sources = raw.sources.len();
    let imports = raw.imports.len();

    Compilation::try_from_raw_with_context(
        RawContract::from(compilation.contract()),
        Some(raw),
        &RuntimeContext::new(Limits {
            max_sources: sources,
            max_import_edges: imports,
            ..Limits::default()
        }),
    )
    .unwrap_or_else(|error| {
        panic!("a bundle exactly at its source limits must remain valid: {error:#?}")
    });
}
