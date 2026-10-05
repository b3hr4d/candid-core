//! Issue #196: the differential fuzz of the TypeScript runtime against the
//! Rust reference, Rust half.
//!
//! This driver generates seeded random cases for three targets and records
//! the reference's verdict for each:
//!
//! - **decode** — `(wire types, bytes, expected types)` triples: a random
//!   value at the wire types, encoded by the `candid` crate, optionally
//!   mutated (byte edits, a LEB128 group made non-minimal, or a structurally
//!   invalid type table), decoded by `IDLArgs::from_bytes_with_types` (see
//!   `wire.rs`); the TypeScript runner feeds the bytes to `decodeArgs` with
//!   schemas `schemaFromContract` builds from the same declarations;
//! - **validate** — a JavaScript domain value converted to a HostValue and
//!   judged by `validate_host_value` (see `host.rs`) under limits aligned
//!   with the runtime's depth budget (`VALIDATE_DEPTH`); the runner calls
//!   `validate` with the same value;
//! - **contract** — a compiled Contract document edited by JSON operations
//!   and judged by `Contract::from_json` (see `contract.rs`); the runner calls
//!   `schemaFromContract` on the same edited document.
//!
//! Each random environment is a `.did` source (see `types.rs`); the reference
//! types come from `candid_parser` over that source, and the Contract the
//! TypeScript side loads comes from candid-core's compiler over the same text.
//! Deep environments (`types::deep_env`: recursive shapes and declaration
//! chains) add decode and validate cases whose values nest close to the
//! runtime's depth bound. A drafted environment either side refuses, or one
//! holding an `opt`-only cycle (`wire::opt_cycle`), is redrawn, and the corpus
//! header counts redraws by reason. Since #234 candid-core's compiler refuses
//! an `opt`-only cycle itself, so such a draft is counted as a compiler
//! refusal (`reference=ok compiler=error`); the `opt_cycle` check stays as a
//! backstop, and a count under it would mean the compiler accepted one. A
//! regression vector whose source the compiler refuses supplies its Contract
//! instead (`build_with_contract`).
//!
//! # No case is judged outside what both sides judge
//!
//! The reference records a verdict for every case, or `inconclusive` when it
//! refused on a budget of its own (its decoding quota, its stack guard, a
//! HostValue JSON budget past which the constructors could not rebuild the
//! value) and so never judged the input. Its budgets that are configured like
//! the runtime's are verdicts, not budgets of its own: decode's type-table
//! size (`wire::GEN_TABLE_ENTRIES`) and validate's value depth and elements
//! (`host_verdict`). Generation is sized so that no other budget is hit: a
//! decode case whose message is outside the bounds of
//! `wire::Scan::outside_bounds` (nesting and values, the runtime budgets the
//! `candid` crate has no counterpart for, and a claimed table size) is redrawn
//! before it is judged, and counted in the header's `case_redraws`. The
//! committed corpus must hold no `inconclusive` case; a campaign counts them.
//! Exact regression vectors pin each boundary (`regressions.json`).
//!
//! # The committed corpus (CI)
//!
//! `differential_corpus_matches_reference` regenerates the corpus from the
//! committed seeds (`CORPUS_SEEDS`, a fixed count, no time budget — #39) plus
//! the minimized regression vectors in
//! `tests/fixtures/differential/regressions.json`, and compares it with
//! `tests/goldens/differential/corpus.jsonl` byte for byte. The TypeScript
//! suite (`ts/tests/differential.test.ts`) replays that file and fails on any
//! divergence whose case id and exact symptom are not in the reviewed list
//! `tests/goldens/differential/divergences.json`, and on any listed case
//! that no longer diverges so. Regenerate deliberately with
//! `UPDATE_GOLDENS=1 cargo test -p candid-core-ts --features compiler --test
//! differential`, then review the diff.
//!
//! # Campaign mode
//!
//! `differential_campaign` (ignored) writes a corpus for other seeds to a file
//! of your choosing; `ts/tests/differential/campaign.ts` replays it;
//! `differential_rejudge` (ignored) rebuilds each environment of an existing
//! batch from its `did` and recomputes every reference verdict with the
//! current tree; and `differential_verdicts` (ignored) answers the reference's
//! verdict for hand-minimized inputs. The procedure is in
//! `docs/verification.md` (the differential-fuzz entry under "Enforced in this
//! repository").
#![cfg(feature = "compiler")]

mod contract;
mod host;
mod rng;
mod types;
mod wire;

use std::collections::BTreeMap;
use std::path::PathBuf;

