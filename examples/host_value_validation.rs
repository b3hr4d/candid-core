use candid_core::{compile_did, validate_host_value, HostFieldValue, HostValue, Limits};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let compilation = compile_did(
        r#"
        type Measurement = record { count: nat; reading: float64 };
        service : { submit: (Measurement) -> () };
        "#,
    )?;
    let contract = compilation.contract();
    let measurement = contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == "Measurement")
        .ok_or("missing Measurement declaration")?;
    let selector = contract.bind_type(measurement.ty)?;
    let value = HostValue::record(
        vec![
            HostFieldValue::new(
                candid_parser::candid::idl_hash("count"),
                HostValue::nat("340282366920938463463374607431768211456")?,
            ),
            HostFieldValue::new(
                candid_parser::candid::idl_hash("reading"),
                // A NaN payload preserved exactly as IEEE-754 bits.
                HostValue::float64("7ff8000000000001")?,
            ),
        ],
        &Limits::default(),
    )?;

    validate_host_value(contract, &selector, &value, &Limits::default())?;
    println!(
        "selector: {} / type {}",
        selector.contract_id, selector.type_ref
    );
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
