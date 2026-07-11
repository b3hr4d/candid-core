use candid_core::{compile_with_resolver, CompileOptions, MemoryResolver, RuntimeContext};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut resolver = MemoryResolver::new();
    resolver.insert(
        "api/root.did",
        r#"import "types.did"; service : { read: () -> (Item) query };"#,
    )?;
    resolver.insert(
        "api/types.did",
        "type Item = record { id: nat64; label: text };",
    )?;

    let compilation = compile_with_resolver(
        "api/root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )?;
    let source_info = compilation
        .source_info()
        .ok_or("source provenance was not requested")?;
    println!("contract: {}", compilation.contract().contract_id());
    println!("source bundle: {}", source_info.source_bundle_id());
    for source in source_info.sources() {
        println!("- {} ({} bytes)", source.name, source.source.len());
    }
    Ok(())
}
