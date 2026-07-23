//! Real-binary CLI contract tests.
//!
//! Every test runs the actual `candid-core` binary and pins the public
//! contract: the strict argument grammar, the 0/1/64 exit statuses, which
//! channel each response uses, the JSON shapes and stable codes, source-info
//! suppression, and the byte bounds at the exact limit and one byte over.

use candid_core::Limits;
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const DID_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/conformance/empty_actor.did"
);
const CONTRACT_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/conformance/empty_actor.contract.json"
);
const USAGE: &str =
    "usage: candid-core compile <path> [--no-source-info]\n       candid-core validate <path>\n";

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("candid-core-cli-{}-{sequence}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

fn run<I, S>(arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_candid-core"))
        .args(arguments)
        .output()
        .unwrap()
}

fn run_in<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_candid-core"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

/// Asserts the JSON response contract: the exact exit status, an empty
/// stderr, and exactly one pretty-printed JSON document on stdout.
fn json_stdout(output: &Output, expected_status: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(expected_status),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "JSON responses must keep stderr empty: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be a single JSON document");
    let mut expected = serde_json::to_string_pretty(&value).unwrap();
    expected.push('\n');
    assert_eq!(
        output.stdout,
        expected.into_bytes(),
        "stdout must be one pretty-printed JSON document followed by a newline",
    );
    value
}

fn assert_usage_error(output: &Output, context: &dyn std::fmt::Debug) {
    assert_eq!(
        output.status.code(),
        Some(64),
        "{context:?} must exit 64, stdout: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        output.stdout.is_empty(),
        "{context:?} must write nothing to stdout: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert_eq!(
        output.stderr,
        USAGE.as_bytes(),
        "{context:?} must print exactly the usage text, got: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn contract_fixture() -> Value {
    serde_json::from_str(&fs::read_to_string(CONTRACT_FIXTURE).unwrap()).unwrap()
}

#[test]
fn usage_errors_reject_every_invalid_argument_shape() {
    // Every argv uses only valid, existing inputs, so a run that reached file
    // processing would succeed; observing exit 64 therefore proves the
    // argument itself was rejected.
    let matrix: &[&[&str]] = &[
        // no arguments / unknown or misspelled commands
        &[],
        &["frobnicate"],
        &["frobnicate", DID_FIXTURE],
        &["COMPILE", DID_FIXTURE],
        // missing paths
        &["compile"],
        &["validate"],
        // options in the path position (misplaced or unknown options)
        &["compile", "--no-source-info"],
        &["compile", "--no-source-info", DID_FIXTURE],
        &["compile", "-x", DID_FIXTURE],
        &["validate", "--typo"],
        &["validate", "-"],
        &["validate", "--", CONTRACT_FIXTURE],
        // unknown options after the path
        &["compile", DID_FIXTURE, "--typo"],
        &["compile", DID_FIXTURE, "--no-source-info", "--typo"],
        &["validate", CONTRACT_FIXTURE, "--typo"],
        // options `validate` does not accept
        &["validate", CONTRACT_FIXTURE, "--no-source-info"],
        // duplicate flags
        &[
            "compile",
            DID_FIXTURE,
            "--no-source-info",
            "--no-source-info",
        ],
        // trailing arguments
        &["compile", DID_FIXTURE, "extra"],
        &["compile", DID_FIXTURE, "--no-source-info", "extra"],
        &["validate", CONTRACT_FIXTURE, "extra"],
    ];
    for arguments in matrix {
        assert_usage_error(&run(*arguments), arguments);
    }
}

#[test]
fn compile_emits_the_contract_and_source_info_by_default() {
    let response = json_stdout(&run(["compile", DID_FIXTURE]), 0);
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["contract"], contract_fixture());
    let source_info = response["source_info"]
        .as_object()
        .expect("source_info must be present by default");
    assert!(source_info.contains_key("source_bundle_id"));
    assert!(source_info.contains_key("sources"));
}

#[test]
fn compile_suppresses_source_info_to_exactly_null() {
    let response = json_stdout(&run(["compile", DID_FIXTURE, "--no-source-info"]), 0);
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["contract"], contract_fixture());
    let object = response.as_object().unwrap();
    assert_eq!(
        object.get("source_info"),
        Some(&Value::Null),
        "the key must stay present and be exactly null",
    );
}

