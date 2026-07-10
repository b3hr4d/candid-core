use candid_contract_runtime::{compile_did, Actor, TypeNode};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let compilation = compile_did(
        r#"
        /// A recursive value that remains finite in the Contract graph.
        type List = opt record {
          /// The current item.
          head: nat;
          /// The rest of the list.
          tail: List;
        };

        /// A small service used by the walkthrough.
        service : {
          /// Return the supplied list.
          echo: (input: List) -> (output: List) query;
        };
        "#,
    )?;

    let contract = compilation.contract();
    println!("contract identity: {}", contract.contract_id());
    println!("interface identity: {:?}", contract.interface_id());
    println!("canonical type nodes: {}", contract.types().len());

    let Some(Actor::Service { service }) = contract.actor() else {
        return Err("walkthrough expected a service actor".into());
    };
    let TypeNode::Service { methods } = &contract.types()[*service as usize] else {
        return Err("actor did not reference a service node".into());
    };
    for method in methods {
        let TypeNode::Func {
            args,
            results,
            mode,
        } = &contract.types()[method.function as usize]
        else {
            return Err("service method did not reference a function node".into());
        };
        println!(
            "method {} (wire id {}): {:?}, {} argument(s), {} result(s)",
            method.name,
            method.id,
            mode,
            args.len(),
            results.len()
        );
    }

    let source_info = compilation
        .source_info()
        .ok_or("source provenance was not requested")?;
    println!(
        "provenance: {} source(s), {} documented field occurrence(s), {} named function value(s)",
        source_info.sources().len(),
        source_info
            .field_labels()
            .iter()
            .filter(|field| !field.docs.is_empty())
            .count(),
        source_info.function_arguments().len()
    );

    println!("\nCanonical Contract JSON:\n{}", contract.to_json_pretty()?);
    Ok(())
}
