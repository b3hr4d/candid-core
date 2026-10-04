use candid_core::{
    compile_did_with_context, CompileOptions, HostValue, HostValueJsonError, Limits, RuntimeContext,
};
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

/// `levels` chained records whose two fields both reference the next alias:
/// the issue #125 shape, where a walk without state deduplication re-expands
/// the shared subtree once per incoming edge and visits O(2^levels) nodes.
fn shared_fanout(levels: usize) -> String {
    let mut source = String::new();
    for index in 1..=levels {
        source.push_str(&format!(
            "type T{index} = record {{ a: T{}; b: T{} }};\n",
            index + 1,
            index + 1
        ));
    }
    source.push_str(&format!("type T{} = nat;\n", levels + 1));
    source.push_str("service : { go: (T1) -> () };");
    source
}

/// Issue #125: the two type-depth guard walks deduplicate shared subtrees
/// and charge `type_preflight_work`, so this compiles at a pinned cost of
/// 2 796 units — 1 398 per walk, pinned separately by the unit tests beside
/// each walk — where the un-deduplicated walks needed O(2^24) visits. The
/// exact one-under boundary is the linearity regression: any return to
/// per-path re-expansion overshoots it by four orders of magnitude.
#[test]
fn shared_subtree_fanout_compiles_at_its_pinned_work_ceiling() {
    let source = shared_fanout(24);
    compile_with_limits(
        &source,
        Limits::default().with_max_type_preflight_work(2_796),
    )
    .unwrap();

    let error = compile_with_limits(
        &source,
        Limits::default().with_max_type_preflight_work(2_795),
    )
    .unwrap_err();
    assert_eq!(error.diagnostics[0].code, "resource_limit_exceeded");
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_preflight_work");
    assert_eq!(resource.limit, 2_795);
    assert_eq!(resource.observed, 2_796);
}

/// Fan-out that genuinely exceeds the configured work budget fails closed
/// with the structured triple, not by hanging: the walk charges one unit per
/// step, so a budget of 100 is crossed at exactly 101.
#[test]
fn shared_subtree_fanout_exceeding_the_work_budget_fails_closed() {
    let error = compile_with_limits(
        &shared_fanout(24),
        Limits::default().with_max_type_preflight_work(100),
    )
    .unwrap_err();
    assert_eq!(error.diagnostics[0].code, "resource_limit_exceeded");
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_preflight_work");
    assert_eq!(resource.limit, 100);
    assert_eq!(resource.observed, 101);
}

/// Recursive types keep compiling unchanged next to shared fan-out: only
/// names on a reference cycle are tracked per path, and the walk still stops
/// exactly where the un-deduplicated walk stopped.
#[test]
fn recursive_types_still_compile_beside_shared_fanout() {
    let source = "type L = opt L;\n\
                  type Tree = variant { leaf: nat; node: record { left: Tree; right: Tree } };\n\
                  type T = record { a: L; b: L; t: Tree };\n\
                  service : { f: (T) -> (L) };";
    compile_with_limits(source, Limits::default()).unwrap();
}

/// The largest corpus fixture pins what a real contract consumes of the
/// issue #125 counter: 806 units end to end, four orders of magnitude under
/// the 10M default, exactly as `Limits::max_type_preflight_work` documents.
#[test]
fn ledger_corpus_compiles_at_its_pinned_work_ceiling() {
    let ledger = std::fs::read_to_string("benches/corpus/ledger.did").unwrap();
    compile_with_limits(&ledger, Limits::default().with_max_type_preflight_work(806)).unwrap();

    let error = compile_with_limits(&ledger, Limits::default().with_max_type_preflight_work(805))
        .unwrap_err();
    let resource = error.diagnostics[0].resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "type_preflight_work");
    assert_eq!(resource.limit, 805);
    assert_eq!(resource.observed, 806);
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

// Issue #219: comment runs.
//
// The pinned upstream `Tokenizer::next` skips a comment by calling itself, so
// a run of consecutive comments costs one stack frame each, in the source
// preflight's own token pass and again in the parser. `max_source_nesting`
// bounds the run with a constant-stack byte scan before either pass.

/// `//\n` lines in the largest source `max_source_bytes` admits:
/// 349 525 × 3 = 1 048 575 bytes, one byte under the 1 MiB default.
const MAX_COMMENT_LINES: usize = 349_525;

