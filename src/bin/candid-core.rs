#[path = "../bounded.rs"]
mod bounded;

use bounded::{read_bounded_utf8, BoundedUtf8Error};
use candid_core::{
    compile_did_file_with_options, CompileOptions, Contract, ContractEnvelope, ContractJsonError,
    ContractValidationError, ContractViolation, Limits, SourceInfo, SourceLabel,
};
use serde_json::json;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: candid-core compile <path> [--no-source-info | --envelope]\n       candid-core validate <path>";

/// The extension carrying field-name triples in an emitted envelope; see
/// [`ContractEnvelope`]'s documentation for the recorded convention.
const FIELD_NAMES_EXTENSION: &str = "org.candid-core.field-names/v1";

enum Invocation {
    Compile {
        path: PathBuf,
        include_source_info: bool,
    },
    CompileEnvelope {
        path: PathBuf,
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
        Some(Invocation::CompileEnvelope { path }) => compile_envelope(&path),
        Some(Invocation::Validate { path }) => validate(&path),
        None => usage(),
    }
}

/// Parses exactly `compile <path> [--no-source-info | --envelope]` or
/// `validate <path>`. The two compile flags are mutually exclusive: the
/// envelope exists to carry the field names that `--no-source-info`
/// suppresses, so asking for both is a contradiction and a usage error.
///
/// Arguments stay OS-native so a non-Unicode path is passed through to the
/// library instead of aborting inside `env::args`.
fn parse_arguments(mut arguments: impl Iterator<Item = OsString>) -> Option<Invocation> {
    let command = arguments.next()?;
    if command == "compile" {
        let path = path_argument(arguments.next()?)?;
        let mut include_source_info = true;
        let mut envelope = false;
        for argument in arguments {
            if argument == "--no-source-info" && include_source_info && !envelope {
                include_source_info = false;
            } else if argument == "--envelope" && !envelope && include_source_info {
                envelope = true;
            } else {
                return None;
            }
        }
        if envelope {
            return Some(Invocation::CompileEnvelope { path });
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

/// `compile <path> --envelope`: the one-document flow. Success prints the
/// envelope document itself — `{"contract": …, "extensions": {…}}`, exactly
/// what `ContractEnvelope` serializes and what the TypeScript runtime's
/// `schemaFromContract` accepts whole — so the output can be saved and
/// consumed without extracting anything. Failures use the same channels as
/// `compile`.
fn compile_envelope(path: &Path) -> ExitCode {
    match compile_did_file_with_options(
        path,
        CompileOptions {
            include_source_info: true,
        },
    ) {
        Ok(compilation) => {
            let (contract, source_info) = compilation.into_parts();
            let source_info = source_info.expect("source info was requested");
            let mut envelope = ContractEnvelope::new(contract);
            match envelope.insert_extension(
                FIELD_NAMES_EXTENSION,
                serde_json::Value::Array(field_name_triples(&source_info)),
                &Limits::default(),
            ) {
                Ok(()) => {
                    write_json(&serde_json::to_value(&envelope).expect("JSON values serialize"))
                }
                Err(error) => write_error(json!({
                    "ok": false,
                    "violations": error.violations,
                })),
            }
        }
        Err(error) => write_error(json!({
            "ok": false,
            "diagnostics": error.diagnostics,
        })),
    }
}

/// The `org.candid-core.field-names/v1` extension value: named field labels
/// as `[container, id, name]` triples, sorted and deduplicated — the table
/// `TsNames::from_source_info` builds and the TypeScript runtime's
/// `FieldNameEntry` describes, derived exactly as the generator's
/// `*.names.json` goldens are. Numeric and positional labels carry no name
/// and are skipped, so those fields keep the `_id_` rendering.
fn field_name_triples(source_info: &SourceInfo) -> Vec<serde_json::Value> {
    let mut triples: Vec<(u32, u32, &str)> = source_info
        .field_labels()
        .iter()
        .filter_map(|provenance| match &provenance.label {
            SourceLabel::Named { name } => {
                Some((provenance.container, provenance.id, name.as_str()))
            }
            _ => None,
        })
        .collect();
    triples.sort();
    triples.dedup();
    triples
        .iter()
        .map(|(container, id, label)| json!([container, id, label]))
        .collect()
}

fn validate(path: &Path) -> ExitCode {
    match fs::File::open(path)
        .map_err(BoundedUtf8Error::Io)
        .and_then(|file| read_bounded_utf8(file, Limits::default().max_input_bytes()))
    {
        Ok(input) => match Contract::from_json(&input) {
            Ok(contract) => write_json(&json!({ "ok": true, "contract": contract })),
            Err(error) => write_error(json_error(error)),
        },
        Err(BoundedUtf8Error::LimitExceeded { observed }) => {
            let limit = Limits::default().max_input_bytes();
            write_error(json_error(ContractJsonError::InvalidContract(
                ContractValidationError {
                    // Exact on every supported target; the library refuses to
                    // compile where usize exceeds 64 bits.
                    violations: vec![ContractViolation::resource_violation(
                        "input_bytes",
                        limit as u64,
                        observed as u64,
                    )],
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
