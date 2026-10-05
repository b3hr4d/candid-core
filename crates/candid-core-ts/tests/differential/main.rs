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
//! Deep environments (`types::deep_env`: recursive shapes and declaration
//! chains) add decode and validate cases whose values nest up to 300 levels,
//! across the runtime's `maxDepth`. A drafted environment either side refuses
//! is redrawn, and the corpus header counts redraws by reason.
//!
//! Every decode verdict carries `flags`: input properties the Rust side reads
//! from the bytes at their own wire types (`wire::Scan`) or from the types,
//! which the TypeScript runner requires before it attributes a symptom to an
//! intended difference or to the reference — never anything the runtime says.
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
//! of your choosing; `ts/tests/differential/campaign.ts` replays it, and
//! `differential_verdicts` (ignored) answers the reference's verdict for
//! hand-minimized inputs. The procedure is in `docs/verification.md` (the
//! differential-fuzz entry under "Enforced in this repository").
#![cfg(feature = "compiler")]

mod contract;
mod host;
mod rng;
mod types;
mod wire;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use candid::types::Type;
use candid::TypeEnv;
use candid_parser::{check_prog, IDLProg};
use serde_json::{json, Value};

use rng::Rng;

/// The committed corpus: random environments `start..start + envs`, each
/// with a fixed number of cases per target, and deep environments
/// `start..start + deep_envs` (see `types::deep_env`), each with a fixed
/// number of decode and validate cases.
struct Seeds {
    start: u64,
    envs: u64,
    decode: usize,
    validate: usize,
    contract: usize,
    deep_envs: u64,
    deep_decode: usize,
    deep_validate: usize,
}

const CORPUS_SEEDS: Seeds = Seeds {
    start: 1,
    envs: 24,
    decode: 40,
    validate: 12,
    contract: 8,
    deep_envs: 6,
    deep_decode: 4,
    deep_validate: 2,
};

/// Deep environments draw from their own stream: seed `s` of a deep
/// environment is not the random environment of seed `s`.
const DEEP_SALT: u64 = 0xdee9_0000_0000_0000;

/// Environments drafted and redrawn because one side or both refused the
/// source, keyed `reference=<outcome> compiler=<outcome>`: the corpus header
/// carries the counts, so a shape the compiler (or the reference) refuses
/// systematically shows up as a count instead of silently leaving coverage.
type Redraws = BTreeMap<String, u64>;

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
    /// Contract nodes holding a field spelled two ways (see
    /// `label_collisions`).
    collisions: BTreeSet<u64>,
}

impl Built {
    /// Whether any of the named declarations reaches a node in
    /// `collisions` through the Contract graph: only there does
    /// `env:label-collision` apply to a case.
    fn reaches_collision(&self, names: &[String]) -> bool {
        if self.collisions.is_empty() {
            return false;
        }
        let contract = &self.envelope["contract"];
        let types = contract["types"].as_array().map_or(&[][..], Vec::as_slice);
        let mut stack: Vec<u64> = contract["declarations"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter(|declaration| {
                names
                    .iter()
                    .any(|name| declaration["name"].as_str() == Some(name))
            })
            .filter_map(|declaration| declaration["type"].as_u64())
            .collect();
        let mut seen = BTreeSet::new();
        while let Some(index) = stack.pop() {
            if !seen.insert(index) {
                continue;
            }
            if self.collisions.contains(&index) {
                return true;
            }
            let Some(node) = types.get(index as usize) else {
                continue;
            };
            stack.extend(node["inner"].as_u64());
            for key in ["fields", "methods"] {
                for child in node[key].as_array().map_or(&[][..], Vec::as_slice) {
                    stack.extend(child["type"].as_u64());
                }
            }
            for key in ["args", "results"] {
                for child in node[key].as_array().map_or(&[][..], Vec::as_slice) {
                    stack.extend(child.as_u64());
                }
            }
        }
        false
    }

    /// Add the case-level `label_collision` flag to a reference verdict.
    fn flag_collision(&self, names: &[String], verdict: &mut Value) {
        if self.reaches_collision(names) {
            let mut flags = verdict["flags"].as_array().cloned().unwrap_or_default();
            flags.push(json!("label_collision"));
            verdict["flags"] = Value::Array(flags);
        }
    }
}

fn reference_env(source: &str) -> Result<TypeEnv, &'static str> {
    let prog: IDLProg = source.parse().map_err(|_| "parse")?;
    let mut env = TypeEnv::new();
    check_prog(&mut env, &prog).map_err(|_| "check")?;
    Ok(env)
}

