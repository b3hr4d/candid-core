#![no_main]

use candid_core::Compilation;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<Compilation>(data);
});