#[test]
fn validate_emits_the_validated_contract() {
    let response = json_stdout(&run(["validate", CONTRACT_FIXTURE]), 0);
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["contract"], contract_fixture());
}

#[test]
fn dash_leading_paths_need_a_dot_slash_prefix() {
    let fixture = Fixture::new();
    fixture.write("-dashed.did", fs::read(DID_FIXTURE).unwrap());

    let bare = run_in(&fixture.path, ["compile", "-dashed.did"]);
    assert_usage_error(&bare, &"compile -dashed.did");

    let escaped = run_in(&fixture.path, ["compile", "./-dashed.did"]);
    let response = json_stdout(&escaped, 0);
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["contract"], contract_fixture());
}

#[test]
fn compile_reports_missing_input_as_json() {
    let fixture = Fixture::new();
    let missing = fixture.path.join("missing.did");
    let response = json_stdout(&run([OsStr::new("compile"), missing.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("did_file_read_error")
    );
    assert_eq!(response["diagnostics"][0]["phase"], json!("load"));
    assert_eq!(response["diagnostics"][0]["severity"], json!("error"));
}

#[test]
fn validate_reports_missing_input_as_json() {
    let fixture = Fixture::new();
    let missing = fixture.path.join("missing.json");
    let response = json_stdout(&run([OsStr::new("validate"), missing.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("contract_file_read_error")
    );
    assert_eq!(response["diagnostics"][0]["phase"], json!("load"));
}

#[test]
fn compile_reports_malformed_did_diagnostics() {
    let fixture = Fixture::new();
    let path = fixture.write("truncated.did", "service : {");
    let response = json_stdout(&run([OsStr::new("compile"), path.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["diagnostics"][0]["code"], json!("did_parse_error"));
    assert_eq!(response["diagnostics"][0]["phase"], json!("parse"));
    assert_eq!(response["diagnostics"][0]["severity"], json!("error"));
}

#[test]
fn validate_reports_malformed_json_diagnostics() {
    let fixture = Fixture::new();
    let path = fixture.write("truncated.json", "{");
    let response = json_stdout(&run([OsStr::new("validate"), path.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("malformed_contract_json")
    );
    assert_eq!(response["diagnostics"][0]["phase"], json!("load"));
}

#[test]
fn validate_reports_violations_for_an_invalid_contract() {
    let fixture = Fixture::new();
    let mut tampered = contract_fixture();
    tampered["identities"]["contract"] = json!("0".repeat(64));
    let path = fixture.write("tampered.json", serde_json::to_string(&tampered).unwrap());
    let response = json_stdout(&run([OsStr::new("validate"), path.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["violations"][0]["code"],
        json!("invalid_contract_id_format")
    );
    assert_eq!(
        response["violations"][0]["path"],
        json!("$.identities.contract")
    );
}

#[test]
fn validate_reports_invalid_utf8_within_the_limit() {
    let fixture = Fixture::new();
    let path = fixture.write("invalid.json", [0xff]);
    let response = json_stdout(&run([OsStr::new("validate"), path.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("contract_file_read_error")
    );
}

#[test]
fn compile_bounds_source_bytes_at_the_limit_and_one_over() {
    let fixture = Fixture::new();
    let limit = Limits::default().max_source_bytes();

    // Valid source padded with trailing whitespace to exactly the limit.
    let mut exact = fs::read(DID_FIXTURE).unwrap();
    assert!(exact.len() <= limit);
    exact.resize(limit, b' ');
    let exact_path = fixture.write("exact.did", exact);
    let response = json_stdout(&run([OsStr::new("compile"), exact_path.as_os_str()]), 0);
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["contract"], contract_fixture());

    // One byte over, and deliberately invalid as UTF-8 and as Candid: the
    // byte bound must fire before decoding or parsing can.
    let over_path = fixture.write("over.did", vec![0xff; limit + 1]);
    let response = json_stdout(&run([OsStr::new("compile"), over_path.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    let diagnostic = &response["diagnostics"][0];
    assert_eq!(diagnostic["code"], json!("resource_limit_exceeded"));
    assert_eq!(diagnostic["phase"], json!("load"));
    assert_eq!(
        diagnostic["resource_limit"],
        json!({
            "resource": "source_bytes",
            "limit": limit,
            "observed": limit + 1,
        })
    );
}

#[test]
fn validate_bounds_input_bytes_at_the_limit_and_one_over() {
    let fixture = Fixture::new();
    let limit = Limits::default().max_input_bytes();

    // Valid contract JSON padded with trailing whitespace to exactly the limit.
    let mut exact = fs::read(CONTRACT_FIXTURE).unwrap();
    assert!(exact.len() <= limit);
    exact.resize(limit, b' ');
    let exact_path = fixture.write("exact.json", exact);
    let response = json_stdout(&run([OsStr::new("validate"), exact_path.as_os_str()]), 0);
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["contract"], contract_fixture());

    // One byte over, and deliberately invalid as UTF-8 and as JSON: the byte
    // bound must fire before decoding or parsing can.
    let over_path = fixture.write("over.json", vec![0xff; limit + 1]);
    let response = json_stdout(&run([OsStr::new("validate"), over_path.as_os_str()]), 1);
    assert_eq!(response["ok"], json!(false));
    let violation = &response["violations"][0];
    assert_eq!(violation["code"], json!("resource_limit_exceeded"));
    assert_eq!(violation["path"], json!("$"));
    assert_eq!(
        violation["resource_limit"],
        json!({
            "resource": "input_bytes",
            "limit": limit,
            "observed": limit + 1,
        })
    );
}

/// `env::args`-based parsing would abort on a non-Unicode argument; the CLI
/// must instead accept the OS-native bytes and report an ordinary JSON error
/// on the normal channel: `validate` hands them to the filesystem, while
/// `compile` rejects a non-UTF-8 entry file name at the source-ID layer
/// before any I/O.
#[cfg(unix)]
#[test]
fn non_unicode_paths_are_reported_as_json_errors() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let path = OsString::from_vec(b"missing-\xff.json".to_vec());
    let response = json_stdout(&run([OsString::from("validate"), path]), 1);
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("contract_file_read_error")
    );

    let path = OsString::from_vec(b"missing-\xff.did".to_vec());
    let response = json_stdout(&run([OsString::from("compile"), path]), 1);
    assert_eq!(
        response["diagnostics"][0]["code"],
        json!("did_invalid_source_id")
    );
}

/// Issue #19: identities from the materialized `check_file` bundle must map
/// back to logical source IDs at the CLI boundary — never a numeric `N.did`,
/// a temp directory, or byte offsets into pretty-printed text — while the
/// failure envelope keys stay frozen.
#[test]
fn compile_maps_materialized_identities_back_to_logical_sources() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/imports/root_missing_service_actor.did"
    );
    let response = json_stdout(&run(["compile", fixture]), 1);
    assert_eq!(
        response.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["diagnostics", "ok"],
        "the compile failure envelope keys are frozen"
    );
    assert_eq!(response["ok"], json!(false));
    assert_eq!(
        response["diagnostics"],
        json!([{
            "code": "did_type_check_error",
            "phase": "type_check",
            "severity": "error",
            "message": "Imported service file \"workspace:/types.did\" has no main service",
            "span": { "source_name": "workspace:/types.did" },
        }])
    );
}

/// Issue #19: the validate failure envelope and its violation items keep the
/// legacy shape exactly — `violations` under `ok`, items carrying only
/// code/path/message (+ resource_limit when structured), never phase or
/// severity keys.
#[test]
fn validate_violation_envelope_and_item_shape_remain_frozen() {
    let mut contract = contract_fixture();
    contract["identities"]["contract"] = json!("not-a-valid-id");
    let fixture = Fixture::new();
    let path = fixture.write("contract.json", serde_json::to_string(&contract).unwrap());
    let response = json_stdout(&run([OsStr::new("validate"), path.as_os_str()]), 1);
    assert_eq!(
        response.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["ok", "violations"],
        "the validate failure envelope keys are frozen"
    );
    assert_eq!(
        response["violations"],
        json!([{
            "code": "invalid_contract_id_format",
            "path": "$.identities.contract",
            "message": "contract identity must use candid-core:contract:v1:sha256:<64 lowercase hex>",
        }])
    );
}