use candid::types::Type;
use candid::TypeEnv;
use candid_parser::{check_prog, IDLProg};
use serde_json::{json, Value};

use rng::Rng;

/// The committed corpus: random environments `start..start + envs`, each
/// with a fixed number of cases per target, and deep environments
/// `start..start + deep_envs` (see `types::deep_env`), each with a fixed
/// number of decode and validate cases. With `deep_kinds`, deep environment
/// `start + i` takes kind `i % types::DEEP_KINDS` (every recursive shape and
/// a chain past 128 constructors, whatever the seeds draw) instead of drawing
/// one.
struct Seeds {
    start: u64,
    envs: u64,
    decode: usize,
    validate: usize,
    contract: usize,
    deep_envs: u64,
    deep_decode: usize,
    deep_validate: usize,
    deep_kinds: bool,
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
    deep_kinds: true,
};

/// Deep environments draw from their own stream: seed `s` of a deep
/// environment is not the random environment of seed `s`.
const DEEP_SALT: u64 = 0xdee9_0000_0000_0000;

/// The value depth the reference's HostValue validator is configured with:
/// the runtime's `validate` refuses the first node at Candid level 128 under
/// its default `maxDepth` of 256 (a Contract-loaded schema charges a `rec`
/// hop and a constructor per level, issue #231), and
/// `validate_host_value` refuses the first node past level `max_value_depth`,
/// so 127 makes the two budgets the same for every node the runtime steps
/// on. The one place they still differ is a variant arm whose payload is
/// `null`: the runtime charges the arm's `rec` hop but never steps on the
/// `null`, the reference charges the `null` as a node one level down, so
/// such an arm 128 levels down is accepted there and refused here. Random
/// generation stays below that level (`wire::GEN_LEVELS`), and an exact
/// vector pins it (`validate_depth_variant_null_128_levels`, listed with
/// #231).
const VALIDATE_DEPTH: usize = 127;

/// The runtime's default budgets, as the corpus header states them: the
/// TypeScript suite checks them against `codec.ts`'s and `validate.ts`'s
/// `DEFAULT_MAX_*`, so the header cannot drift from the runtime it pins.
/// Generated cases stay below every one of them (`bounds`), and exact
/// regression vectors (`regressions.json`) pin each exactly at the bound and
/// past it (one step past wherever the shape allows) on the paths they
/// name, and nowhere else: `maxBytes` (`bytes_text_message_*`),
/// `maxTypeTableEntries` (`table_entries_*`, aligned with the reference's
/// `max_type_len`), `maxNumericBytes` for a skipped `nat` and `int`
/// (`numeric_*_groups`), `maxDepth` on the decoded
/// variant, opt, record and vec chains, coercion-inserted `opt`s, the skip of
/// an extra field, an absorbed value, an expected `reserved` argument and
/// field, an extra argument (vec, opt and variant chains) and a record nested
/// in a skipped value, and on
/// validate's vec, variant, record, tuple and opt chains; and `maxElements`
/// for a skipped vec (of `null`, of a variant, of a non-empty record), a
/// decoded vec of `null`, a blob and a decoded vec of mixed elements (record,
/// variant, `opt`, tuple, text), and validate's vec, record and mixed values.
fn runtime_budgets() -> Value {
    json!({
        "decode": {
            "maxBytes": 10_485_760,
            "maxTypeTableEntries": wire::GEN_TABLE_ENTRIES,
            "maxDepth": 256,
            "maxElements": 1_000_000,
            "maxNumericBytes": 1_048_576,
        },
        "validate": { "maxDepth": 256, "maxElements": 1_000_000 },
    })
}

/// How many drafts a case may take to fall within the generation bounds
/// before the generator gives up (a generator defect, not a verdict).
const MAX_DRAFTS: usize = 256;

/// Redraws by reason: environments one side or both refused (keyed
/// `reference=<outcome> compiler=<outcome>`; candid-core's compiler refuses an
/// `opt`-only cycle since #234) or that compile although they hold an
/// `opt`-only cycle (`opt_cycle`), and cases drafted outside the generation
/// bounds (keyed
/// `<target>:<bound>`). The corpus header carries the counts, so a shape that
/// is redrawn systematically shows up as a count instead of silently leaving
/// coverage.
#[derive(Default)]
struct Redraws {
    envs: BTreeMap<String, u64>,
    cases: BTreeMap<String, u64>,
}

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
    /// The diagnostic code candid-core's compiler refuses the source with,
    /// for a regression vector that supplies its Contract instead
    /// (`build_with_contract`); `None` when the envelope was compiled.
    compile_refused: Option<String>,
}

