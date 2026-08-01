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
//! therefore intern names to dense indices and share each path's set between
//! the children that inherit it unchanged.
//!
//! This suite pins that property the only way it is observable: measure it.
//! The assertion is an invariance, not a magic number — two bundles identical
//! in structure and differing only in how long their identifiers are must
//! charge identical work and retain memory that differs only by what the
//! longer source itself costs.

use candid_core::{compile_did_with_context, CompileOptions, Limits, RuntimeContext};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

/// The counters below are process globals, so only one test may measure at a
/// time; the harness runs test functions in parallel by default. Held for a
/// whole test body rather than around each measurement, because an
/// unsynchronized allocation from another test would perturb the peak just as
/// badly between two measurements as during one.
static MEASURING: Mutex<()> = Mutex::new(());

fn measuring() -> MutexGuard<'static, ()> {
    // A panicking measurement poisons the lock; the counters are reset at the
    // start of every measurement, so the next test can still measure.
    MEASURING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
/// Both walks intern names to dense indices, so an active set costs one
/// machine word per tracked name however long that name is. Retaining owned
/// copies instead — `BTreeSet<String>` in either walk — makes the memo hold
/// `states * tracked names * name length` bytes that no charge ever sees,
/// which is the defect this asserts against.
///
/// Proven in both directions on this input, release profile: as written, peak
/// live bytes grow 17 580 bytes against a 383 232-byte allowance, and with
/// the owned set reintroduced in `check_type_depth` the same growth is
/// 17 202 080 bytes — 45x the allowance it must stay under, and 979x the
/// growth the interning walk shows. The margin either way is three orders of
/// magnitude wider than the platform and profile noise in a peak-heap
/// reading.
#[test]
fn retained_memory_does_not_scale_with_identifier_length() {
    let _measuring = measuring();
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

/// A bundle with `towers` `opt` towers of distinct heights over one shared
/// record of `fields` fields, and short identifiers throughout. Scaling both
/// scales the state count, which is what the memo retains.
fn state_heavy_bundle(towers: usize, fields: usize) -> String {
    let fields: Vec<String> = (0..fields)
        .map(|index| format!("f{index}: opt nat"))
        .collect();
    let mut source = format!("type Shared = record {{ {} }};\n", fields.join("; "));
    let towers: Vec<String> = (0..towers)
        .map(|height| format!("t{height}: {}Shared", "opt ".repeat(height)))
        .collect();
    source.push_str(&format!(
        "type Recursive = variant {{ stop: null; go: Recursive; {} }};\n",
        towers.join("; ")
    ));
    source.push_str("service : { go: (Recursive) -> () };");
    source
}

/// `max_type_preflight_work` bounds memory, not only time, so the bytes it
/// authorizes per unit are part of its contract.
///
/// Deduplication is what makes the depth walks cheap and also what makes them
/// retain: every distinct state stays in a memo for the whole walk. A host
/// sizing the limit for a constrained heap — the guidance
/// `Limits::max_type_preflight_work` gives for `wasm32` — needs that exchange
/// rate to be a measured, stable number rather than an implementation detail
/// free to drift. Measured marginal cost across these four sizes, release
/// profile: 19.4, 19.3, and 19.4 bytes per unit. The measured quantity is the
/// size each allocation *requests*, not the block an allocator hands back, so
/// it is a property of the data structures rather than of the platform; the
/// band below is still wide enough to absorb profile differences while
/// catching any change that materially moves what a state retains.
#[test]
fn retained_bytes_per_charged_unit_stay_within_the_documented_band() {
    let _measuring = measuring();
    let mut previous: Option<(usize, usize)> = None;
    let mut observed = Vec::new();
    for (towers, fields) in [(60, 40), (120, 60), (180, 80), (240, 100)] {
        let source = state_heavy_bundle(towers, fields);
        let work = charged_work(&source);
        let peak = peak_live_bytes(&source);
        if let Some((previous_work, previous_peak)) = previous {
            let bytes_per_unit =
                peak.saturating_sub(previous_peak) as f64 / (work - previous_work) as f64;
            observed.push(bytes_per_unit);
        }
        previous = Some((work, peak));
    }

    for bytes_per_unit in &observed {
        assert!(
            (8.0..=40.0).contains(bytes_per_unit),
            "marginal retained bytes per charged unit is {bytes_per_unit:.1}, outside the \
             8-40 band Limits::max_type_preflight_work documents as ~19. Its guidance for \
             sizing this limit on a constrained heap is derived from that number, so a real \
             move here has to be re-measured and re-documented, not just re-pinned. \
             Observed across sizes: {observed:?}"
        );
    }
}

/// A long reference cycle, ending in a wide record that closes it.
///
/// Walking `A1` enters each `A` in turn, and because the final record
/// references `A1` again every one of them is a cycle member and so is
/// tracked. By the time the walk reaches that record the active set holds
/// `cycle` names, and the record then pushes one child state per field, each
/// inheriting that set. Both factors are attacker-controlled.
fn cycle_then_wide_record(cycle: usize, fields: usize) -> String {
    let mut source = String::new();
    for index in 1..cycle {
        source.push_str(&format!("type A{index} = A{};\n", index + 1));
    }
    let fields: Vec<String> = (0..fields).map(|index| format!("f{index}: nat")).collect();
    source.push_str(&format!(
        "type A{cycle} = record {{ back: A1; {} }};\n",
        fields.join("; ")
    ));
    source.push_str("service : { go: (A1) -> () };");
    source
}

/// A state must not commit unbounded memory *before* its own charge lands.
///
/// Charging happens when a state is popped, but children are created when
/// their parent is expanded. If each child copied the parent's active set, one
/// expansion of a wide record would allocate `fields * set` entries against a
/// parent charge of `1 + set` — memory the counter has not authorized and, at
/// a nearly exhausted budget, never will. Sharing the set makes a child cost a
/// reference-count bump instead.
///
/// The assertion is the documented exchange rate applied at the moment of
/// refusal: whatever a compilation allocates before failing closed must still
/// be in proportion to what it managed to charge. Measured on this input,
/// release profile: 22 bytes per charged unit as written, against 559 with the
/// set copied into every child — the copying walk overshoots the band that
/// `Limits::max_type_preflight_work` documents by an order of magnitude, which
/// is exactly the promise it breaks.
#[test]
fn memory_committed_before_a_charge_lands_stays_in_proportion() {
    let _measuring = measuring();
    const BUDGET: usize = 50_000;

    let source = cycle_then_wide_record(200, 4_000);
    ENABLED.store(false, Ordering::SeqCst);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    let result = compile_did_with_context(
        &source,
        CompileOptions::default(),
        &RuntimeContext::new(Limits::default().with_max_type_preflight_work(BUDGET)),
    );
    let peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    ENABLED.store(false, Ordering::SeqCst);

    // The point of the input: it must actually exhaust the budget, so the
    // measurement is of memory committed on the way to a refusal.
    let error = result.expect_err("the tight budget must refuse this bundle");
    let resource = error.diagnostics[0]
        .resource_limit
        .as_ref()
        .expect("refusal must carry the resource triple");
    assert_eq!(resource.resource, "type_preflight_work");
    assert_eq!(resource.limit, BUDGET as u64);

    let bytes_per_charged_unit = peak as f64 / BUDGET as f64;
    assert!(
        bytes_per_charged_unit <= 60.0,
        "committed {peak} bytes against {BUDGET} charged units \
         ({bytes_per_charged_unit:.0} bytes/unit), above the 60 ceiling \
         Limits::max_type_preflight_work documents. A state is allocating ahead of its \
         charge — most likely each child of a wide record is copying the path's active \
         set instead of sharing it."
    );
}
