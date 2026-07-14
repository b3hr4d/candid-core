mod support;

use candid_core::{
    compile_did_with_options, compile_with_resolver, CompileOptions, Contract, RuntimeContext,
    WorkspaceResolver,
};
use candid_parser::candid::TypeEnv;
use candid_parser::{check_file, check_prog, IDLProg};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::time::Duration;

fn official_check(source: &str) {
    let program = source.parse::<IDLProg>().expect("benchmark DID must parse");
    let mut environment = TypeEnv::new();
    let actor = check_prog(&mut environment, &program).expect("benchmark DID must type check");
    black_box((environment, actor));
}

fn compilation_benchmarks(criterion: &mut Criterion) {
    for case in support::import_free_cases() {
        official_check(&case.source);
        compile_did_with_options(
            &case.source,
            CompileOptions {
                include_source_info: false,
            },
        )
        .expect("benchmark DID must compile without source info");
        compile_did_with_options(&case.source, CompileOptions::default())
            .expect("benchmark DID must compile with source info");

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
                    black_box(
                        compile_did_with_options(
                            black_box(source),
                            CompileOptions {
                                include_source_info: false,
                            },
                        )
                        .expect("benchmark DID must compile"),
                    )
                })
            },
        );
        group.bench_with_input(
            BenchmarkId::new("core_full", case.source.len()),
            &case.source,
            |bencher, source| {
                bencher.iter(|| {
                    black_box(
                        compile_did_with_options(black_box(source), CompileOptions::default())
                            .expect("benchmark DID must compile"),
                    )
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

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = compilation_benchmarks, imported_file_benchmarks, artifact_benchmarks
}
criterion_main!(benches);
