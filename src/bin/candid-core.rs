#[path = "../bounded.rs"]
mod bounded;

use bounded::{read_bounded_utf8, BoundedUtf8Error};
use candid_core::{
    compile_did_file_with_options, CompileOptions, Contract, ContractJsonError,
    ContractValidationError, ContractViolation, Limits, ResourceLimitInfo,
};
use serde_json::json;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str =
    "usage: candid-core compile <path> [--no-source-info]\n       candid-core validate <path>";

enum Invocation {
    Compile {
        path: PathBuf,
        include_source_info: bool,
    },
    Validate {
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    match parse_arguments(env::args_os().skip(1)) {
        Some(Invocation::Compile {
            path,
            include_source_info,
        }) => compile(&path, include_source_info),
        Some(Invocation::Validate { path }) => validate(&path),
        None => usage(),
    }
}

/// Parses exactly `compile <path> [--no-source-info]` or `validate <path>`.
///
/// Arguments stay OS-native so a non-Unicode path is passed through to the
/// library instead of aborting inside `env::args`.
fn parse_arguments(mut arguments: impl Iterator<Item = OsString>) -> Option<Invocation> {
    let command = arguments.next()?;
    if command == "compile" {
        let path = path_argument(arguments.next()?)?;
        let mut include_source_info = true;
        for argument in arguments {
            if !include_source_info || argument != "--no-source-info" {
                return None;
            }
            include_source_info = false;
        }
        return Some(Invocation::Compile {
            path,
            include_source_info,
        });
    }
    if command == "validate" {
        let path = path_argument(arguments.next()?)?;
        if arguments.next().is_some() {
            return None;
        }
        return Some(Invocation::Validate { path });
    }
    None
}

/// Accepts the mandatory `<path>` token. An option-looking token in this
/// position is a misplaced or unknown option, never a path; a dash-leading
/// relative file is spelled with a `./` prefix instead.
fn path_argument(argument: OsString) -> Option<PathBuf> {
    if argument.as_encoded_bytes().starts_with(b"-") {
        return None;
    }
    Some(PathBuf::from(argument))
}

fn compile(path: &Path, include_source_info: bool) -> ExitCode {
    match compile_did_file_with_options(
        path,
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

fn validate(path: &Path) -> ExitCode {
    match fs::File::open(path)
        .map_err(BoundedUtf8Error::Io)
        .and_then(|file| read_bounded_utf8(file, Limits::default().max_input_bytes))
    {
        Ok(input) => match Contract::from_json(&input) {
            Ok(contract) => write_json(&json!({ "ok": true, "contract": contract })),
            Err(error) => write_error(json_error(error)),
        },
        Err(BoundedUtf8Error::LimitExceeded { observed }) => {
            let limit = Limits::default().max_input_bytes;
            write_error(json_error(ContractJsonError::InvalidContract(
                ContractValidationError {
                    violations: vec![ContractViolation {
                        code: "resource_limit_exceeded".to_string(),
                        path: "$".to_string(),
                        message: format!(
                            "resource input_bytes exceeded limit {limit}; observed {}",
                            observed
                        ),
                        resource_limit: Some(ResourceLimitInfo {
                            resource: "input_bytes".to_string(),
                            limit,
                            observed,
                        }),
                    }],
                },
            )))
        }
        Err(BoundedUtf8Error::Io(error)) => write_error(json!({
            "ok": false,
            "diagnostics": [{
                "code": "contract_file_read_error",
                "phase": "load",
                "severity": "error",
                "message": format!("cannot read {}: {error}", path.display()),
            }],
        })),
        Err(BoundedUtf8Error::InvalidUtf8(error)) => write_error(json!({
            "ok": false,
            "diagnostics": [{
                "code": "contract_file_read_error",
                "phase": "load",
                "severity": "error",
                "message": format!("cannot read {}: {error}", path.display()),
            }],
        })),
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
    eprintln!("{USAGE}");
    ExitCode::from(64)
}
