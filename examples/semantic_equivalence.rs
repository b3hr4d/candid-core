use candid_contract_runtime::compile_did;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let first = compile_did(
        r#"
        type Payload = record { owner: principal; amount: nat };
        service : {
          z: (Payload) -> () query;
          a: (Payload) -> () query;
        };
        "#,
    )?;
    let second = compile_did(
        r#"
        // Different name, documentation, field order, and method order.
        type Transfer = record { amount: nat; owner: principal };
        service : {
          a: (Transfer) -> () query;
          z: (Transfer) -> () query;
        };
        "#,
    )?;

    println!("first:  {}", first.contract.fingerprint);
    println!("second: {}", second.contract.fingerprint);
    println!(
        "same wire semantics: {}",
        first.contract.fingerprint == second.contract.fingerprint
    );
    println!(
        "same source bundle: {}",
        first.source_info == second.source_info
    );

    assert_eq!(first.contract.fingerprint, second.contract.fingerprint);
    assert_ne!(first.source_info, second.source_info);
    Ok(())
}
