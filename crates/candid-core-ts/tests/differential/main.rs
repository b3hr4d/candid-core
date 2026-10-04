//! Issue #196: the differential fuzz of the TypeScript runtime against the
//! Rust reference, Rust half.
//!
//! This driver generates seeded random cases for three targets and records
//! the reference's verdict for each:
//!
//! - **decode** — `(wire types, bytes, expected types)` triples: a random
//!   value at the wire types, encoded by the `candid` crate, optionally
//!   mutated, decoded by `IDLArgs::from_bytes_with_types` (see `wire.rs`);
//!   the TypeScript runner feeds the bytes to `decodeArgs` with schemas
//!   `schemaFromContract` builds from the same declarations;
//! - **validate** — a JavaScript domain value converted to a HostValue and
//!   judged by `validate_host_value` (see `host.rs`); the runner calls
//!   `validate` with the same value;
//! - **contract** — a compiled Contract document edited by JSON operations
//!   and judged by `Contract::from_json` (see `contract.rs`); the runner calls
//!   `schemaFromContract` on the same edited document.
//!
//! Each random environment is a `.did` source (see `types.rs`); the reference
//! types come from `candid_parser` over that source, and the Contract the
//! TypeScript side loads comes from candid-core's compiler over the same text.
//!
//! # The committed corpus (CI)
//!
//! `differential_corpus_matches_reference` regenerates the corpus from the
//! committed seeds (`CORPUS_SEEDS`, a fixed count, no time budget — #39) plus
//! the minimized regression vectors in
//! `tests/fixtures/differential/regressions.json`, and compares it with
//! `tests/goldens/differential/corpus.jsonl` byte for byte. The TypeScript
//! suite (`ts/tests/differential.test.ts`) replays that file and fails on any
//! divergence that is not in the reviewed expected-divergence list. Regenerate
//! deliberately with `UPDATE_GOLDENS=1 cargo test -p candid-core-ts --features
//! compiler --test differential`, then review the diff.
//!
//! # Campaign mode
//!
//! `differential_campaign` (ignored) writes a corpus for other seeds to a file
//! of your choosing; `ts/tests/differential/campaign.ts` replays it. See
//! `docs/verification.md` ("Differential fuzz").
#![cfg(feature = "compiler")]

mod contract;
mod host;
mod rng;
mod types;
mod wire;

use std::path::PathBuf;

use candid::types::Type;
use candid::TypeEnv;
use candid_parser::{check_prog, IDLProg};
use serde_json::{json, Value};

use rng::Rng;

/// The committed corpus: environments `start..start + envs`, each with a
/// fixed number of cases per target.
struct Seeds {
    start: u64,
    envs: u64,
    decode: usize,
    validate: usize,
    contract: usize,
}

const CORPUS_SEEDS: Seeds = Seeds {
    start: 1,
    envs: 24,
    decode: 40,
    validate: 12,
    contract: 8,
};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One generated environment: its source, the reference type environment,
/// and the envelope the TypeScript loader reads.
struct Built {
    env: types::Env,
    types: TypeEnv,
    envelope: Value,
    contract: candid_core::Contract,
}

fn reference_env(source: &str) -> Option<TypeEnv> {
    let prog: IDLProg = source.parse().ok()?;
    let mut env = TypeEnv::new();
    check_prog(&mut env, &prog).ok()?;
    Some(env)
}

/// The Contract of `source` as the one-document envelope the TypeScript
/// loader reads, normalized as `wire_vectors.rs` normalizes the coercion
/// golden's (a fixed producer block, a canonical reparse, field names in the
/// `org.candid-core.field-names/v1` extension).
fn envelope_of(source: &str) -> Option<(Value, candid_core::Contract)> {
    let compilation = candid_core::compile_did(source).ok()?;
    let mut document = serde_json::to_value(compilation.contract()).ok()?;
    document["producer"] = json!({
        "name": "candid-core",
        "version": "0.0.0-golden",
        "candid_version": "0.0.0-golden",
        "candid_parser_version": "0.0.0-golden",
    });
    let contract = candid_core::Contract::from_json(&document.to_string()).ok()?;
    let mut names = std::collections::BTreeMap::new();
    for provenance in compilation.source_info()?.field_labels() {
        if let candid_core::SourceLabel::Named { name } = &provenance.label {
            names.insert((provenance.container, provenance.id), name.clone());
        }
    }
    let triples: Vec<Value> = names
        .into_iter()
        .map(|((container, id), name)| json!([container, id, name]))
        .collect();
    let mut envelope = candid_core::ContractEnvelope::new(contract.clone());
    envelope
        .insert_extension(
            "org.candid-core.field-names/v1",
            Value::Array(triples),
            &candid_core::Limits::default(),
        )
        .ok()?;
    Some((serde_json::to_value(&envelope).ok()?, contract))
}

