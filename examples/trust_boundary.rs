use candid_core::{compile_did, Contract};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let compilation = compile_did("service : { ping: () -> (nat) query };")?;
    let canonical_json = compilation.contract().to_json_pretty()?;
    let accepted = Contract::from_json(&canonical_json)?;
    println!("validated {} type nodes", accepted.types().len());

    let mut injected: serde_json::Value = serde_json::from_str(&canonical_json)?;
    injected["widget"] = serde_json::json!("date-picker");
    let rejected = Contract::from_json(&serde_json::to_string(&injected)?).unwrap_err();
    println!("unknown core metadata rejected: {rejected}");

    let mut tampered: serde_json::Value = serde_json::from_str(&canonical_json)?;
    tampered["identities"]["contract"] = serde_json::json!(
        "candid-core:contract:v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    let rejected = Contract::from_json(&serde_json::to_string(&tampered)?).unwrap_err();
    println!("tampered semantic identity rejected: {rejected}");
    Ok(())
}
