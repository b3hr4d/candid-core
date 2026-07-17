use candid_core::{Limits, MemoryResolver, SourceId, SourceResolver, WorkspaceResolver};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
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
fn cli_validation_bounds_contract_files_before_decoding() {
    let fixture = Fixture::new();
    let limit = Limits::default().max_input_bytes;
    let fixture_json = fs::read_to_string(format!(
        "{}/tests/fixtures/conformance/empty_actor.contract.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut exact = fixture_json.into_bytes();
    exact.resize(limit, b' ');
    let exact_path = fixture.path.join("exact.json");
    fs::write(&exact_path, exact).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_candid-core"))
        .args(["validate", exact_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let over_path = fixture.path.join("over.json");
    fs::write(&over_path, vec![0xff; limit + 1]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_candid-core"))
        .args(["validate", over_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["violations"][0]["code"], "resource_limit_exceeded");
    assert_eq!(
        response["violations"][0]["resource_limit"],
        serde_json::json!({
            "resource": "input_bytes",
            "limit": limit,
            "observed": limit + 1,
        })
    );
}