/// The canonical containers that hold one field under two source
/// spellings: the same `(container, id)` written with a name in one
/// declaration and numbered (or under another name) in another. The
/// field-name table attaches names to canonical nodes, so the runtime keys
/// every occurrence by the one name, while the reference reads each
/// declaration's own labels.
fn label_collisions(compilation: &candid_core::Compilation) -> BTreeSet<u64> {
    let Some(info) = compilation.source_info() else {
        return BTreeSet::new();
    };
    let mut spellings: BTreeMap<(u32, u32), BTreeSet<String>> = BTreeMap::new();
    for provenance in info.field_labels() {
        let spelling = match &provenance.label {
            candid_core::SourceLabel::Named { name } => format!("name:{name}"),
            _ => "number".to_string(),
        };
        spellings
            .entry((provenance.container, provenance.id))
            .or_default()
            .insert(spelling);
    }
    spellings
        .into_iter()
        .filter(|(_, set)| set.len() > 1)
        .map(|((container, _), _)| u64::from(container))
        .collect()
}

type Envelope = (Value, candid_core::Contract, BTreeSet<u64>);

/// The Contract of `source` as the one-document envelope the TypeScript
/// loader reads, normalized as `wire_vectors.rs` normalizes the coercion
/// golden's (a fixed producer block, a canonical reparse, field names in the
/// `org.candid-core.field-names/v1` extension).
fn envelope_of(source: &str) -> Result<Envelope, &'static str> {
    let compilation = candid_core::compile_did(source).map_err(|_| "error")?;
    let collisions = label_collisions(&compilation);
    let mut document = serde_json::to_value(compilation.contract()).map_err(|_| "serialize")?;
    document["producer"] = json!({
        "name": "candid-core",
        "version": "0.0.0-golden",
        "candid_version": "0.0.0-golden",
        "candid_parser_version": "0.0.0-golden",
    });
    let contract =
        candid_core::Contract::from_json(&document.to_string()).map_err(|_| "contract")?;
    let mut names = BTreeMap::new();
    let info = compilation.source_info().ok_or("source_info")?;
    for provenance in info.field_labels() {
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
        .map_err(|_| "extension")?;
    let envelope = serde_json::to_value(&envelope).map_err(|_| "serialize")?;
    Ok((envelope, contract, collisions))
}

/// Both sides over one source: the environment, or why it is redrawn
/// (`reference=<ok|parse|check|panic> compiler=<ok|error|contract|…|panic>`).
fn build_from_source(env: types::Env) -> Result<Built, String> {
    let reference = wire::guarded(|| reference_env(&env.source)).unwrap_or(Err("panic"));
    let compiled = wire::guarded(|| envelope_of(&env.source)).unwrap_or(Err("panic"));
    match (reference, compiled) {
        (Ok(types), Ok((envelope, contract, collisions))) => Ok(Built {
            env,
            types,
            envelope,
            contract,
            collisions,
        }),
        (reference, compiled) => Err(format!(
            "reference={} compiler={}",
            reference.err().unwrap_or("ok"),
            compiled.err().unwrap_or("ok")
        )),
    }
}

