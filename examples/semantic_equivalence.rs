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

    println!("first:  {}", first.contract().interface_id().unwrap());
    println!("second: {}", second.contract().interface_id().unwrap());
    println!(
        "same wire semantics: {}",
        first.contract().interface_id() == second.contract().interface_id()
    );
    println!(
        "same source bundle: {}",
        first.source_info().map(|source| source.source_bundle_id())
            == second.source_info().map(|source| source.source_bundle_id())
    );

    assert_eq!(
        first.contract().interface_id(),
        second.contract().interface_id()
    );
    assert_ne!(
        first.source_info().unwrap().source_bundle_id(),
        second.source_info().unwrap().source_bundle_id()
    );
    Ok(())
}