/// The stack the default-limit acceptance cases run on: Rust's default for a
/// spawned thread, which every `cargo test` worker is. A run at the default
/// limit needs about 800 KB of it in a debug build (the measured figures are
/// in `Limits::max_source_nesting`, and pinned below).
const COMMENT_RUN_STACK_BYTES: usize = 2 * 1024 * 1024;

fn comment_run(lines: usize, tail: &str) -> String {
    format!("{}{tail}", "//\n".repeat(lines))
}

/// The one refusal every entry point must produce for an over-long run.
fn assert_comment_run_refused(entry: &str, error: &candid_core::CompileError, limit: usize) {
    assert_eq!(error.diagnostics.len(), 1, "{entry}: {error:#?}");
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "resource_limit_exceeded", "{entry}");
    let resource = diagnostic.resource_limit.as_ref().unwrap();
    assert_eq!(resource.resource, "source_nesting", "{entry}");
    assert_eq!(resource.limit, limit as u64, "{entry}");
    assert_eq!(resource.observed, limit as u64 + 1, "{entry}");
}

/// Re-run one test of this binary as a child process with `marker` set, so a
/// stack overflow is observed as a failed child instead of killing the run.
fn run_in_child(test: &str, marker: &str, value: &str) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(test)
        .arg("--nocapture")
        .env(marker, value)
        .output()
        .unwrap()
}

fn assert_child_succeeded(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "filesystem-compiler")]
struct DidFile(std::path::PathBuf);

#[cfg(feature = "filesystem-compiler")]
impl DidFile {
    fn new(label: &str, source: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "candid-core-comment-run-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("entry.did");
        std::fs::write(&path, source).unwrap();
        Self(path)
    }
}

#[cfg(feature = "filesystem-compiler")]
impl Drop for DidFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

