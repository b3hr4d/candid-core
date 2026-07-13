#![no_main]

use candid_core::SourceId;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(id) = std::str::from_utf8(data) {
        let _ = SourceId::parse(id);
    }
});