fn build_from_source(env: types::Env) -> Option<Built> {
    let types = wire::guarded(|| reference_env(&env.source)).ok()??;
    let (envelope, contract) = wire::guarded(|| envelope_of(&env.source)).ok()??;
    Some(Built {
        env,
        types,
        envelope,
        contract,
    })
}

/// A random environment both sides accept; rejected drafts are redrawn from
/// the same stream, so the result is still a function of the seed.
fn build_random(rng: &mut Rng) -> Built {
    loop {
        if let Some(built) = build_from_source(types::random_env(rng)) {
            return built;
        }
    }
}

fn find(types: &TypeEnv, name: &str) -> Type {
    types
        .find_type(name)
        .unwrap_or_else(|_| panic!("declaration {name} must exist"))
        .clone()
}

fn names(indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .map(|index| types::decl_name(*index))
        .collect()
}

fn family_of(env: &types::Env, decl: usize) -> &[usize] {
    env.families
        .iter()
        .find(|family| family.contains(&decl))
        .map_or(&[], Vec::as_slice)
}

fn raw_bytes(rng: &mut Rng) -> Vec<u8> {
    let mut bytes = b"DIDL".to_vec();
    for _ in 0..rng.below(12) {
        bytes.push(if rng.chance(1, 2) {
            rng.byte()
        } else {
            *rng.pick(&[
                0x00, 0x01, 0x02, 0x6c, 0x6b, 0x6d, 0x6e, 0x7d, 0x7c, 0x71, 0x7f,
            ])
        });
    }
    bytes
}

fn decode_case(built: &Built, rng: &mut Rng, id: String, env_id: &str) -> Value {
    let decls = built.env.decls;
    let mut wire: Option<Vec<usize>> = None;
    let mut bytes = Vec::new();
    let mut expected: Vec<usize> = Vec::new();
    if !rng.chance(1, 25) {
        for _attempt in 0..4 {
            let count = [0, 1, 1, 1, 1, 1, 1, 2, 2, 3][rng.below(10)];
            let wire_decls: Vec<usize> = (0..count).map(|_| rng.below(decls)).collect();
            let mut values = Vec::new();
            let wire_types: Vec<Type> = names(&wire_decls)
                .iter()
                .map(|name| find(&built.types, name))
                .collect();
            for ty in &wire_types {
                match wire::random_value(&built.types, ty, rng, 0) {
                    Some(value) => values.push(value),
                    None => break,
                }
            }
            if values.len() != wire_types.len() {
                continue;
            }
            let Some(encoded) = wire::encode(&built.types, &wire_types, values) else {
                continue;
            };
            bytes = encoded;
            expected = wire_decls
                .iter()
                .map(|&decl| match rng.below(20) {
                    0..=8 => decl,
                    9..=15 => *rng.pick(family_of(&built.env, decl)),
                    _ => rng.below(decls),
                })
                .collect();
            match rng.below(10) {
                0 if !expected.is_empty() => {
                    expected.pop();
                }
                1 => expected.push(rng.below(decls)),
                _ => {}
            }
            wire = Some(wire_decls);
            break;
        }
    }
    let mut mutation = Vec::new();
    if wire.is_none() {
        bytes = raw_bytes(rng);
        expected = (0..rng.below(3)).map(|_| rng.below(decls)).collect();
        mutation.push("raw");
    } else if !rng.chance(9, 20) {
        for _ in 0..1 + rng.below(2) {
            mutation.push(wire::mutate_once(rng, &mut bytes));
        }
    }
    let expected_types: Vec<Type> = names(&expected)
        .iter()
        .map(|name| find(&built.types, name))
        .collect();
    json!({
        "kind": "decode",
        "id": id,
        "env": env_id,
        "wire": wire.map(|decls| names(&decls)),
        "expected": names(&expected),
        "mutation": if mutation.is_empty() { "none".to_string() } else { mutation.join("+") },
        "hex": wire::hex(&bytes),
        "ref": wire::reference_verdict(&built.types, &bytes, &expected_types),
    })
}