fn reference_env(source: &str) -> Result<TypeEnv, &'static str> {
    let prog: IDLProg = source.parse().map_err(|_| "parse")?;
    let mut env = TypeEnv::new();
    check_prog(&mut env, &prog).map_err(|_| "check")?;
    Ok(env)
}

/// The Contract of `source` as the one-document envelope the TypeScript
/// loader reads, normalized as `wire_vectors.rs` normalizes the coercion
/// golden's (a fixed producer block, a canonical reparse, field names in the
/// `org.candid-core.field-names/v1` extension).
fn envelope_of(source: &str) -> Result<(Value, candid_core::Contract), &'static str> {
    let compilation = candid_core::compile_did(source).map_err(|_| "error")?;
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
    envelope_with_names(contract, triples)
}

/// The envelope of a normalized Contract and its field-name triples.
fn envelope_with_names(
    contract: candid_core::Contract,
    triples: Vec<Value>,
) -> Result<(Value, candid_core::Contract), &'static str> {
    let mut envelope = candid_core::ContractEnvelope::new(contract.clone());
    envelope
        .insert_extension(
            "org.candid-core.field-names/v1",
            Value::Array(triples),
            &candid_core::Limits::default(),
        )
        .map_err(|_| "extension")?;
    let envelope = serde_json::to_value(&envelope).map_err(|_| "serialize")?;
    Ok((envelope, contract))
}

/// Both sides over one source: the environment, or why it is redrawn
/// (`reference=<ok|parse|check|panic> compiler=<ok|error|contract|…|panic>`).
fn build_from_source(env: types::Env) -> Result<Built, String> {
    let reference = wire::guarded(|| reference_env(&env.source)).unwrap_or(Err("panic"));
    let compiled = wire::guarded(|| envelope_of(&env.source)).unwrap_or(Err("panic"));
    match (reference, compiled) {
        (Ok(types), Ok((envelope, contract))) => Ok(Built {
            env,
            types,
            envelope,
            contract,
            compile_refused: None,
        }),
        (reference, compiled) => Err(format!(
            "reference={} compiler={}",
            reference.err().unwrap_or("ok"),
            compiled.err().unwrap_or("ok")
        )),
    }
}

/// A regression environment whose source the reference accepts and
/// candid-core's compiler refuses (an `opt`-only cycle since #234), run on
/// the Contract the vector supplies (`types`, `declarations` and an optional
/// `actor`, as a Contract document writes them): the Contract loaders still
/// accept such a graph, so the runtime still meets it. The Contract is built
/// through the model and normalized as `envelope_of` normalizes a compiled
/// one; it carries no field names. Panics unless the compiler does refuse
/// the source, so a supplied Contract never stands in for one it compiles.
fn build_with_contract(source: &str, document: &Value) -> Result<Built, String> {
    let types = reference_env(source).map_err(|reason| format!("reference={reason}"))?;
    let code = match candid_core::compile_did(source) {
        Ok(_) => return Err("the compiler accepts the source; drop its contract".to_string()),
        Err(error) => error.diagnostics[0].code.clone(),
    };
    let nodes: Vec<candid_core::TypeNode> =
        serde_json::from_value(document["types"].clone()).map_err(|e| e.to_string())?;
    let declarations: Vec<candid_core::Declaration> =
        serde_json::from_value(document["declarations"].clone()).map_err(|e| e.to_string())?;
    let actor: Option<candid_core::Actor> = match document.get("actor") {
        None => None,
        Some(actor) => Some(serde_json::from_value(actor.clone()).map_err(|e| e.to_string())?),
    };
    let draft = candid_core::ContractDraft::new(nodes, declarations, actor);
    let built = draft.build().map_err(|error| format!("{error:?}"))?;
    let mut normalized = serde_json::to_value(&built).map_err(|e| e.to_string())?;
    normalized["producer"] = json!({
        "name": "candid-core",
        "version": "0.0.0-golden",
        "candid_version": "0.0.0-golden",
        "candid_parser_version": "0.0.0-golden",
    });
    let contract =
        candid_core::Contract::from_json(&normalized.to_string()).map_err(|e| e.to_string())?;
    let (envelope, contract) = envelope_with_names(contract, Vec::new())?;
    Ok(Built {
        env: types::Env {
            source: source.to_string(),
            families: Vec::new(),
            decls: 0,
        },
        types,
        envelope,
        contract,
        compile_refused: Some(code),
    })
}