type EntryOutcome = (&'static str, Result<(), candid_core::CompileError>);

/// Compile `source` through every `.did` entry point under `limits`.
fn every_entry_point(source: &str, limits: &Limits, label: &str) -> Vec<EntryOutcome> {
    let context = RuntimeContext::new(limits.clone());
    let default_limits = *limits == Limits::default();
    let mut outcomes: Vec<EntryOutcome> = vec![
        (
            "compile_did_with_context",
            compile_did_with_context(source, CompileOptions::default(), &context).map(drop),
        ),
        ("compile_with_resolver", {
            let mut resolver = MemoryResolver::new();
            resolver.insert("entry.did", source).unwrap();
            compile_with_resolver("entry.did", &resolver, CompileOptions::default(), &context)
                .map(drop)
        }),
    ];
    if default_limits {
        outcomes.push(("compile_did", candid_core::compile_did(source).map(drop)));
        outcomes.push((
            "compile_did_with_options",
            candid_core::compile_did_with_options(source, CompileOptions::default()).map(drop),
        ));
    }
    #[cfg(feature = "filesystem-compiler")]
    {
        let file = DidFile::new(label, source);
        outcomes.push((
            "compile_did_file_with_context",
            candid_core::compile_did_file_with_context(
                &file.0,
                CompileOptions::default(),
                &context,
            )
            .map(drop),
        ));
        if default_limits {
            outcomes.push((
                "compile_did_file",
                candid_core::compile_did_file(&file.0).map(drop),
            ));
            outcomes.push((
                "compile_did_file_with_options",
                candid_core::compile_did_file_with_options(&file.0, CompileOptions::default())
                    .map(drop),
            ));
        }
    }
    #[cfg(not(feature = "filesystem-compiler"))]
    let _ = label;
    outcomes
}

/// Acceptance criterion 1 of issue #219: the largest comment run that fits
/// `max_source_bytes` is refused by every entry point on a 64 KiB stack,
/// where the upstream recursion overflows after about 18 comments in a debug
/// build. With the pre-scan removed this child aborts (SIGABRT, exit 134).
#[test]
fn small_stack_refuses_a_maximal_comment_run_on_every_entry_point() {
    const MARKER: &str = "CANDID_CORE_COMMENT_RUN_CHILD";
    if std::env::var_os(MARKER).is_some() {
        let source = comment_run(MAX_COMMENT_LINES, "");
        assert_eq!(source.len(), 1_048_575);
        assert!(source.len() <= Limits::default().max_source_bytes());
        let outcomes = std::thread::Builder::new()
            .stack_size(SMALL_STACK_BYTES)
            .spawn(move || every_entry_point(&source, &Limits::default(), "maximal"))
            .unwrap()
            .join()
            .expect("small-stack comment-run worker must not abort");
        let expected = if cfg!(feature = "filesystem-compiler") {
            7
        } else {
            4
        };
        assert_eq!(outcomes.len(), expected);
        for (entry, outcome) in &outcomes {
            assert_comment_run_refused(entry, outcome.as_ref().unwrap_err(), 256);
        }
        return;
    }
    assert_child_succeeded(&run_in_child(
        "small_stack_refuses_a_maximal_comment_run_on_every_entry_point",
        MARKER,
        "1",
    ));
}

/// Acceptance criterion 5 of issue #219: a presented `SourceInfo` whose
/// embedded source carries an over-long comment run is refused by
/// rederivation with the same resource, on a 64 KiB stack, not by an abort.
/// The sidecar is genuine (compiled under a raised limit on a large stack),
/// so its `source_bundle_id` matches and rederivation is actually reached.
#[test]
fn small_stack_sidecar_rederivation_refuses_an_over_long_comment_run() {
    const MARKER: &str = "CANDID_CORE_COMMENT_RUN_SIDECAR_CHILD";
    if std::env::var_os(MARKER).is_some() {
        let source = comment_run(257, "service : { f: () -> () };\n");
        let compilation = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                compile_did_with_context(
                    &source,
                    CompileOptions::default(),
                    &RuntimeContext::new(Limits::default().with_max_source_nesting(257)),
                )
                .unwrap()
            })
            .unwrap()
            .join()
            .unwrap();
        let raw: candid_core::RawSourceInfo = serde_json::from_value(
            serde_json::to_value(compilation.source_info().unwrap()).unwrap(),
        )
        .unwrap();
        let contract = compilation.contract().clone();
        let error = std::thread::Builder::new()
            .stack_size(SMALL_STACK_BYTES)
            .spawn(move || {
                candid_core::SourceInfo::try_from_raw_with_context(
                    raw,
                    &contract,
                    &RuntimeContext::default(),
                )
                .map(drop)
            })
            .unwrap()
            .join()
            .expect("small-stack rederivation worker must not abort")
            .unwrap_err();
        assert_eq!(error.violations.len(), 1, "{error:#?}");
        let violation = &error.violations[0];
        assert_eq!(violation.code, "resource_limit_exceeded");
        let resource = violation.resource_limit.as_ref().unwrap();
        assert_eq!(resource.resource, "source_nesting");
        assert_eq!((resource.limit, resource.observed), (256, 257));
        return;
    }
    assert_child_succeeded(&run_in_child(
        "small_stack_sidecar_rederivation_refuses_an_over_long_comment_run",
        MARKER,
        "1",
    ));
}

/// Acceptance criterion 2 of issue #219: a run of exactly the default limit
/// compiles through every entry point, leading or trailing, on Rust's
/// default 2 MiB thread stack, and one more comment is refused with
/// `observed == limit + 1`.
#[test]
fn comment_run_accepts_the_default_limit_and_refuses_one_over() {
    const MARKER: &str = "CANDID_CORE_COMMENT_RUN_BOUNDARY_CHILD";
    if std::env::var_os(MARKER).is_some() {
        let limit = Limits::default().max_source_nesting();
        assert_eq!(limit, 256);
        let outcomes = std::thread::Builder::new()
            .stack_size(COMMENT_RUN_STACK_BYTES)
            .spawn(move || {
                let leading = comment_run(limit, "service : { f: () -> () };\n");
                let trailing = format!("service : {{ f: () -> () }};\n{}", comment_run(limit, ""));
                let over = comment_run(limit + 1, "service : { f: () -> () };\n");
                [
                    every_entry_point(&leading, &Limits::default(), "leading"),
                    every_entry_point(&trailing, &Limits::default(), "trailing"),
                    every_entry_point(&over, &Limits::default(), "over"),
                ]
            })
            .unwrap()
            .join()
            .expect("a run at the default limit must fit a 2 MiB stack");
        let [leading, trailing, over] = outcomes;
        for (entry, outcome) in leading.iter().chain(&trailing) {
            assert!(outcome.is_ok(), "{entry}: {outcome:#?}");
        }
        for (entry, outcome) in &over {
            assert_comment_run_refused(entry, outcome.as_ref().unwrap_err(), limit);
        }
        return;
    }
    assert_child_succeeded(&run_in_child(
        "comment_run_accepts_the_default_limit_and_refuses_one_over",
        MARKER,
        "1",
    ));
}