fn host_verdict(built: &Built, name: &str, host: &Value) -> Value {
    let limits = candid_core::Limits::default();
    let value = match candid_core::HostValue::from_json_with_limits(&host.to_string(), &limits) {
        Ok(value) => value,
        Err(_) => return json!({ "verdict": "reject", "class": "host_value_json" }),
    };
    let Some(declaration) = built
        .contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == name)
    else {
        return json!({ "verdict": "reject", "class": "no_declaration" });
    };
    let selector = candid_core::ContractTypeRef {
        contract_id: built.contract.contract_id().to_string(),
        type_ref: declaration.ty,
    };
    match candid_core::validate_host_value(&built.contract, &selector, &value, &limits) {
        Ok(()) => json!({ "verdict": "accept" }),
        Err(error) => json!({
            "verdict": "reject",
            "class": error.violations.first().map_or("unknown".to_string(), |v| v.code.clone()),
        }),
    }
}

fn validate_case(built: &Built, rng: &mut Rng, id: String, env_id: &str) -> Value {
    let decl = rng.below(built.env.decls);
    let name = types::decl_name(decl);
    let ty = find(&built.types, &name);
    let mut value = wire::random_value(&built.types, &ty, rng, 0)
        .and_then(|value| host::descriptor(&built.types, &ty, &value))
        .unwrap_or_else(|| host::random_scalar(rng));
    let mut mutated = false;
    if rng.chance(13, 20) {
        for _ in 0..1 + rng.below(2) {
            host::mutate(rng, &mut value);
        }
        mutated = true;
    }
    let host_json = host::host_value(&built.types, &ty, &value);
    json!({
        "kind": "validate",
        "id": id,
        "env": env_id,
        "type": name,
        "mutated": mutated,
        "value": value,
        "host": host_json,
        "ref": host_verdict(built, &name, &host_json),
    })
}

fn contract_case(built: &Built, rng: &mut Rng, id: String, env_id: &str) -> Value {
    let mut document = built.envelope["contract"].clone();
    let ops = contract::random_ops(rng, &mut document);
    json!({
        "kind": "contract",
        "id": id,
        "env": env_id,
        "ops": ops,
        "ref": contract::reference_verdict(&document, &ops),
    })
}

fn env_line(env_id: &str, built: &Built) -> Value {
    json!({
        "kind": "env",
        "env": env_id,
        "did": built.env.source,
        "envelope": built.envelope,
    })
}

fn generate_env(seed: u64, seeds: &Seeds, lines: &mut Vec<Value>) {
    let mut rng = Rng::new(seed);
    let built = build_random(&mut rng);
    let env_id = format!("e{seed}");
    lines.push(env_line(&env_id, &built));
    for index in 0..seeds.decode {
        let id = format!("d/{seed}/{index}");
        lines.push(decode_case(&built, &mut rng, id, &env_id));
    }
    for index in 0..seeds.validate {
        let id = format!("v/{seed}/{index}");
        lines.push(validate_case(&built, &mut rng, id, &env_id));
    }
    for index in 0..seeds.contract {
        let id = format!("c/{seed}/{index}");
        lines.push(contract_case(&built, &mut rng, id, &env_id));
    }
}

