#![no_main]

use candid_core::{Contract, Limits, RawContract};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = serde_json::from_slice::<RawContract>(data) {
        let _ = Contract::build_raw(raw, &Limits::default());
    }
});