#[test]
fn comment_run_accepts_a_configured_limit_and_refuses_one_over() {
    let limits = Limits::default().with_max_source_nesting(32);
    let exact = format!("{}service : {{}};", "// line\n/* block */\n".repeat(16));
    for (entry, outcome) in every_entry_point(&exact, &limits, "configured-exact") {
        assert!(outcome.is_ok(), "{entry}: {outcome:#?}");
    }
    let over = format!("// one more\n{exact}");
    for (entry, outcome) in every_entry_point(&over, &limits, "configured-over") {
        assert_comment_run_refused(entry, outcome.as_ref().unwrap_err(), 32);
    }
}

fn compile_under(limit: usize, source: &str) -> Result<(), candid_core::CompileError> {
    compile_with_limits(source, Limits::default().with_max_source_nesting(limit))
}

/// Acceptance criterion 3 of issue #219, through the public API.
#[test]
fn comment_runs_count_comments_between_real_tokens() {
    // Line and block comments count alike, alternating or not.
    let alternating = "// a\n/* b */\n// c\n/* d */\nservice : {};";
    compile_under(4, alternating).unwrap();
    assert_comment_run_refused(
        "alternating",
        &compile_under(3, alternating).unwrap_err(),
        3,
    );

    // A real token between comments resets the run.
    let split = "// a\n// b\n// c\ntype A = nat;\n// d\n// e\n// f\nservice : {};";
    compile_under(3, split).unwrap();
    assert_comment_run_refused("split", &compile_under(2, split).unwrap_err(), 2);

    // Blank lines and spaces do not reset it: upstream skips whitespace
    // without returning, so they cost nothing and end nothing.
    let spaced = "// a\n\n\n// b\n   \n// c\n\n\n\n// d\nservice : {};";
    assert_comment_run_refused("spaced", &compile_under(3, spaced).unwrap_err(), 3);

    // Comment markers inside a string literal count as nothing.
    let quoted = "// a\n// b\nservice : { \"// /* */ //\": () -> () };\n// c\n// d";
    compile_under(2, quoted).unwrap();

    // A nested block comment is one comment.
    let nested = "/* /* /* */ */ */\n// a\nservice : {};";
    compile_under(2, nested).unwrap();
    assert_comment_run_refused("nested", &compile_under(1, nested).unwrap_err(), 1);
}

/// The issue's measured controls: whitespace and nesting that upstream skips
/// without recursing compile under the default limits.
#[test]
fn whitespace_and_nested_block_comments_cost_no_comment_run() {
    let blank_lines = format!("{}service : {{}};", "\n".repeat(1_000_000));
    compile_with_limits(&blank_lines, Limits::default()).unwrap();
    let spaces = format!("{}service : {{}};", " ".repeat(1_000_000));
    compile_with_limits(&spaces, Limits::default()).unwrap();
    let nested = format!(
        "{}{}\nservice : {{}};",
        "/* ".repeat(100_000),
        " */".repeat(100_000)
    );
    compile_with_limits(&nested, Limits::default()).unwrap();
}

