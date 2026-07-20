mod support;

use candid_core::{
    compile_did, compile_did_with_context, compile_did_with_options, compile_with_resolver,
    validate_host_value, CompileOptions, Contract, HostValue, Limits, RuntimeContext,
    WorkspaceResolver,
};
use candid_parser::candid::{idl_hash, TypeEnv};
use candid_parser::{check_file, check_prog, IDLProg};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::time::Duration;

fn official_check(source: &str) {
    let program = source.parse::<IDLProg>().expect("benchmark DID must parse");
    let mut environment = TypeEnv::new();
    let actor = check_prog(&mut environment, &program).expect("benchmark DID must type check");
    black_box((environment, actor));
}

fn compile_trusted_benchmark(
    source: &str,
    options: CompileOptions,
    context: &RuntimeContext,
) -> candid_core::Compilation {
    compile_did_with_context(source, options, context).expect("benchmark DID must compile")
}

fn compilation_benchmarks(criterion: &mut Criterion) {
    let context = RuntimeContext::new(Limits {
        max_type_depth: 1_024,
        ..Limits::default()
    });
    for case in support::import_free_cases() {
        official_check(&case.source);
        compile_trusted_benchmark(
            &case.source,
            CompileOptions {
                include_source_info: false,
            },
            &context,
        );
        compile_trusted_benchmark(&case.source, CompileOptions::default(), &context);

        let mut group = criterion.benchmark_group(format!("compile/{}", case.name));
        group.throughput(Throughput::Bytes(case.source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("official_parse_check", case.source.len()),
            &case.source,
            |bencher, source| bencher.iter(|| official_check(black_box(source))),
        );
        group.bench_with_input(
            BenchmarkId::new("core_minimal", case.source.len()),
            &case.source,
            |bencher, source| {
                bencher.iter(|| {
                    black_box(compile_trusted_benchmark(
                        black_box(source),
                        CompileOptions {
                            include_source_info: false,
                        },
                        black_box(&context),
                    ))
                })
            },
        );
        group.bench_with_input(
            BenchmarkId::new("core_full", case.source.len()),
            &case.source,
            |bencher, source| {
                bencher.iter(|| {
                    black_box(compile_trusted_benchmark(
                        black_box(source),
                        CompileOptions::default(),
                        black_box(&context),
                    ))
                })
            },
        );
        group.finish();
    }
}

fn imported_file_benchmarks(criterion: &mut Criterion) {
    let entry = support::imported_root();
    let root = entry.parent().expect("import fixture must have a parent");
    let resolver = WorkspaceResolver::new(root).expect("import fixture root must resolve");
    let context = RuntimeContext::default();

    check_file(&entry).expect("official checker must accept imported fixture");
    compile_with_resolver("root.did", &resolver, CompileOptions::default(), &context)
        .expect("core compiler must accept imported fixture");

    let mut group = criterion.benchmark_group("compile/imported_bundle");
    group.throughput(Throughput::Bytes(support::imported_bundle_bytes()));
    group.bench_function("official_check_file", |bencher| {
        bencher.iter(|| black_box(check_file(black_box(&entry)).expect("fixture must type check")))
    });
    group.bench_function("core_compile_with_resolver", |bencher| {
        bencher.iter(|| {
            black_box(
                compile_with_resolver(
                    black_box("root.did"),
                    black_box(&resolver),
                    CompileOptions::default(),
                    black_box(&context),
                )
                .expect("fixture must compile"),
            )
        })
    });
    group.finish();
}

fn artifact_benchmarks(criterion: &mut Criterion) {
    let source = support::ledger_source();
    let compilation = compile_did_with_options(
        source,
        CompileOptions {
            include_source_info: false,
        },
    )
    .expect("ledger fixture must compile");
    let contract = compilation.contract();
    let compact_json = serde_json::to_string(contract).expect("Contract must serialize");

    let mut group = criterion.benchmark_group("artifact/ledger");
    group.throughput(Throughput::Elements(contract.types().len() as u64));
    group.bench_function("validate", |bencher| {
        bencher.iter(|| {
            contract.validate().expect("Contract must remain valid");
            black_box(())
        })
    });
    group.bench_function("canonicalize", |bencher| {
        bencher.iter(|| black_box(contract.canonicalize().expect("Contract must canonicalize")))
    });
    group.bench_function("serialize_compact", |bencher| {
        bencher.iter(|| {
            black_box(serde_json::to_string(black_box(contract)).expect("Contract must serialize"))
        })
    });
    group.bench_function("serialize_validated_pretty", |bencher| {
        bencher.iter(|| black_box(contract.to_json_pretty().expect("Contract must serialize")))
    });
    group.bench_function("parse_validate_canonicalize", |bencher| {
        bencher.iter(|| {
            black_box(
                Contract::from_json(black_box(&compact_json))
                    .expect("serialized Contract must parse"),
            )
        })
    });
    group.finish();
}

/// Contract-directed HostValue validation over adversarially wide type tables.
///
/// These two shapes are the tag/field-lookup paths hardened for the resource
/// audit: a wide `record` exercises the field-ID matching, and a `vec variant`
/// exercises the per-element variant-tag lookup that used to run an uncharged,
/// unbounded scan. Charging every comparison keeps both linear in the work done
/// rather than free, and these benchmarks make that cost observable.
fn host_value_validation_benchmarks(criterion: &mut Criterion) {
    const WIDTH: usize = 256;
    const ELEMENTS: usize = 256;
    // The wide-record value performs O(WIDTH^2) charged comparisons; raise the
    // work ceiling so the benchmark measures the validation, not a rejection.
    let limits = Limits {
        max_canonicalization_work: 100_000_000,
        max_value_elements: 10_000_000,
        ..Limits::default()
    };
    let context = RuntimeContext::new(limits.clone());

    let record_source = {
        let mut source = String::from("type Wide = record {\n");
        for index in 0..WIDTH {
            source.push_str(&format!("  field_{index}: nat;\n"));
        }
        source.push_str("};\nservice : { put: (Wide) -> () };\n");
        source
    };
    let record_compilation = compile_did(&record_source).expect("wide record fixture must compile");
    let record_contract = record_compilation.contract();
    let record_selector = record_contract
        .bind_type(declaration_ref(record_contract, "Wide"))
        .expect("Wide must bind");
    let record_fields: Vec<String> = (0..WIDTH)
        .map(|index| {
            let id = idl_hash(&format!("field_{index}"));
            format!(r#"{{"id":{id},"value":{{"kind":"nat","value":"1"}}}}"#)
        })
        .collect();
    let record_json = format!(
        r#"{{"kind":"record","fields":[{}]}}"#,
        record_fields.join(",")
    );
    let record_value = HostValue::from_json_with_context(&record_json, &context)
        .expect("wide record value must decode");

    let variant_source = {
        let mut source = String::from("type Tag = variant {\n");
        for index in 0..WIDTH {
            source.push_str(&format!("  tag_{index}: null;\n"));
        }
        source.push_str("};\ntype List = vec Tag;\nservice : { push: (List) -> () };\n");
        source
    };
    let variant_compilation =
        compile_did(&variant_source).expect("wide variant fixture must compile");
    let variant_contract = variant_compilation.contract();
    let variant_selector = variant_contract
        .bind_type(declaration_ref(variant_contract, "List"))
        .expect("List must bind");
    // Every element selects the last tag, the worst case for a table scan.
    let last_tag = idl_hash(&format!("tag_{}", WIDTH - 1));
    let variant_elements: Vec<String> = (0..ELEMENTS)
        .map(|_| format!(r#"{{"kind":"variant","id":{last_tag},"value":{{"kind":"null"}}}}"#))
        .collect();
    let variant_json = format!(
        r#"{{"kind":"vec","values":[{}]}}"#,
        variant_elements.join(",")
    );
    let variant_value = HostValue::from_json_with_context(&variant_json, &context)
        .expect("vec-of-variants value must decode");

    let mut group = criterion.benchmark_group("host_value/validate");
    group.throughput(Throughput::Elements(WIDTH as u64));
    group.bench_function(BenchmarkId::new("wide_record", WIDTH), |bencher| {
        bencher.iter(|| {
            validate_host_value(
                black_box(record_contract),
                black_box(&record_selector),
                black_box(&record_value),
                black_box(&limits),
            )
            .expect("wide record value must validate")
        })
    });
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.bench_function(BenchmarkId::new("vec_variant", ELEMENTS), |bencher| {
        bencher.iter(|| {
            validate_host_value(
                black_box(variant_contract),
                black_box(&variant_selector),
                black_box(&variant_value),
                black_box(&limits),
            )
            .expect("vec-of-variants value must validate")
        })
    });
    group.finish();
}

fn declaration_ref(contract: &Contract, name: &str) -> u32 {
    contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == name)
        .unwrap_or_else(|| panic!("missing declaration {name}"))
        .ty
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = compilation_benchmarks, imported_file_benchmarks, artifact_benchmarks,
        host_value_validation_benchmarks
}
criterion_main!(benches);
