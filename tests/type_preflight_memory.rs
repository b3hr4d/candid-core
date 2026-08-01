//! The `type_preflight_work` counter must model what the depth guards
//! actually retain.
//!
//! Issue #125 replaced two exponential tree walks with deduplicated ones that
//! charge a work counter. Deduplication is what makes the walks cheap, but it
//! is also what makes them *retain*: the memo holds one entry per distinct
//! `(node, depth, active recursive names)` state for the whole walk, where the
//! old walks held only a DFS stack. A counter that charges one unit per
//! tracked name while the walk retains an owned copy of that name would model
//! that retention wrongly by a factor of the identifier length — and Candid
//! identifiers are bounded only by `max_source_bytes`, a megabyte. The walks
//! therefore track *borrows* of names their inputs already own.
//!
//! This suite pins that property the only way it is observable: measure it.
//! The assertion is an invariance, not a magic number — two bundles identical
//! in structure and differing only in how long their identifiers are must
//! charge identical work and retain memory that differs only by what the
//! longer source itself costs.

use candid_core::{compile_did_with_context, CompileOptions, Limits, RuntimeContext};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;

static ENABLED: AtomicBool = AtomicBool::new(false);
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

/// Peak simultaneously-live heap bytes during one compilation.
fn peak_live_bytes(source: &str) -> usize {
    ENABLED.store(false, Ordering::SeqCst);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    let result = compile_did_with_context(
        source,
        CompileOptions::default(),
        &RuntimeContext::new(Limits::default()),
    );
    let peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    ENABLED.store(false, Ordering::SeqCst);
    result.expect("the adversarial bundle is valid and within every default limit");
    peak
}

/// The smallest `max_type_preflight_work` at which `source` still compiles,
/// found by bisection — the exact total the two walks charge for it.
fn charged_work(source: &str) -> usize {
    let compiles_within = |limit: usize| {
        compile_did_with_context(
            source,
            CompileOptions::default(),
            &RuntimeContext::new(Limits::default().with_max_type_preflight_work(limit)),
        )
        .is_ok()
    };
    let (mut low, mut high) = (0usize, Limits::default().max_type_preflight_work());
    assert!(compiles_within(high), "must compile at the default limit");
    while low < high {
        let midpoint = low + (high - low) / 2;
        if compiles_within(midpoint) {
            high = midpoint;
        } else {
            low = midpoint + 1;
        }
    }
    low
}

/// A bundle whose depth walks visit many states while every state carries one
/// tracked recursive name padded to `name_bytes`.
///
/// `Recursive` is self-referential, so it is a cycle member and *is* tracked
/// in the per-path active set. Its body reaches one shared record through
/// `TOWERS` `opt` towers of distinct heights, so that record and each of its
/// `FIELDS` fields is a distinct state at every one of those depths — several
/// thousand retained memo entries, every one of them inside `Recursive`'s
/// expansion and therefore holding it in its active set.
///
/// Only the recursive name carries the padding, which is what makes the
/// measurement an isolation rather than a correlation: it is the one name the
/// walks track, it occurs three times in the source, and every other
/// identifier stays short. Retention that scales with *states* times name
/// length therefore separates from the source's own growth by the state
/// count, not by a constant.
fn padded_bundle(name_bytes: usize) -> String {
    const TOWERS: usize = 60;
    const FIELDS: usize = 40;

    let recursive = format!("Recursive_{}", "x".repeat(name_bytes));

    let fields: Vec<String> = (0..FIELDS)
        .map(|index| format!("f{index}: opt nat"))
        .collect();
    let mut source = format!("type Shared = record {{ {} }};\n", fields.join("; "));

    let towers: Vec<String> = (0..TOWERS)
        .map(|height| format!("t{height}: {}Shared", "opt ".repeat(height)))
        .collect();
    source.push_str(&format!(
        "type {recursive} = variant {{ stop: null; go: {recursive}; {} }};\n",
        towers.join("; ")
    ));
    source.push_str(&format!("service : {{ go: ({recursive}) -> () }};"));
    source
}

/// Long identifiers must cost what a longer source costs, and nothing more.
///
/// Both walks track borrows of names their inputs already own, so an active
/// set costs one pointer pair per tracked name however long that name is.
/// Retaining owned copies instead — `BTreeSet<String>` in either walk — makes
/// the memo hold `states * tracked names * name length` bytes that no charge
/// ever sees, which is the defect this asserts against.
///
/// Proven in both directions on this input, release profile: as written, peak
/// live bytes grow 17 580 bytes against a 383 232-byte allowance, and with
/// the owned set reintroduced in `check_type_depth` the same growth is
/// 17 202 080 bytes — 45x the allowance it must stay under, and 979x the
/// growth the borrowing walk shows. The margin either way is three orders of
/// magnitude wider than the platform and profile noise in a peak-heap
/// reading.
#[test]
fn retained_memory_does_not_scale_with_identifier_length() {
    const SHORT: usize = 4;
    const LONG: usize = 2_000;

    let short = padded_bundle(SHORT);
    let long = padded_bundle(LONG);

    // Same structure, so the same states: the charge is a cardinality, and
    // cardinalities do not know how long a name is.
    let short_work = charged_work(&short);
    assert_eq!(
        short_work,
        charged_work(&long),
        "identifier length must not change what the walks charge"
    );
    assert!(
        short_work > 10_000,
        "the shape must actually exercise many states, charged {short_work}"
    );

    let short_peak = peak_live_bytes(&short);
    let long_peak = peak_live_bytes(&long);

    // What a longer name legitimately costs: the source text, the parsed AST,
    // the checked environment, and the lowered Contract each hold it a
    // bounded number of times, so the honest cost is `occurrences * name
    // length` — a constant multiple of how much the source itself grew. A 64x
    // allowance on that growth covers all of it many times over. What it
    // refuses is `states * name length`, which for this bundle is several
    // thousand times the same quantity.
    let source_growth = long.len() - short.len();
    let allowance = source_growth.saturating_mul(64);
    let observed_growth = long_peak.saturating_sub(short_peak);
    assert!(
        observed_growth <= allowance,
        "peak live bytes grew {observed_growth} when identifiers grew {source_growth} bytes \
         (allowance {allowance}); retained memory is scaling with identifier length, so the \
         depth walks are holding owned names the type_preflight_work counter never charges. \
         short={short_peak} long={long_peak}"
    );
}