/// Acceptance criterion 4 of issue #219: an in-limit header compiles to the
/// identities and documentation it compiled to before the pre-scan existed.
/// The three digests were measured on `main` at c810cdb, before the change.
#[test]
fn a_comment_header_at_the_limit_keeps_its_identities_and_documentation() {
    use sha2::{Digest, Sha256};

    let header: String = (1..=256)
        .map(|line| format!("// header line {line}\n"))
        .collect();
    let source = format!(
        "{header}type Item = record {{ id: nat64 }};\n\
         service : {{\n  // Reads one item.\n  get: (nat64) -> (opt Item) query;\n}};\n"
    );
    let compilation = candid_core::compile_did(&source).unwrap();
    let info = compilation.source_info().unwrap();
    assert_eq!(
        compilation.contract().contract_id(),
        "candid-core:contract:v1:sha256:643b323b455c5c8cab7d8345d0031dec8099b6d12fe417b8e86bd5dda373e42e"
    );
    assert_eq!(
        info.source_bundle_id(),
        "candid-core:source-bundle:v1:sha256:5a1189e025b721b552a19c240390702d9e8285f0bb07adc898339c4dee1d2992"
    );
    assert_eq!(
        hex::encode(Sha256::digest(serde_json::to_vec(info).unwrap())),
        "38792c8fdff3cc82a1f007cd8f89fb80b6d3045f43050cb4720203889f28d2c0"
    );
    // The whole header is the declaration's documentation, line for line.
    let expected: Vec<String> = (1..=256)
        .map(|line| format!("header line {line}"))
        .collect();
    assert_eq!(info.declarations()[0].docs, expected);
    assert_eq!(info.methods()[0].docs, ["Reads one item."]);
}

/// Acceptance criterion 4 of issue #219: inside the limit, a lexical error
/// still reports the parser's own diagnostic. Past it, the comment-run
/// refusal comes first, because upstream would recurse through the run
/// before it reached any error after it.
#[test]
fn a_lexical_error_keeps_the_parser_diagnostic_inside_the_limit() {
    let lexical = |lines: usize| format!("{}type T = nat; @", "// c\n".repeat(lines));
    let error = compile_with_limits(&lexical(256), Limits::default()).unwrap_err();
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.code, "did_parse_error");
    let at = lexical(256).len() as u64 - 1;
    let span = diagnostic.span.as_ref().unwrap();
    assert_eq!((span.start_byte, span.end_byte), (Some(at), Some(at + 1)));
    assert!(
        diagnostic.message.contains("Unknown token @"),
        "{diagnostic:#?}"
    );

    let error = compile_with_limits(&lexical(257), Limits::default()).unwrap_err();
    assert_comment_run_refused("lexical", &error, 256);
}

/// The measured per-comment stack costs `Limits::max_source_nesting`
/// documents, in bytes. Keep the two in step.
const DOCUMENTED_BYTES_PER_COMMENT: usize = if cfg!(debug_assertions) { 3_100 } else { 590 };

/// The stack the cost probes run on. Large next to the fixed cost of a
/// compilation, so the run length dominates what it measures.
const COST_PROBE_STACK_BYTES: usize = 4 * 1024 * 1024;

/// Acceptance criterion 6 of issue #219: the documented stack cost per
/// comment is a measurement, pinned here within a factor of two either way
/// on every platform the suite runs on. The debug figure is checked by every
/// test run; the release figure by Verify's stable job, which runs this test
/// with `--release` on each of its platforms. With the limit lifted, a run the
/// probe stack holds at twice the documented cost compiles, and a run that
/// needs it at half the documented cost overflows it.
#[test]
fn comment_run_stack_cost_matches_the_documented_figure() {
    const MARKER: &str = "CANDID_CORE_COMMENT_COST_CHILD";
    if let Some(lines) = std::env::var_os(MARKER) {
        let lines: usize = lines.to_str().unwrap().parse().unwrap();
        std::thread::Builder::new()
            .stack_size(COST_PROBE_STACK_BYTES)
            .spawn(move || {
                compile_with_limits(
                    &comment_run(lines, "service : {};"),
                    Limits::default().with_max_source_nesting(usize::MAX),
                )
            })
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        return;
    }
    let test = "comment_run_stack_cost_matches_the_documented_figure";
    let fits = COST_PROBE_STACK_BYTES / (2 * DOCUMENTED_BYTES_PER_COMMENT);
    assert_child_succeeded(&run_in_child(test, MARKER, &fits.to_string()));

    let overflows = 2 * COST_PROBE_STACK_BYTES / DOCUMENTED_BYTES_PER_COMMENT;
    let output = run_in_child(test, MARKER, &overflows.to_string());
    assert!(
        !output.status.success(),
        "{overflows} comments fit: the documented cost is stale"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            Some(6),
            "SIGABRT, not a test failure"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("has overflowed its stack"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(windows)]
    assert_eq!(
        output.status.code(),
        Some(0xC000_00FD_u32 as i32),
        "STATUS_STACK_OVERFLOW, not a test failure"
    );
}