/// The minimized regression vectors, each in its own environment.
fn generate_regressions(lines: &mut Vec<Value>) {
    let path = manifest_dir()
        .join("tests")
        .join("fixtures")
        .join("differential")
        .join("regressions.json");
    let text = std::fs::read_to_string(&path).expect("the regression vectors must be readable");
    let document: Value = serde_json::from_str(&text).expect("the regression vectors are JSON");
    for vector in document["vectors"].as_array().expect("a vectors array") {
        let name = vector["name"].as_str().expect("a name");
        let source = vector["did"].as_str().expect("a did source").to_string();
        let built = build_from_source(types::Env {
            source,
            families: Vec::new(),
            decls: 0,
        })
        .unwrap_or_else(|| panic!("regression {name}: both sides must accept its source"));
        let env_id = format!("r/{name}");
        lines.push(env_line(&env_id, &built));
        let id = format!("r/{name}");
        let case = match vector["target"].as_str() {
            Some("decode") => {
                let expected: Vec<String> = vector["expected"]
                    .as_array()
                    .expect("expected names")
                    .iter()
                    .map(|name| name.as_str().expect("a name").to_string())
                    .collect();
                let types: Vec<Type> = expected
                    .iter()
                    .map(|name| find(&built.types, name))
                    .collect();
                let hex = vector["hex"].as_str().expect("hex");
                json!({
                    "kind": "decode",
                    "id": id,
                    "env": env_id,
                    "wire": Value::Null,
                    "expected": expected,
                    "mutation": "regression",
                    "hex": hex,
                    "ref": wire::reference_verdict(&built.types, &wire::unhex(hex), &types),
                })
            }
            Some("validate") => {
                let name = vector["type"].as_str().expect("a type name");
                let ty = find(&built.types, name);
                let value = vector["value"].clone();
                let host_json = host::host_value(&built.types, &ty, &value);
                json!({
                    "kind": "validate",
                    "id": id,
                    "env": env_id,
                    "type": name,
                    "mutated": true,
                    "value": value,
                    "host": host_json,
                    "ref": host_verdict(&built, name, &host_json),
                })
            }
            Some("contract") => {
                let ops: Vec<Value> = vector["ops"].as_array().expect("ops").clone();
                let mut document = built.envelope["contract"].clone();
                for op in &ops {
                    contract::apply(&mut document, op);
                }
                json!({
                    "kind": "contract",
                    "id": id,
                    "env": env_id,
                    "ops": ops,
                    "ref": contract::reference_verdict(&document, &ops),
                })
            }
            other => panic!("regression {name}: unknown target {other:?}"),
        };
        lines.push(case);
    }
}

/// Run `work` on a thread with a large stack (so the reference's own
/// stack-based recursion guard never decides a verdict for these small
/// inputs), with the output of guarded reference panics silenced: those are
/// recorded as verdicts, not test failures.
fn on_big_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::panic::set_hook(Box::new(|info| {
        if !wire::quiet() {
            eprintln!("{info}");
        }
    }));
    let result = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(work)
        .expect("the generator thread must start")
        .join();
    let _ = std::panic::take_hook();
    result.unwrap_or_else(|_| panic!("the generator panicked outside a reference call"))
}

fn render(header: Value, lines: &[Value]) -> String {
    let mut text = String::new();
    text.push_str(&header.to_string());
    text.push('\n');
    for line in lines {
        text.push_str(&line.to_string());
        text.push('\n');
    }
    text
}

fn corpus_text() -> String {
    on_big_stack(|| {
        let seeds = CORPUS_SEEDS;
        let mut lines = Vec::new();
        for seed in seeds.start..seeds.start + seeds.envs {
            generate_env(seed, &seeds, &mut lines);
        }
        generate_regressions(&mut lines);
        let header = json!({
            "kind": "header",
            "about": "Differential corpus of issue #196. Generated by \
                      crates/candid-core-ts/tests/differential/main.rs from the committed \
                      seeds and tests/fixtures/differential/regressions.json; regenerate with \
                      UPDATE_GOLDENS=1 cargo test -p candid-core-ts --features compiler \
                      --test differential.",
            "seeds": { "start": seeds.start, "envs": seeds.envs },
            "per_env": { "decode": seeds.decode, "validate": seeds.validate, "contract": seeds.contract },
        });
        render(header, &lines)
    })
}

