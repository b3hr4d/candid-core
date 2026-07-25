use candid_core::{
    compile_did_with_context, CompileOptions, HostValue, HostValueJsonError, Limits, RuntimeContext,
};
#[cfg(feature = "filesystem-compiler")]
use candid_core::{compile_with_resolver, MemoryResolver};
use std::process::Command;

#[cfg(not(windows))]
const SMALL_STACK_BYTES: usize = 64 * 1024;
// The Windows test runtime itself requires more than 64 KiB before the
// compiler preflight runs; 512 KiB remains well below its default stack.
#[cfg(windows)]
const SMALL_STACK_BYTES: usize = 512 * 1024;

fn nested_opts(depth: usize) -> String {
    format!("type T = {}nat; service : {{}};", "opt ".repeat(depth))
}

fn alias_chain(depth: usize) -> String {
    let mut source = String::from("type T0 = nat;\n");
    for index in 1..=depth {
        source.push_str(&format!("type T{index} = opt T{};\n", index - 1));
    }
    source.push_str(&format!("service : {{ f: (T{depth}) -> (); }};"));
    source
}

fn compile_with_limits(source: &str, limits: Limits) -> Result<(), candid_core::CompileError> {
    compile_did_with_context(
        source,
        CompileOptions {
            include_source_info: true,
        },
        &RuntimeContext::new(limits),
    )
    .map(|_| ())
}

/// `depth` nested `opt` wrappers around a `null`, as the portable tagged ABI.
fn nested_opt_json(depth: usize) -> String {
    let mut json = String::new();
    for _ in 0..depth {
        json.push_str(r#"{"kind":"opt","value":"#);
    }
    json.push_str(r#"{"kind":"null"}"#);
    for _ in 0..depth {
        json.push('}');
    }
    json
}

#[cfg(feature = "filesystem-compiler")]
fn compile_imported_alias_chain() -> Result<(), candid_core::CompileError> {
    const FILES: usize = 40;
    const OPTS_PER_FILE: usize = 8;
    let mut resolver = MemoryResolver::new();
    resolver
        .insert(
            "root.did",
            r#"import "f0.did"; service : { read: () -> (T0) query };"#,
        )
        .unwrap();
    for index in 0..FILES {
        resolver
            .insert(
                format!("f{index}.did"),
                format!(
                    "import \"f{}.did\"; type T{index} = {}T{};",
                    index + 1,
                    "opt ".repeat(OPTS_PER_FILE),
                    index + 1
                ),
            )
            .unwrap();
    }
    resolver
        .insert(format!("f{FILES}.did"), format!("type T{FILES} = nat;"))
        .unwrap();
    compile_with_resolver(
        "root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .map(|_| ())
}

#[test]
fn source_nesting_accepts_exact_limit_and_rejects_one_over() {
    let limits = Limits::default()
        .with_max_source_nesting(32)
        .with_max_type_depth(64);
    compile_with_limits(&nested_opts(32), limits.clone()).unwrap();

    let error = compile_with_limits(&nested_opts(33), limits).unwrap_err();
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_nesting");
    assert_eq!(resource.limit, 32);
    assert_eq!(resource.observed, 33);
}

#[test]
fn checked_type_depth_accepts_exact_limit_and_rejects_one_over() {
    let limits = Limits::default()
        .with_max_source_nesting(64)
        .with_max_type_depth(32);
    compile_with_limits(&nested_opts(32), limits.clone()).unwrap();

    let limits = limits.with_max_type_depth(31);
    let error = compile_with_limits(&nested_opts(32), limits).unwrap_err();
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_depth");
    assert_eq!(resource.limit, 31);
    assert_eq!(resource.observed, 32);
}

#[test]
fn default_stack_rejects_hostile_nesting_without_aborting() {
    let error = compile_with_limits(&nested_opts(3_000), Limits::default()).unwrap_err();
    assert_eq!(
        error.diagnostics[0]
            .resource_limit
            .as_ref()
            .unwrap()
            .resource,
        "source_nesting"
    );
}

#[test]
fn shallow_alias_chain_is_rejected_before_upstream_type_checking() {
    let error = compile_with_limits(&alias_chain(3_000), Limits::default()).unwrap_err();
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_depth");
    assert_eq!(resource.limit, 256);
    assert_eq!(resource.observed, 257);
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn imported_alias_chain_is_rejected_before_upstream_type_checking() {
    let error = compile_imported_alias_chain().unwrap_err();
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_depth");
    assert_eq!(resource.limit, 256);
    assert_eq!(resource.observed, 257);
}

/// The JSON decode path is the one route where hostile nesting is reachable
/// from bytes alone, with no host Rust code involved.
///
/// This runs in a subprocess for the same reason the compiler case below does:
/// a stack overflow aborts the process outright and cannot be caught, so a
/// regression has to be observable as a failed child rather than as a killed
/// test binary.
///
/// What this asserts is that input nested *past* `max_value_nesting` is
/// rejected without recursing, which the constant-stack pre-scan guarantees in
/// every build profile. It deliberately does NOT decode a document at exactly
/// the limit on this stack: that decode does recurse, at a per-level cost that
/// depends on the build profile, and a debug build exhausts 64 KiB in single
/// digits. No fixed limit is safe in both profiles at this stack size, so the
/// guarantee is scoped to what the pre-scan can actually deliver.
/// `Limits::max_value_nesting` states the measured costs; they are deliberately
/// not repeated here, so the two cannot drift apart.
#[test]
fn small_stack_rejects_hostile_host_value_json_without_aborting() {
    if std::env::var_os("CANDID_CORE_DEEP_NESTING_JSON_CHILD").is_some() {
        let error = std::thread::Builder::new()
            .stack_size(SMALL_STACK_BYTES)
            .spawn(|| HostValue::from_json_with_limits(&nested_opt_json(3_000), &Limits::default()))
            .unwrap()
            .join()
            .expect("small-stack HostValue worker must not abort")
            .unwrap_err();

        let HostValueJsonError::ValueLimit {
            resource,
            limit,
            observed,
            ..
        } = error
        else {
            panic!("expected a resource limit, found {error:?}");
        };
        assert_eq!(resource, "value_nesting");
        assert_eq!(limit, Limits::default().max_value_nesting());
        assert_eq!(observed, limit + 1);
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("small_stack_rejects_hostile_host_value_json_without_aborting")
        .arg("--nocapture")
        .env("CANDID_CORE_DEEP_NESTING_JSON_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success(), "small-stack subprocess failed: {status}");
}

#[cfg(feature = "filesystem-compiler")]
#[test]
fn small_stack_rejects_hostile_nesting_without_aborting() {
    if std::env::var_os("CANDID_CORE_DEEP_NESTING_CHILD").is_some() {
        let handle = std::thread::Builder::new()
            .stack_size(SMALL_STACK_BYTES)
            .spawn(|| compile_with_limits(&nested_opts(3_000), Limits::default()))
            .unwrap();
        let error = handle
            .join()
            .expect("small-stack worker must not abort")
            .unwrap_err();
        assert_eq!(error.diagnostics[0].code, "resource_limit_exceeded");
        let imported = std::thread::Builder::new()
            .stack_size(SMALL_STACK_BYTES)
            .spawn(compile_imported_alias_chain)
            .unwrap()
            .join()
            .expect("small-stack imported worker must not abort")
            .unwrap_err();
        assert_eq!(imported.diagnostics[0].code, "resource_limit_exceeded");
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("small_stack_rejects_hostile_nesting_without_aborting")
        .arg("--nocapture")
        .env("CANDID_CORE_DEEP_NESTING_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success(), "small-stack subprocess failed: {status}");
}
