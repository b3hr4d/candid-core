#![no_main]

use candid_core::{ContractDraft, RawContract};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The seed corpus is RawContract-shaped, so decode that wire DTO and then
    // rebuild through the producer API. A draft carries no format markers,
    // identities, or producer fields, so `build` stamps the current constants
    // and `ProducerInfo::current()` and canonicalizes every structurally valid
    // graph — covering at least the canonicalization inputs the removed
    // `Contract::build_raw` reached, and then some (`build_raw` rejected
    // mismatched format markers or an empty producer before canonicalizing).
    // The complementary `try_from_raw` trust-boundary path, which rejects a
    // malformed presented identity before the canonicalizer, is exercised by
    // the contract_json target instead.
    if let Ok(raw) = serde_json::from_slice::<RawContract>(data) {
        let _ = ContractDraft::new(raw.types, raw.declarations, raw.actor).build();
    }
});