/// Regenerate the committed corpus and compare it with the golden. Every
/// reference verdict in it must be a decision, never a crash of the harness:
/// no reference panic and no value the domain mapping cannot express (a
/// reference panic would be a reference bug to report, not a TypeScript
/// divergence).
#[test]
fn differential_corpus_matches_reference() {
    let text = corpus_text();
    let mut failures = Vec::new();
    for line in text.lines().skip(1) {
        let case: Value = serde_json::from_str(line).expect("a JSON line");
        let verdict = case["ref"]["verdict"].as_str().unwrap_or("");
        if verdict == "panic" || verdict == "mapping_error" {
            failures.push(format!("{}: {}", case["id"], case["ref"]));
        }
    }
    assert!(failures.is_empty(), "harness failures: {failures:#?}");
    let path = manifest_dir()
        .join("tests")
        .join("goldens")
        .join("differential")
        .join("corpus.jsonl");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("goldens dir");
        std::fs::write(&path, &text).expect("golden must be writable");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {path:?}; run with UPDATE_GOLDENS=1"));
    assert!(
        text == golden,
        "the differential corpus diverged from {path:?}; regenerate deliberately with \
         UPDATE_GOLDENS=1 and review the diff"
    );
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .map(|value| value.parse().expect("a number"))
        .unwrap_or(default)
}

/// Campaign mode: `DIFF_SEED` (first environment seed), `DIFF_ENVS` (how many
/// environments), `DIFF_OUT` (the JSONL file to write), and optionally
/// `DIFF_DECODE`/`DIFF_VALIDATE`/`DIFF_CONTRACT` (cases per environment).
#[test]
#[ignore = "campaign mode: run explicitly with DIFF_SEED, DIFF_ENVS and DIFF_OUT"]
fn differential_campaign() {
    let out = std::env::var("DIFF_OUT").expect("DIFF_OUT names the output file");
    let seeds = Seeds {
        start: env_u64("DIFF_SEED", 1_000_000),
        envs: env_u64("DIFF_ENVS", 100),
        decode: env_u64("DIFF_DECODE", 200) as usize,
        validate: env_u64("DIFF_VALIDATE", 60) as usize,
        contract: env_u64("DIFF_CONTRACT", 30) as usize,
    };
    let text = on_big_stack(move || {
        let mut lines = Vec::new();
        for seed in seeds.start..seeds.start + seeds.envs {
            generate_env(seed, &seeds, &mut lines);
        }
        let header = json!({
            "kind": "header",
            "about": "Differential campaign batch (issue #196).",
            "seeds": { "start": seeds.start, "envs": seeds.envs },
            "per_env": { "decode": seeds.decode, "validate": seeds.validate, "contract": seeds.contract },
        });
        render(header, &lines)
    });
    std::fs::write(out, text).expect("DIFF_OUT must be writable");
}

/// Minimization oracle: `DIFF_IN` holds one JSON request per line —
/// `{"did", "expected", "hex"}` — and `DIFF_OUT` receives the reference
/// verdict for each, in order.
#[test]
#[ignore = "minimization mode: run explicitly with DIFF_IN and DIFF_OUT"]
fn differential_verdicts() {
    let input = std::env::var("DIFF_IN").expect("DIFF_IN names the request file");
    let out = std::env::var("DIFF_OUT").expect("DIFF_OUT names the output file");
    let text = std::fs::read_to_string(input).expect("DIFF_IN must be readable");
    let result = on_big_stack(move || {
        let mut output = String::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let request: Value = serde_json::from_str(line).expect("a JSON request");
            let source = request["did"].as_str().expect("did");
            let verdict = match reference_env(source) {
                None => json!({ "verdict": "invalid_env" }),
                Some(types) => {
                    let expected: Option<Vec<Type>> = request["expected"]
                        .as_array()
                        .expect("expected")
                        .iter()
                        .map(|name| types.find_type(name.as_str()?).ok().cloned())
                        .collect();
                    match expected {
                        None => json!({ "verdict": "invalid_env" }),
                        Some(expected) => wire::reference_verdict(
                            &types,
                            &wire::unhex(request["hex"].as_str().expect("hex")),
                            &expected,
                        ),
                    }
                }
            };
            output.push_str(&verdict.to_string());
            output.push('\n');
        }
        output
    });
    std::fs::write(out, result).expect("DIFF_OUT must be writable");
}
