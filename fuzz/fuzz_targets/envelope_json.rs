#![no_main]

use candid_core::{ContractEnvelope, Limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Envelope parsing is a trust boundary with no other coverage: it gates
    // `max_input_bytes` before decoding, validates the nested Contract and the
    // namespaced extension map on one budget, and rejects malformed extension
    // names. Driving the byte entry point directly also exercises the
    // non-UTF-8 rejection path that a `from_str` target cannot reach.
    let _ = ContractEnvelope::from_slice_with_limits(data, &Limits::default());
});
