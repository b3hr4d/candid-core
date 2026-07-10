use candid_contract_runtime::{
    compile_did_file_with_options, CompileOptions, Contract, ContractJsonError,
};
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let Some(command) = arguments.next() else {
        return usage();
    };
    let Some(path) = arguments.next() else {
        return usage();
    };
    let path = PathBuf::from(path);

    match command.as_str() {
        "compile" => {
            let include_source_info = !arguments.any(|argument| argument == "--no-source-info");
            match compile_did_file_with_options(
                &path,
                CompileOptions {
                    include_source_info,
                },
            ) {
                Ok(compilation) => {
                    let (contract, source_info) = compilation.into_parts();
                    write_json(&json!({
                        "ok": true,
                        "contract": contract,
                        "source_info": source_info,
                    }))
                }
                Err(error) => write_error(json!({
                    "ok": false,
                    "diagnostics": error.diagnostics,
                })),
            }
        }
        "validate" => match fs::read_to_string(&path) {
            Ok(input) => match Contract::from_json(&input) {
                Ok(contract) => write_json(&json!({ "ok": true, "contract": contract })),
                Err(error) => write_error(json_error(error)),
            },
            Err(error) => write_error(json!({
                "ok": false,
                "diagnostics": [{
                    "code": "contract_file_read_error",
                    "phase": "load",
                    "severity": "error",
                    "message": format!("cannot read {}: {error}", path.display()),
                }],
            })),
        },
        _ => usage(),
    }
}

fn json_error(error: ContractJsonError) -> serde_json::Value {
    match error {
        ContractJsonError::MalformedJson(message) => json!({
            "ok": false,
            "diagnostics": [{
                "code": "malformed_contract_json",
                "phase": "load",
                "severity": "error",
                "message": message,
            }],
        }),
        ContractJsonError::InvalidContract(error) => json!({
            "ok": false,
            "violations": error.violations,
        }),
    }
}

fn write_json(value: &serde_json::Value) -> ExitCode {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("JSON values serialize")
    );
    ExitCode::SUCCESS
}

fn write_error(value: serde_json::Value) -> ExitCode {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("JSON values serialize")
    );
    ExitCode::FAILURE
}

fn usage() -> ExitCode {
    eprintln!("usage: candid-contract <compile|validate> <path> [--no-source-info]");
    ExitCode::from(64)
}
