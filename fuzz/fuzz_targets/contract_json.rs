#![no_main]

use candid_core::{Contract, Limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(json) = std::str::from_utf8(data) {
        let _ = Contract::from_json_with_limits(json, &Limits::default());
    }
});
