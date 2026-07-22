use candid_core::{
    compile_with_resolver, CompileOptions, Limits, MemoryResolver, RawSourceInfo, ResolveError,
    ResolvedSource, RuntimeContext, SourceId, SourceInfo, SourceResolver, WorkspaceResolver,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "candid-core-input-bounds-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn memory_resolver_checks_borrowed_sources_before_cloning() {
    let mut resolver = MemoryResolver::new();
    resolver.insert("exact.did", "1234").unwrap();
    resolver.insert("over.did", "12345").unwrap();
    let limits = Limits {
        max_source_bytes: 4,
        ..Limits::default()
    };

    assert_eq!(
        resolver
            .load(&SourceId::parse("exact.did").unwrap(), &limits)
            .unwrap()
            .source,
        "1234"
    );
    let error = resolver
        .load(&SourceId::parse("over.did").unwrap(), &limits)
        .unwrap_err();
    assert_eq!(error.code, "resource_limit_exceeded");
    assert_eq!(error.resource_limit.unwrap().observed, 5);
}

#[test]
fn workspace_resolver_bounds_reads_and_preserves_utf8_errors() {
    let fixture = Fixture::new();
    fs::write(fixture.path.join("exact.did"), b"1234").unwrap();
    fs::write(fixture.path.join("over.did"), b"12345").unwrap();
    fs::write(fixture.path.join("invalid.did"), [0xff]).unwrap();
    let resolver = WorkspaceResolver::new(&fixture.path).unwrap();
    let limits = Limits {
        max_source_bytes: 4,
        ..Limits::default()
    };

    assert_eq!(
        resolver
            .load(&SourceId::parse("workspace:/exact.did").unwrap(), &limits)
            .unwrap()
            .source,
        "1234"
    );
    let error = resolver
        .load(&SourceId::parse("workspace:/over.did").unwrap(), &limits)
        .unwrap_err();
    assert_eq!(error.code, "resource_limit_exceeded");
    let resource = error.resource_limit.unwrap();
    assert_eq!(resource.resource, "source_bytes");
    assert_eq!(resource.limit, 4);
    assert_eq!(resource.observed, 5);

    let error = resolver
        .load(&SourceId::parse("workspace:/invalid.did").unwrap(), &limits)
        .unwrap_err();
    assert_eq!(error.code, "did_file_read_error");
}

#[test]
fn compile_bounds_source_id_length_at_the_limit_and_one_over() {
    // A logical source ID is otherwise bounded only cumulatively by
    // `max_string_bytes`, so one megabyte-long path could slip through. The
    // resolver accounting seam enforces the per-ID limit before the source is
    // hashed or parsed.
    let entry = format!("{}.did", "a".repeat(64));
    let mut resolver = MemoryResolver::new();
    resolver.insert(&entry, "service : {};").unwrap();
    let id_len = SourceId::parse(&entry).unwrap().as_str().len();

    compile_with_resolver(
        &entry,
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_id_bytes: id_len,
            ..Limits::default()
        }),
    )
    .unwrap_or_else(|error| panic!("an ID exactly at the limit must compile: {error:#?}"));

    let error = compile_with_resolver(
        &entry,
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_id_bytes: id_len - 1,
            ..Limits::default()
        }),
    )
    .expect_err("an ID one byte over the limit must be rejected");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_id_bytes");
    assert_eq!(resource.limit, id_len - 1);
    assert_eq!(resource.observed, id_len);
}

/// Maps one aliased import spelling to a fixed canonical target and delegates
/// everything else to a `MemoryResolver`.
struct AliasResolver {
    inner: MemoryResolver,
    alias: String,
    target: SourceId,
}

impl SourceResolver for AliasResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        if from.is_some() && import == self.alias {
            return Ok(self.target.clone());
        }
        self.inner.identify(from, import)
    }

    fn load(&self, id: &SourceId, limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        self.inner.load(id, limits)
    }
}

