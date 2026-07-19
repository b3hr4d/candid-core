// Bounded parsing: untrusted bytes decoded under a caller-supplied policy.
//
// `Serialize` on Contract / Compilation / ContractEnvelope and the derived
// `Deserialize` on the raw DTOs (RawContract, RawSourceInfo) are the trusted
// serde integration: they consult no limits and revalidate nothing. Decoding a
// raw DTO carries no allocation bound, so untrusted bytes must arrive through
// the bounded parse entry points used below.

use candid_core::{compile_did, Compilation, Contract, ContractJsonError, Limits, RuntimeContext};
use std::error::Error;

fn resource_limit(error: &ContractJsonError) -> String {
    match error {
        ContractJsonError::InvalidContract(inner) => inner
            .violations
            .iter()
            .find_map(|violation| violation.resource_limit.as_ref())
            .map(|info| {
                format!(
                    "resource={} limit={} observed={}",
                    info.resource, info.limit, info.observed
                )
            })
            .unwrap_or_else(|| "no resource metadata".to_string()),
        ContractJsonError::MalformedJson(message) => format!("malformed: {message}"),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let compilation = compile_did("service : { ping: () -> (nat) query };")?;
    let contract_json = compilation.contract().to_json_pretty()?;
    let compilation_json = compilation.to_json_pretty_with_limits(&Limits::default())?;

    // The byte gate runs before serde_json is invoked, so an oversized document
    // is rejected without being decoded. This bounds peak allocation against
    // the chosen ceiling; it does not reject element-by-element during decode.
    // Decode-time element charging is a named follow-up.
    let context = RuntimeContext::new(Limits {
        max_input_bytes: 64,
        ..Limits::default()
    });
    let rejected =
        Contract::from_slice_with_context(contract_json.as_bytes(), &context).unwrap_err();
    println!("oversized Contract rejected: {}", resource_limit(&rejected));

    // Compilation previously had no *bounded* parse entry point — only the
    // unbounded `impl Deserialize` this change removed. It is now gated the
    // same way as Contract.
    let rejected =
        Compilation::from_slice_with_context(compilation_json.as_bytes(), &context).unwrap_err();
    println!(
        "oversized Compilation rejected: {}",
        resource_limit(&rejected)
    );

    // Raised limits admit both documents. The byte gate, decode, and validation
    // share one budget, so the whole parse is charged against this policy.
    let raised = Limits {
        max_input_bytes: contract_json.len().max(compilation_json.len()) + 1,
        ..Limits::default()
    };
    let context = RuntimeContext::new(raised.clone());
    let contract = Contract::from_slice_with_context(contract_json.as_bytes(), &context)?;
    let parsed = Compilation::from_slice_with_context(compilation_json.as_bytes(), &context)?;
    println!(
        "accepted {} type nodes; compilation {}",
        contract.types().len(),
        parsed.contract().contract_id()
    );

    // Serialization consumes a second budget. Whichever structural limit gated
    // construction, rendering additionally charges the emitted byte length
    // against `max_canonicalization_work`, so a caller who raised only
    // `max_input_bytes` to parse a document may still fail to render it.
    let mut render = raised.clone();
    render.max_canonicalization_work = 16;
    let starved_render = contract.to_json_pretty_with_limits(&render).unwrap_err();
    let metadata = starved_render
        .violations
        .iter()
        .find_map(|violation| violation.resource_limit.as_ref())
        .ok_or("expected resource metadata")?;
    println!(
        "render starved: resource={} limit={} observed={}",
        metadata.resource, metadata.limit, metadata.observed
    );

    render.max_canonicalization_work = Limits::default().max_canonicalization_work;
    let round_tripped = contract.to_json_pretty_with_limits(&render)?;
    println!("round-trip stable: {}", round_tripped == contract_json);
    Ok(())
}