/// An environment both sides accept and that holds no `opt`-only cycle;
/// rejected drafts are redrawn from the same stream (so the result is still
/// a function of the seed) and counted.
fn build_drawn(
    rng: &mut Rng,
    redraws: &mut Redraws,
    mut draw: impl FnMut(&mut Rng) -> types::Env,
) -> Built {
    loop {
        let reason = match build_from_source(draw(rng)) {
            Ok(built) => {
                let names: Vec<String> = (0..built.env.decls).map(types::decl_name).collect();
                if !wire::opt_cycle(&built.types, &names) {
                    return built;
                }
                "opt_cycle".to_string()
            }
            Err(reason) => reason,
        };
        *redraws.envs.entry(reason).or_default() += 1;
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

/// One drafted decode case, before the generation bounds are checked.
struct DecodeDraft {
    wire: Option<Vec<usize>>,
    expected: Vec<usize>,
    mutation: Vec<&'static str>,
    bytes: Vec<u8>,
}

fn draft_decode(built: &Built, rng: &mut Rng) -> DecodeDraft {
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
    DecodeDraft {
        wire,
        expected,
        mutation,
        bytes,
    }
}

/// A drafted decode case judged by the reference, or why it is outside the
/// generation bounds: what the scan decides is checked before the reference
/// runs (`wire::Scan::outside_bounds`), the nesting at the expected types
/// after (`wire::Scan::too_deep`).
fn judge_decode(
    built: &Built,
    draft: DecodeDraft,
    env_id: &str,
    id: &str,
) -> (Value, Option<&'static str>) {
    let expected = names(&draft.expected);
    let types: Vec<Type> = expected
        .iter()
        .map(|name| find(&built.types, name))
        .collect();
    let Some(expansion) = wire::expansion(&built.types, &types) else {
        return (Value::Null, Some("opt_cycle"));
    };
    let scan = wire::scan(&draft.bytes);
    if let Some(bound) = scan.outside_bounds(expansion) {
        return (Value::Null, Some(bound));
    }
    let (verdict, decoded) = wire::reference_judgement(&built.types, &draft.bytes, &types);
    if scan.too_deep(expansion, decoded) {
        return (Value::Null, Some("levels"));
    }
    let line = json!({
        "kind": "decode",
        "id": id,
        "env": env_id,
        "wire": draft.wire.map(|decls| names(&decls)),
        "expected": expected,
        "mutation": if draft.mutation.is_empty() {
            "none".to_string()
        } else {
            draft.mutation.join("+")
        },
        "hex": wire::hex(&draft.bytes),
        "ref": verdict,
    });
    (line, None)
}

/// Draft cases with `draft` until one is within the generation bounds,
/// counting the redrawn ones.
fn within_bounds<T>(
    id: &str,
    target: &str,
    redraws: &mut Redraws,
    mut draft: impl FnMut() -> (T, Option<&'static str>),
) -> T {
    for _ in 0..MAX_DRAFTS {
        let (case, outside) = draft();
        match outside {
            None => return case,
            Some(bound) => {
                *redraws
                    .cases
                    .entry(format!("{target}:{bound}"))
                    .or_default() += 1
            }
        }
    }
    panic!("{id}: no draft within the generation bounds after {MAX_DRAFTS} attempts")
}

fn decode_case(
    built: &Built,
    rng: &mut Rng,
    id: String,
    env_id: &str,
    redraws: &mut Redraws,
) -> Value {
    within_bounds(&id, "decode", redraws, || {
        judge_decode(built, draft_decode(built, rng), env_id, &id)
    })
}

/// The composite nesting of a HostValue JSON document: `opt` (absent
/// included), `vec`, `record` and `variant` each count a level, as
/// `wire::Scan::levels` counts them on the wire.
fn host_levels(host: &Value) -> usize {
    let children: Vec<&Value> = match host["kind"].as_str() {
        Some("opt" | "variant") => vec![&host["value"]],
        Some("vec") => host["values"]
            .as_array()
            .map_or(Vec::new(), |v| v.iter().collect()),
        Some("record") => host["fields"].as_array().map_or(Vec::new(), |fields| {
            fields.iter().map(|f| &f["value"]).collect()
        }),
        _ => return 0,
    };
    1 + children.into_iter().map(host_levels).max().unwrap_or(0)
}

/// The reference verdict for one validate case: `validate_host_value` under
/// `Limits::default()` with `max_value_depth` aligned to the runtime's depth
/// budget (`VALIDATE_DEPTH`). Its two value budgets are verdicts, compared
/// exactly with the runtime's refusals on the same resource: `value_depth`
/// (aligned) and `value_elements` (`max_value_elements`, 1,000,000, the
/// runtime's documented `maxElements`; the runtime also charges each `rec` hop
/// and examined record key, so the two counts differ, and exact vectors pin
/// where: `validate_elements_*`). A refusal on any other budget of the
/// reference (or of the HostValue constructors) is `inconclusive`.
fn host_verdict(built: &Built, name: &str, host: &Value) -> Value {
    let limits = candid_core::Limits::default();
    let mut built_by = None;
    let value = match candid_core::HostValue::from_json_with_limits(&host.to_string(), &limits) {
        Ok(value) => value,
        Err(candid_core::HostValueJsonError::Malformed(_)) => {
            return json!({ "verdict": "reject", "class": "host_value_json" });
        }
        // A budget of the HostValue JSON decoder (above all its 64-container
        // nesting cap, a guard for serde_json's recursion, and its value
        // budgets): a policy of that ABI, not a judgement of the value. The
        // value is rebuilt through the HostValue constructors with their own
        // depth and element budgets raised (building a value is not judging
        // it), and `validate_host_value` judges it like any other, charging
        // its budgets in walk order.
        Err(_) => match host_from_constructors(host, &construction_limits()) {
            Ok(value) => {
                built_by = Some("constructors");
                value
            }
            Err(candid_core::HostValueJsonError::Malformed(_)) => {
                return json!({ "verdict": "reject", "class": "host_value_json" });
            }
            Err(_) => return json!({ "verdict": "inconclusive", "budget": "host_value_limit" }),
        },
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
    let aligned = limits.with_max_value_depth(VALIDATE_DEPTH);
    let mut verdict =
        match candid_core::validate_host_value(&built.contract, &selector, &value, &aligned) {
            Ok(()) => json!({ "verdict": "accept" }),
            Err(error) => match error.violations.first() {
                None => json!({ "verdict": "reject", "class": "unknown" }),
                Some(violation) => match &violation.resource_limit {
                    Some(info)
                        if info.resource == "value_depth" || info.resource == "value_elements" =>
                    {
                        json!({
                            "verdict": "reject",
                            "class": format!("{}/{}", violation.code, info.resource),
                        })
                    }
                    Some(info) => json!({ "verdict": "inconclusive", "budget": info.resource }),
                    None => json!({ "verdict": "reject", "class": violation.code }),
                },
            },
        };
    if let Some(by) = built_by {
        verdict["built"] = json!(by);
    }
    verdict
}

/// The limits the HostValue constructors build a value under in
/// `host_verdict`: the defaults with the value depth and element budgets out
/// of the way, so `validate_host_value` alone applies them.
fn construction_limits() -> candid_core::Limits {
    candid_core::Limits::default()
        .with_max_value_depth(usize::MAX / 2)
        .with_max_value_elements(usize::MAX / 2)
}

/// The HostValue a HostValue JSON document denotes, built through the
/// public constructors (containers) and the JSON decoder (scalars, one level
/// each), so no JSON nesting budget applies; the constructors still refuse a
/// value past `limits.max_value_depth` or `limits.max_value_elements`.
fn host_from_constructors(
    json: &Value,
    limits: &candid_core::Limits,
) -> Result<candid_core::HostValue, candid_core::HostValueJsonError> {
    use candid_core::{HostFieldValue, HostValue, HostValueJsonError};
    let malformed = |what: &str| HostValueJsonError::Malformed(format!("$: {what}"));
    match json["kind"].as_str() {
        Some("opt") => {
            let inner = match &json["value"] {
                Value::Null => None,
                inner => Some(host_from_constructors(inner, limits)?),
            };
            HostValue::opt(inner, limits)
        }
        Some("vec") => {
            let mut values = Vec::new();
            for item in json["values"]
                .as_array()
                .ok_or_else(|| malformed("values"))?
            {
                values.push(host_from_constructors(item, limits)?);
            }
            HostValue::vector(values, limits)
        }
        Some("record") => {
            let mut fields = Vec::new();
            for field in json["fields"]
                .as_array()
                .ok_or_else(|| malformed("fields"))?
            {
                let id = field["id"]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or_else(|| malformed("field id"))?;
                fields.push(HostFieldValue::new(
                    id,
                    host_from_constructors(&field["value"], limits)?,
                ));
            }
            HostValue::record(fields, limits)
        }
        Some("variant") => {
            let id = json["id"]
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| malformed("variant id"))?;
            HostValue::variant(id, host_from_constructors(&json["value"], limits)?, limits)
        }
        _ => HostValue::from_json_with_limits(&json.to_string(), limits),
    }
}

/// A validate case's value, mutated or pushed to a boundary or not, and its
/// HostValue; outside the generation bounds when it nests too deep.
fn draft_validate(
    built: &Built,
    rng: &mut Rng,
    name: &str,
    mut value: Value,
) -> ((Value, bool, Value), Option<&'static str>) {
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
    let outside = (host_levels(&host_json) + 2 > wire::GEN_LEVELS).then_some("levels");
    ((value, mutated, host_json), outside)
}

/// The longest HostValue JSON a validate line records (for review only;
/// nothing reads it back): a boundary vector's million elements would put
/// megabytes in the golden, so past this its byte length stands in for it.
const HOST_RECORDED_BYTES: usize = 65_536;

fn validate_line(
    built: &Built,
    id: String,
    env_id: &str,
    name: &str,
    (value, mutated, host_json): (Value, bool, Value),
) -> Value {
    let verdict = host_verdict(built, name, &host_json);
    let text = host_json.to_string();
    let host = if text.len() > HOST_RECORDED_BYTES {
        json!({ "elided_bytes": text.len() })
    } else {
        host_json
    };
    json!({
        "kind": "validate",
        "id": id,
        "env": env_id,
        "type": name,
        "mutated": mutated,
        "value": value,
        "ref": verdict,
        "host": host,
    })
}

fn validate_case(
    built: &Built,
    rng: &mut Rng,
    id: String,
    env_id: &str,
    redraws: &mut Redraws,
) -> Value {
    let decl = rng.below(built.env.decls);
    let name = types::decl_name(decl);
    let ty = find(&built.types, &name);
    let drafted = within_bounds(&id, "validate", redraws, || {
        let value = wire::random_value(&built.types, &ty, rng, 0)
            .and_then(|value| host::descriptor(&built.types, &ty, &value))
            .unwrap_or_else(|| host::random_scalar(rng));
        draft_validate(built, rng, &name, value)
    });
    validate_line(built, id, env_id, &name, drafted)
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
    let mut line = json!({
        "kind": "env",
        "env": env_id,
        "did": built.env.source,
        "envelope": built.envelope,
    });
    if let Some(code) = &built.compile_refused {
        line["compile_refused"] = json!(code);
    }
    line
}

fn generate_env(seed: u64, seeds: &Seeds, lines: &mut Vec<Value>, redraws: &mut Redraws) {
    let mut rng = Rng::new(seed);
    let built = build_drawn(&mut rng, redraws, types::random_env);
    let env_id = format!("e{seed}");
    lines.push(env_line(&env_id, &built));
    for index in 0..seeds.decode {
        let id = format!("d/{seed}/{index}");
        lines.push(decode_case(&built, &mut rng, id, &env_id, redraws));
    }
    for index in 0..seeds.validate {
        let id = format!("v/{seed}/{index}");
        lines.push(validate_case(&built, &mut rng, id, &env_id, redraws));
    }
    for index in 0..seeds.contract {
        let id = format!("c/{seed}/{index}");
        lines.push(contract_case(&built, &mut rng, id, &env_id));
    }
}

/// How deep a deep case's value nests: half of the time within 8 levels of
/// the generation bound (`wire::GEN_LEVELS`), otherwise anywhere below it. A
/// draft whose nesting at the expected types reaches the bound (a sibling
/// whose coercion inserts an `opt` at every level doubles it) is redrawn.
fn deep_levels(rng: &mut Rng) -> usize {
    let most = wire::GEN_LEVELS - 3;
    if rng.chance(1, 2) {
        most - rng.below(8)
    } else {
        rng.below(most + 1)
    }
}

/// One deep decode case: a value of `T0` nested close to the generation
/// bound half of the time, optionally mutated, read at `T0` or a sibling.
fn deep_decode_case(
    built: &Built,
    rng: &mut Rng,
    id: String,
    env_id: &str,
    redraws: &mut Redraws,
) -> Value {
    within_bounds(&id, "deep_decode", redraws, || {
        let wire_ty = find(&built.types, &types::decl_name(0));
        let levels = deep_levels(rng);
        let value = wire::deep_value(&built.types, &wire_ty, rng, levels)
            .or_else(|| wire::random_value(&built.types, &wire_ty, rng, 0));
        let bytes = value.and_then(|value| {
            wire::encode(&built.types, std::slice::from_ref(&wire_ty), vec![value])
        });
        let mut mutation = vec!["deep"];
        let (wire, mut bytes) = match bytes {
            Some(bytes) => (Some(vec![0]), bytes),
            None => {
                mutation.push("raw");
                (None, raw_bytes(rng))
            }
        };
        if wire.is_some() && rng.chance(1, 4) {
            mutation.push(wire::mutate_once(rng, &mut bytes));
        }
        let expected = if rng.chance(2, 3) {
            0
        } else {
            *rng.pick(family_of(&built.env, 0))
        };
        let draft = DecodeDraft {
            wire,
            expected: vec![expected],
            mutation,
            bytes,
        };
        judge_decode(built, draft, env_id, &id)
    })
}

/// One deep validate case: a value of `T0` nested close to the generation
/// bound half of the time (see `wire::GEN_LEVELS`), which crosses the
/// HostValue JSON decoder's 64-container nesting cap (the reference then
/// judges the value rebuilt through the HostValue constructors, see
/// `host_verdict`).
fn deep_validate_case(
    built: &Built,
    rng: &mut Rng,
    id: String,
    env_id: &str,
    redraws: &mut Redraws,
) -> Value {
    let name = types::decl_name(0);
    let ty = find(&built.types, &name);
    let drafted = within_bounds(&id, "deep_validate", redraws, || {
        let levels = deep_levels(rng);
        let value = wire::deep_value(&built.types, &ty, rng, levels)
            .and_then(|value| host::descriptor(&built.types, &ty, &value))
            .unwrap_or_else(|| host::random_scalar(rng));
        draft_validate(built, rng, &name, value)
    });
    validate_line(built, id, env_id, &name, drafted)
}

fn generate_deep_env(seed: u64, seeds: &Seeds, lines: &mut Vec<Value>, redraws: &mut Redraws) {
    let mut rng = Rng::new(seed ^ DEEP_SALT);
    let built = if seeds.deep_kinds {
        let kind = usize::try_from(seed - seeds.start).unwrap_or(0) % types::DEEP_KINDS;
        build_drawn(&mut rng, redraws, |rng| types::deep_env_of_kind(rng, kind))
    } else {
        build_drawn(&mut rng, redraws, types::deep_env)
    };
    let env_id = format!("x{seed}");
    lines.push(env_line(&env_id, &built));
    for index in 0..seeds.deep_decode {
        let id = format!("xd/{seed}/{index}");
        lines.push(deep_decode_case(&built, &mut rng, id, &env_id, redraws));
    }
    for index in 0..seeds.deep_validate {
        let id = format!("xv/{seed}/{index}");
        lines.push(deep_validate_case(&built, &mut rng, id, &env_id, redraws));
    }
}

/// Every environment and case of `seeds`, in order, and the redraw counts.
fn generate(seeds: &Seeds, lines: &mut Vec<Value>) -> Redraws {
    let mut redraws = Redraws::default();
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
        "deep_kinds": if seeds.deep_kinds { "cycled" } else { "drawn" },
        "bounds": {
            "levels": wire::GEN_LEVELS,
            "elements": wire::GEN_ELEMENTS,
            "length": wire::GEN_LENGTH,
            "table_entries": wire::GEN_TABLE_ENTRIES,
            "validate_max_value_depth": VALIDATE_DEPTH,
        },
        "runtime_budgets": runtime_budgets(),
        "redraws": redraws.envs,
        "case_redraws": redraws.cases,
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
    // Deep validate vectors nest past serde_json's 128-level parse limit.
    let document = deep_json(&text);
    for vector in document["vectors"].as_array().expect("a vectors array") {
        let name = vector["name"].as_str().expect("a name");
        let source = vector["did"].as_str().expect("a did source").to_string();
        let built = match vector.get("contract") {
            None => build_from_source(types::Env {
                source,
                families: Vec::new(),
                decls: 0,
            })
            .unwrap_or_else(|reason| {
                panic!("regression {name}: both sides must accept its source ({reason})")
            }),
            Some(document) => build_with_contract(&source, document)
                .unwrap_or_else(|reason| panic!("regression {name}: {reason}")),
        };
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
                validate_line(&built, id, &env_id, name, (value, true, host_json))
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

/// The structural type-table edits (`wire::mutate_table`) the committed
/// corpus must exercise, each at least once in a generated decode case.
const TABLE_MUTATIONS: &[&str] = &[
    "duplicate_field_id",
    "unsorted_field_ids",
    "duplicate_variant_id",
    "unsorted_variant_ids",
    "duplicate_method_name",
    "unsorted_method_names",
];

/// Regenerate the committed corpus and compare it with the golden. The
/// reference must judge every case in it: no reference panic, no value the
/// domain mapping cannot express, and no refusal on a budget of the
/// reference's own (`inconclusive`) — the generator is sized so none occurs.
#[test]
fn differential_corpus_matches_reference() {
    let (header, lines) = corpus();
    let failures: Vec<String> = lines
        .iter()
        .filter(|line| {
            matches!(
                line["ref"]["verdict"].as_str(),
                Some("panic" | "mapping_error" | "inconclusive")
            )
        })
        .map(|line| format!("{}: {}", line["id"], line["ref"]))
        .collect();
    assert!(
        failures.is_empty(),
        "cases the reference did not judge: {failures:#?}"
    );
    let missing: Vec<&str> = TABLE_MUTATIONS
        .iter()
        .copied()
        .filter(|name| {
            !lines.iter().any(|line| {
                line["kind"] == "decode"
                    && !line["id"].as_str().unwrap_or("").starts_with("r/")
                    && line["mutation"]
                        .as_str()
                        .unwrap_or("")
                        .split('+')
                        .any(|applied| applied == *name)
            })
        })
        .collect();
    assert!(
        missing.is_empty(),
        "structural table edits the committed corpus never applies: {missing:?}"
    );
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
/// `DIFF_DEEP_VALIDATE` (cases per deep environment), and `DIFF_DEEP_KINDS`
/// (set: deep environments cycle through the kinds, as in the committed
/// corpus, instead of drawing one).
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
        deep_kinds: std::env::var_os("DIFF_DEEP_KINDS").is_some(),
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

/// One JSON document (a corpus line, or the regression vectors), read without
/// serde_json's 128-level recursion limit: a deep case nests past it. Only
/// ever called on `on_big_stack`.
fn deep_json(line: &str) -> Value {
    let mut deserializer = serde_json::Deserializer::from_str(line);
    deserializer.disable_recursion_limit();
    let mut values = deserializer.into_iter::<Value>();
    let value = values.next().expect("a JSON document").expect("JSON");
    assert!(values.next().is_none(), "one JSON value per document");
    value
}

/// Rejudge mode: `DIFF_IN` is a corpus or campaign batch (possibly written by
/// an earlier generator), `DIFF_OUT` receives it with every environment
/// rebuilt from its `did` and every reference verdict recomputed by this
/// tree. A rebuilt envelope that differs from the recorded one is a harness
/// failure.
#[test]
#[ignore = "rejudge mode: run explicitly with DIFF_IN and DIFF_OUT"]
fn differential_rejudge() {
    let input = std::env::var("DIFF_IN").expect("DIFF_IN names the batch to rejudge");
    let out = std::env::var("DIFF_OUT").expect("DIFF_OUT names the output file");
    let text = std::fs::read_to_string(input).expect("DIFF_IN must be readable");
    let result = on_big_stack(move || {
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header: Value = deep_json(lines.next().expect("a header"));
        let mut output = Vec::new();
        let mut built: Option<Built> = None;
        for line in lines {
            let mut case: Value = deep_json(line);
            match case["kind"].as_str() {
                Some("env") => {
                    let source = case["did"].as_str().expect("did").to_string();
                    let rebuilt = if case["compile_refused"].is_string() {
                        build_with_contract(&source, &case["envelope"]["contract"])
                    } else {
                        build_from_source(types::Env {
                            source,
                            families: Vec::new(),
                            decls: 0,
                        })
                    }
                    .unwrap_or_else(|reason| panic!("{}: {reason}", case["env"]));
                    assert!(
                        rebuilt.envelope == case["envelope"],
                        "{}: the envelope rebuilt differently",
                        case["env"]
                    );
                    assert!(
                        rebuilt.compile_refused.as_deref() == case["compile_refused"].as_str(),
                        "{}: the compiler's verdict changed",
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
                    case["ref"] = wire::reference_verdict(&built.types, &bytes, &types);
                }
                Some("validate") => {
                    let built = built.as_ref().expect("an environment first");
                    let name = case["type"].as_str().expect("type").to_string();
                    let ty = find(&built.types, &name);
                    let host_json = host::host_value(&built.types, &ty, &case["value"]);
                    case["ref"] = host_verdict(built, &name, &host_json);
                    case["host"] = host_json;
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