#[test]
fn compile_bounds_import_spellings_that_resolve_to_short_targets() {
    // A resolver may alias a long import spelling to a short canonical target.
    // The spelling itself is emitted verbatim into `SourceInfo.imports`, where
    // the sidecar preflight bounds it with `max_source_id_bytes`, so the
    // compiler must enforce the same bound — otherwise it can emit provenance
    // that validation under the very same limits rejects.
    let alias = format!("alias/{}", "a".repeat(58));
    let root = format!("import \"{alias}\";\nservice : {{}};");
    let mut inner = MemoryResolver::new();
    inner.insert("root.did", root).unwrap();
    inner.insert("dep.did", "type Item = nat;").unwrap();
    let resolver = AliasResolver {
        inner,
        alias: alias.clone(),
        target: SourceId::parse("memory:/dep.did").unwrap(),
    };

    let at_limit = Limits {
        max_source_id_bytes: alias.len(),
        ..Limits::default()
    };
    let compilation = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(at_limit.clone()),
    )
    .unwrap_or_else(|error| panic!("a spelling exactly at the limit must compile: {error:#?}"));

    let raw: RawSourceInfo =
        serde_json::from_value(serde_json::to_value(compilation.source_info().unwrap()).unwrap())
            .unwrap();
    SourceInfo::try_from_raw_with_context(
        raw,
        compilation.contract(),
        &RuntimeContext::new(at_limit),
    )
    .unwrap_or_else(|error| {
        panic!("compiler-emitted provenance must validate under the same limits: {error:#?}")
    });

    let error = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_id_bytes: alias.len() - 1,
            ..Limits::default()
        }),
    )
    .expect_err("a spelling one byte over the limit must fail during compilation");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_id_bytes");
    assert_eq!(resource.limit, alias.len() - 1);
    assert_eq!(resource.observed, alias.len());
}

#[test]
fn compile_reports_downstream_source_limits_before_import_spelling_bytes() {
    // The spelling bound is terminal for the whole load pass, mirroring the
    // sidecar preflight: an imported source that was already invalid under a
    // pre-existing byte or count limit keeps reporting that resource even when
    // the spelling that reaches it is also over the limit.
    let alias = format!("alias/{}", "a".repeat(58));
    let root = format!("import \"{alias}\";\nservice : {{}};");
    let dep_source = format!("type Item = nat; // {}", "d".repeat(180));
    assert!(dep_source.len() > root.len());
    let mut inner = MemoryResolver::new();
    inner.insert("root.did", &root).unwrap();
    inner.insert("dep.did", &dep_source).unwrap();
    let resolver = AliasResolver {
        inner,
        alias: alias.clone(),
        target: SourceId::parse("memory:/dep.did").unwrap(),
    };

    let error = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_source_bytes: dep_source.len() - 1,
            max_source_id_bytes: alias.len() - 1,
            ..Limits::default()
        }),
    )
    .expect_err("the oversized imported source must be rejected");
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_bytes");
    assert_eq!(resource.observed, dep_source.len());

    let error = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_sources: 1,
            max_source_id_bytes: alias.len() - 1,
            ..Limits::default()
        }),
    )
    .expect_err("the second source must be rejected");
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "sources");
    assert_eq!(resource.observed, 2);
}

#[test]
fn compile_reports_sources_before_source_id_bytes_on_mixed_invalid_input() {
    // A source that both exceeds the source count and carries an oversized
    // resolved ID must keep reporting the pre-existing `sources` limit; the
    // newer `source_id_bytes` check is terminal in `accept_source`.
    let dep = format!("{}.did", "d".repeat(64));
    let root = format!("import \"{dep}\";\nservice : {{}};");
    let mut resolver = MemoryResolver::new();
    resolver.insert("root.did", root).unwrap();
    resolver.insert(&dep, "type Item = nat;").unwrap();
    // Over the limit as a resolved `memory:/` ID while the raw import spelling
    // stays within it, so only the accounting-seam ordering is exercised.
    let dep_id_len = SourceId::parse(&dep).unwrap().as_str().len();

    let error = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_sources: 1,
            max_source_id_bytes: dep_id_len - 1,
            ..Limits::default()
        }),
    )
    .expect_err("the second source must be rejected");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "sources");
    assert_eq!(resource.limit, 1);
    assert_eq!(resource.observed, 2);
}

#[test]
fn compile_reports_bundle_bytes_before_source_id_bytes_on_mixed_invalid_input() {
    // A source that both overflows the cumulative bundle budget and carries an
    // oversized resolved ID must keep reporting `bundle_bytes`.
    let dep = format!("{}.did", "d".repeat(64));
    let root = format!("import \"{dep}\";\nservice : {{}};");
    let dep_source = "type Item = nat;";
    let mut resolver = MemoryResolver::new();
    resolver.insert("root.did", &root).unwrap();
    resolver.insert(&dep, dep_source).unwrap();
    let dep_id_len = SourceId::parse(&dep).unwrap().as_str().len();
    let bundle_bytes = root.len() + dep_source.len();

    let error = compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::new(Limits {
            max_bundle_bytes: bundle_bytes - 1,
            max_source_id_bytes: dep_id_len - 1,
            ..Limits::default()
        }),
    )
    .expect_err("the bundle overflow must be rejected");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "bundle_bytes");
    assert_eq!(resource.limit, bundle_bytes - 1);
    assert_eq!(resource.observed, bundle_bytes);
}
