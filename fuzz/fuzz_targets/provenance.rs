#![no_main]

use candid_core::{Compilation, Limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Exercises the bounded parse path that replaced `impl Deserialize for
    // Compilation`: the `max_input_bytes` gate, raw sidecar decoding, and
    // provenance remapping and validation, all charged to one budget.
    let _ = Compilation::from_slice_with_limits(data, &Limits::default());
});