/// An environment both sides accept; rejected drafts are redrawn from the
/// same stream (so the result is still a function of the seed) and counted.
fn build_drawn(rng: &mut Rng, redraws: &mut Redraws, draw: fn(&mut Rng) -> types::Env) -> Built {
    loop {
        match build_from_source(draw(rng)) {
            Ok(built) => return built,
            Err(reason) => *redraws.entry(reason).or_default() += 1,
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
    decode_line(built, id, env_id, wire, &expected, &mutation, &bytes)
}

fn decode_line(
    built: &Built,
    id: String,
    env_id: &str,
    wire: Option<Vec<usize>>,
    expected: &[usize],
    mutation: &[&str],
    bytes: &[u8],
) -> Value {
    let expected = names(expected);
    let expected_types: Vec<Type> = expected
        .iter()
        .map(|name| find(&built.types, name))
        .collect();
    let mut verdict = wire::reference_verdict(&built.types, bytes, &expected_types);
    built.flag_collision(&expected, &mut verdict);
    json!({
        "kind": "decode",
        "id": id,
        "env": env_id,
        "wire": wire.map(|decls| names(&decls)),
        "expected": expected,
        "mutation": if mutation.is_empty() { "none".to_string() } else { mutation.join("+") },
        "hex": wire::hex(bytes),
        "ref": verdict,
    })
}

fn host_verdict(built: &Built, name: &str, host: &Value) -> Value {
    let limits = candid_core::Limits::default();
    let value = match candid_core::HostValue::from_json_with_limits(&host.to_string(), &limits) {
        Ok(value) => value,
        Err(candid_core::HostValueJsonError::Malformed(_)) => {
            return json!({ "verdict": "reject", "class": "host_value_json" });
        }
        // Size, nesting, depth and element budgets of the HostValue JSON
        // decoder: a policy, distinct from a malformed value.
        Err(_) => return json!({ "verdict": "reject", "class": "host_value_limit" }),
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
    let value = wire::random_value(&built.types, &ty, rng, 0)
        .and_then(|value| host::descriptor(&built.types, &ty, &value))
        .unwrap_or_else(|| host::random_scalar(rng));
    validate_line(built, rng, id, env_id, &name, value)
}

fn validate_line(
    built: &Built,
    rng: &mut Rng,
    id: String,
    env_id: &str,
    name: &str,
    mut value: Value,
) -> Value {
    let ty = find(&built.types, name);
    let mut mutated = false;
    match rng.below(20) {
        0..=6 => {}
        7..=12 => {
            host::boundary(rng, &built.types, &ty, &mut value);
            mutated = true;
        }
        _ => {
            for _ in 0..1 + rng.below(2) {
                host::mutate(rng, &mut value);
            }
            mutated = true;
        }
    }
    let host_json = host::host_value(&built.types, &ty, &value);
    let mut verdict = host_verdict(built, name, &host_json);
    built.flag_collision(&[name.to_string()], &mut verdict);
    json!({
        "kind": "validate",
        "id": id,
        "env": env_id,
        "type": name,
        "mutated": mutated,
        "value": value,
        "host": host_json,
        "ref": verdict,
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
        "ref": contract::judge(&built.envelope["contract"], &ops),
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

fn generate_env(seed: u64, seeds: &Seeds, lines: &mut Vec<Value>, redraws: &mut Redraws) {
    let mut rng = Rng::new(seed);
    let built = build_drawn(&mut rng, redraws, types::random_env);
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

/// One deep decode case: a value of `T0` nested about `levels` deep, half of
/// the time near the runtime's depth boundary (see `wire::DEEP_LEVELS`),
/// optionally mutated, read at `T0` or a sibling.
fn deep_decode_case(built: &Built, rng: &mut Rng, id: String, env_id: &str) -> Value {
    let wire_ty = find(&built.types, &types::decl_name(0));
    let levels = if rng.chance(1, 2) {
        wire::DEEP_LEVELS - 8 + rng.below(16)
    } else {
        rng.below(300)
    };
    let value = wire::deep_value(&built.types, &wire_ty, rng, levels)
        .or_else(|| wire::random_value(&built.types, &wire_ty, rng, 0));
    let bytes = value
        .and_then(|value| wire::encode(&built.types, std::slice::from_ref(&wire_ty), vec![value]));
    let mut mutation = vec!["deep"];
    let (wire, bytes) = match bytes {
        Some(bytes) => (Some(vec![0]), bytes),
        None => {
            mutation.push("raw");
            (None, raw_bytes(rng))
        }
    };
    let mut bytes = bytes;
    if wire.is_some() && rng.chance(1, 4) {
        mutation.push(wire::mutate_once(rng, &mut bytes));
    }
    let expected = if rng.chance(2, 3) {
        0
    } else {
        *rng.pick(family_of(&built.env, 0))
    };
    decode_line(built, id, env_id, wire, &[expected], &mutation, &bytes)
}

/// One deep validate case: a value of `T0` nested up to 80 levels, across
/// the HostValue JSON decoder's 64-container nesting cap.
fn deep_validate_case(built: &Built, rng: &mut Rng, id: String, env_id: &str) -> Value {
    let name = types::decl_name(0);
    let ty = find(&built.types, &name);
    let levels = rng.below(80);
    let value = wire::deep_value(&built.types, &ty, rng, levels)
        .and_then(|value| host::descriptor(&built.types, &ty, &value))
        .unwrap_or_else(|| host::random_scalar(rng));
    validate_line(built, rng, id, env_id, &name, value)
}

fn generate_deep_env(seed: u64, seeds: &Seeds, lines: &mut Vec<Value>, redraws: &mut Redraws) {
    let mut rng = Rng::new(seed ^ DEEP_SALT);
    let built = build_drawn(&mut rng, redraws, types::deep_env);
    let env_id = format!("x{seed}");
    lines.push(env_line(&env_id, &built));
    for index in 0..seeds.deep_decode {
        let id = format!("xd/{seed}/{index}");
        lines.push(deep_decode_case(&built, &mut rng, id, &env_id));
    }
    for index in 0..seeds.deep_validate {
        let id = format!("xv/{seed}/{index}");
        lines.push(deep_validate_case(&built, &mut rng, id, &env_id));
    }
}

/// Every environment and case of `seeds`, in order, and the redraw counts.
fn generate(seeds: &Seeds, lines: &mut Vec<Value>) -> Redraws {
    let mut redraws = Redraws::new();
    for seed in seeds.start..seeds.start + seeds.envs {
        generate_env(seed, seeds, lines, &mut redraws);
    }
    for seed in seeds.start..seeds.start + seeds.deep_envs {
        generate_deep_env(seed, seeds, lines, &mut redraws);
    }
    redraws
}

fn header(about: &str, seeds: &Seeds, redraws: &Redraws) -> Value {
    json!({
        "kind": "header",
        "about": about,
        "seeds": { "start": seeds.start, "envs": seeds.envs, "deep_envs": seeds.deep_envs },
        "per_env": { "decode": seeds.decode, "validate": seeds.validate, "contract": seeds.contract },
        "per_deep_env": { "decode": seeds.deep_decode, "validate": seeds.deep_validate },
        "redraws": redraws,
    })
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
        .unwrap_or_else(|reason| {
            panic!("regression {name}: both sides must accept its source ({reason})")
        });
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
                let mut verdict = wire::reference_verdict(&built.types, &wire::unhex(hex), &types);
                built.flag_collision(&expected, &mut verdict);
                json!({
                    "kind": "decode",
                    "id": id,
                    "env": env_id,
                    "wire": Value::Null,
                    "expected": expected,
                    "mutation": "regression",
                    "hex": hex,
                    "ref": verdict,
                })
            }
            Some("validate") => {
                let name = vector["type"].as_str().expect("a type name");
                let ty = find(&built.types, name);
                let value = vector["value"].clone();
                let host_json = host::host_value(&built.types, &ty, &value);
                let mut verdict = host_verdict(&built, name, &host_json);
                built.flag_collision(&[name.to_string()], &mut verdict);
                json!({
                    "kind": "validate",
                    "id": id,
                    "env": env_id,
                    "type": name,
                    "mutated": true,
                    "value": value,
                    "host": host_json,
                    "ref": verdict,
                })
            }
            Some("contract") => {
                let ops: Vec<Value> = vector["ops"].as_array().expect("ops").clone();
                json!({
                    "kind": "contract",
                    "id": id,
                    "env": env_id,
                    "ops": ops,
                    "ref": contract::judge(&built.envelope["contract"], &ops),
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

/// The committed corpus, its lines kept as values: a deep case nests past
/// `serde_json`'s 128-level parse limit, so nothing re-reads the rendered text.
fn corpus() -> (Value, Vec<Value>) {
    on_big_stack(|| {
        let seeds = CORPUS_SEEDS;
        let mut lines = Vec::new();
        let redraws = generate(&seeds, &mut lines);
        generate_regressions(&mut lines);
        let header = header(
            "Differential corpus of issue #196. Generated by \
             crates/candid-core-ts/tests/differential/main.rs from the committed \
             seeds and tests/fixtures/differential/regressions.json; regenerate with \
             UPDATE_GOLDENS=1 cargo test -p candid-core-ts --features compiler \
             --test differential.",
            &seeds,
            &redraws,
        );
        (header, lines)
    })
}

/// Regenerate the committed corpus and compare it with the golden. Every
/// reference verdict in it must be a decision, never a crash of the harness:
/// no reference panic and no value the domain mapping cannot express (a
/// reference panic would be a reference bug to report, not a TypeScript
/// divergence).
#[test]
fn differential_corpus_matches_reference() {
    let (header, lines) = corpus();
    let failures: Vec<String> = lines
        .iter()
        .filter(|line| {
            matches!(
                line["ref"]["verdict"].as_str(),
                Some("panic" | "mapping_error")
            )
        })
        .map(|line| format!("{}: {}", line["id"], line["ref"]))
        .collect();
    assert!(failures.is_empty(), "harness failures: {failures:#?}");
    let text = render(header, &lines);
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
    // docs/verification.md states the committed count; keep the two equal.
    let seeds = CORPUS_SEEDS;
    let stated = format!(
        "{} environments × {} decode, {} validate and {} contract cases, and {} deep \
         environments × {} decode and {} validate cases",
        seeds.envs,
        seeds.decode,
        seeds.validate,
        seeds.contract,
        seeds.deep_envs,
        seeds.deep_decode,
        seeds.deep_validate
    );
    let verification = std::fs::read_to_string(manifest_dir().join("../../docs/verification.md"))
        .expect("docs/verification.md must be readable");
    assert!(
        verification.contains(&stated),
        "docs/verification.md must state the committed corpus as `{stated}`"
    );
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
/// `DIFF_DECODE`/`DIFF_VALIDATE`/`DIFF_CONTRACT` (cases per environment),
/// `DIFF_DEEP_ENVS` (deep environments) and `DIFF_DEEP_DECODE`/
/// `DIFF_DEEP_VALIDATE` (cases per deep environment).
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
        deep_envs: env_u64("DIFF_DEEP_ENVS", 10),
        deep_decode: env_u64("DIFF_DEEP_DECODE", 20) as usize,
        deep_validate: env_u64("DIFF_DEEP_VALIDATE", 5) as usize,
    };
    let text = on_big_stack(move || {
        let mut lines = Vec::new();
        let redraws = generate(&seeds, &mut lines);
        let header = header(
            "Differential campaign batch (issue #196).",
            &seeds,
            &redraws,
        );
        render(header, &lines)
    });
    std::fs::write(out, text).expect("DIFF_OUT must be writable");
}

/// Minimization oracle: `DIFF_IN` holds one JSON request per line —
/// `{"did", "expected", "hex"}`, or `{"did", "expected", "wire", "textual"}`
/// to have the reference encode a textual value at the wire declarations
/// first — and `DIFF_OUT` receives the reference verdict for each, in order
/// (with the encoded `hex` for a textual request).
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
                Err(_) => json!({ "verdict": "invalid_env" }),
                Ok(types) => {
                    let expected: Option<Vec<Type>> = request["expected"]
                        .as_array()
                        .expect("expected")
                        .iter()
                        .map(|name| types.find_type(name.as_str()?).ok().cloned())
                        .collect();
                    let bytes = match request["textual"].as_str() {
                        None => Some(wire::unhex(request["hex"].as_str().expect("hex"))),
                        Some(textual) => {
                            let wire_types: Option<Vec<Type>> = request["wire"]
                                .as_array()
                                .expect("wire")
                                .iter()
                                .map(|name| types.find_type(name.as_str()?).ok().cloned())
                                .collect();
                            wire_types.and_then(|wire_types| {
                                candid_parser::parse_idl_args(textual)
                                    .ok()?
                                    .annotate_types(true, &types, &wire_types)
                                    .ok()?
                                    .to_bytes_with_types(&types, &wire_types)
                                    .ok()
                            })
                        }
                    };
                    match (expected, bytes) {
                        (Some(expected), Some(bytes)) => {
                            let mut verdict = wire::reference_verdict(&types, &bytes, &expected);
                            verdict["hex"] = json!(wire::hex(&bytes));
                            verdict
                        }
                        _ => json!({ "verdict": "invalid_env" }),
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

/// Rejudge mode: `DIFF_IN` is a corpus or campaign batch (possibly written by
/// an earlier generator), `DIFF_OUT` receives it with every environment
/// rebuilt from its `did` and every reference verdict recomputed by this
/// tree — so an earlier campaign's inputs can be classified again under the
/// current verdict mapping and flags. A rebuilt envelope that differs from
/// the recorded one is a harness failure.
#[test]
#[ignore = "rejudge mode: run explicitly with DIFF_IN and DIFF_OUT"]
fn differential_rejudge() {
    let input = std::env::var("DIFF_IN").expect("DIFF_IN names the batch to rejudge");
    let out = std::env::var("DIFF_OUT").expect("DIFF_OUT names the output file");
    let text = std::fs::read_to_string(input).expect("DIFF_IN must be readable");
    let result = on_big_stack(move || {
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header: Value = serde_json::from_str(lines.next().expect("a header")).expect("JSON");
        let mut output = Vec::new();
        let mut built: Option<Built> = None;
        for line in lines {
            let mut case: Value = serde_json::from_str(line).expect("a JSON line");
            match case["kind"].as_str() {
                Some("env") => {
                    let rebuilt = build_from_source(types::Env {
                        source: case["did"].as_str().expect("did").to_string(),
                        families: Vec::new(),
                        decls: 0,
                    })
                    .unwrap_or_else(|reason| panic!("{}: {reason}", case["env"]));
                    assert!(
                        rebuilt.envelope == case["envelope"],
                        "{}: the envelope rebuilt differently",
                        case["env"]
                    );
                    let id = case["env"].as_str().expect("env").to_string();
                    output.push(env_line(&id, &rebuilt));
                    built = Some(rebuilt);
                    continue;
                }
                Some("decode") => {
                    let built = built.as_ref().expect("an environment first");
                    let expected: Vec<String> = case["expected"]
                        .as_array()
                        .expect("expected")
                        .iter()
                        .map(|name| name.as_str().expect("a name").to_string())
                        .collect();
                    let types: Vec<Type> = expected
                        .iter()
                        .map(|name| find(&built.types, name))
                        .collect();
                    let bytes = wire::unhex(case["hex"].as_str().expect("hex"));
                    let mut verdict = wire::reference_verdict(&built.types, &bytes, &types);
                    built.flag_collision(&expected, &mut verdict);
                    case["ref"] = verdict;
                }
                Some("validate") => {
                    let built = built.as_ref().expect("an environment first");
                    let name = case["type"].as_str().expect("type").to_string();
                    let ty = find(&built.types, &name);
                    let host_json = host::host_value(&built.types, &ty, &case["value"]);
                    let mut verdict = host_verdict(built, &name, &host_json);
                    built.flag_collision(&[name], &mut verdict);
                    case["host"] = host_json;
                    case["ref"] = verdict;
                }
                Some("contract") => {
                    let built = built.as_ref().expect("an environment first");
                    let ops = case["ops"].as_array().expect("ops").clone();
                    case["ref"] = contract::judge(&built.envelope["contract"], &ops);
                }
                other => panic!("unknown line kind {other:?}"),
            }
            output.push(case);
        }
        render(header, &output)
    });
    std::fs::write(out, result).expect("DIFF_OUT must be writable");
}
