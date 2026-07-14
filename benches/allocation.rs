#[allow(dead_code)]
mod support;

use candid_core::{compile_did_with_options, CompileOptions, Contract};
use candid_parser::candid::TypeEnv;
use candid_parser::{check_prog, IDLProg};
use serde::Serialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ENABLED.load(Ordering::Relaxed) {
            record_deallocation(layout.size());
        }
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            record_deallocation(layout.size());
            record_allocation(new_size);
        }
        new_pointer
    }
}

fn record_allocation(size: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(size, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    let mut peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    while live > peak {
        match PEAK_LIVE_BYTES.compare_exchange_weak(
            peak,
            live,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

fn record_deallocation(size: usize) {
    let _ = LIVE_BYTES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
        Some(live.saturating_sub(size))
    });
}

fn reset_counters() {
    ENABLED.store(false, Ordering::SeqCst);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
}

#[derive(Serialize)]
struct AllocationMeasurement {
    case: &'static str,
    operation: &'static str,
    input_bytes: usize,
    allocations: usize,
    allocated_bytes: usize,
    peak_live_bytes: usize,
}

fn measure<T>(
    case: &'static str,
    operation: &'static str,
    input_bytes: usize,
    operation_fn: impl FnOnce() -> T,
) -> AllocationMeasurement {
    reset_counters();
    let output = operation_fn();
    let measurement = AllocationMeasurement {
        case,
        operation,
        input_bytes,
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        peak_live_bytes: PEAK_LIVE_BYTES.load(Ordering::Relaxed),
    };
    ENABLED.store(false, Ordering::SeqCst);
    drop(output);
    measurement
}

fn official_check(source: &str) {
    let program = source.parse::<IDLProg>().expect("benchmark DID must parse");
    let mut environment = TypeEnv::new();
    let actor = check_prog(&mut environment, &program).expect("benchmark DID must type check");
    std::hint::black_box((environment, actor));
}

fn main() {
    let source = support::ledger_source();
    if std::env::args().any(|argument| argument == "--test") {
        official_check(source);
        compile_did_with_options(source, CompileOptions::default())
            .expect("allocation probe fixture must compile");
        return;
    }

    let minimal_options = CompileOptions {
        include_source_info: false,
    };
    official_check(source);
    compile_did_with_options(source, minimal_options)
        .expect("allocation probe fixture must compile without source info");
    compile_did_with_options(source, CompileOptions::default())
        .expect("allocation probe fixture must compile with source info");

    let contract = compile_did_with_options(source, minimal_options)
        .expect("allocation probe fixture must compile")
        .into_parts()
        .0;
    let compact_json = serde_json::to_string(&contract).expect("Contract must serialize");
    contract.validate().expect("Contract must validate");
    contract.canonicalize().expect("Contract must canonicalize");
    contract
        .to_json_pretty()
        .expect("Contract must serialize to canonical JSON");
    Contract::from_json(&compact_json).expect("Contract JSON must parse");

    let measurements = vec![
        measure("ledger", "official_parse_check", source.len(), || {
            official_check(source)
        }),
        measure("ledger", "core_minimal", source.len(), || {
            compile_did_with_options(source, minimal_options)
                .expect("allocation probe fixture must compile")
        }),
        measure("ledger", "core_full", source.len(), || {
            compile_did_with_options(source, CompileOptions::default())
                .expect("allocation probe fixture must compile")
        }),
        measure("ledger", "validate", compact_json.len(), || {
            contract.validate().expect("Contract must validate")
        }),
        measure("ledger", "canonicalize", compact_json.len(), || {
            contract.canonicalize().expect("Contract must canonicalize")
        }),
        measure("ledger", "serialize_compact", compact_json.len(), || {
            serde_json::to_string(&contract).expect("Contract must serialize")
        }),
        measure(
            "ledger",
            "serialize_validated_pretty",
            compact_json.len(),
            || {
                contract
                    .to_json_pretty()
                    .expect("Contract must serialize to canonical JSON")
            },
        ),
        measure(
            "ledger",
            "parse_validate_canonicalize",
            compact_json.len(),
            || Contract::from_json(&compact_json).expect("Contract JSON must parse"),
        ),
    ];

    println!(
        "{}",
        serde_json::to_string_pretty(&measurements)
            .expect("allocation measurements must serialize")
    );
}
