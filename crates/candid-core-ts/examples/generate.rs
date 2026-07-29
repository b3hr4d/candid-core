//! Dev-only driver: compile one self-contained `.did` file and print the
//! generated schema module to stdout.
//!
//! This is developer tooling, not crate surface: examples build against
//! dev-dependencies, so the library's base-surface boundary claim is
//! untouched, and `required-features = ["compiler"]` keeps the featureless
//! build from ever seeing it. The candid-start prototype's regeneration
//! script (`prototypes/candid-start/scripts/generate.ts`) is the consumer.
//!
//! Reading the file with `std::fs` here is deliberate and confined: the
//! *library* takes sources through resolvers precisely so that ambient
//! filesystem access stays in binaries and examples like this one.

use std::process::ExitCode;

use candid_core_ts::{generate_module, TsNames, TsOptions};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("usage: generate <path-to-did-file>");
        return ExitCode::from(64);
    };

    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };

    let compilation = match candid_core::compile_did(&source) {
        Ok(compilation) => compilation,
        Err(error) => {
            eprintln!("error: compilation failed: {error:?}");
            return ExitCode::FAILURE;
        }
    };

    let names = match compilation.source_info() {
        Some(source_info) => TsNames::from_source_info(source_info),
        None => TsNames::new(),
    };

    match generate_module(compilation.contract(), &names, &TsOptions::default()) {
        Ok(module) => {
            print!("{module}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: generation failed: {error}");
            ExitCode::FAILURE
        }
    }
}
