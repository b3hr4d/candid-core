use candid_core::{
    compile_did_with_context, compile_with_resolver, CompileOptions, Limits, MemoryResolver,
    RuntimeContext,
};
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
        &RuntimeContext { limits },
    )
    .map(|_| ())
}

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
    let limits = Limits {
        max_source_nesting: 32,
        max_type_depth: 64,
        ..Limits::default()
    };
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
    let limits = Limits {
        max_source_nesting: 64,
        max_type_depth: 32,
        ..Limits::default()
    };
    compile_with_limits(&nested_opts(32), limits.clone()).unwrap();

    let limits = Limits {
        max_type_depth: 31,
        ..limits
    };
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

#[test]
fn imported_alias_chain_is_rejected_before_upstream_type_checking() {
    let error = compile_imported_alias_chain().unwrap_err();
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_depth");
    assert_eq!(resource.limit, 256);
    assert_eq!(resource.observed, 257);
}

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
