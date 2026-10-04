//! The `decodeArgs` target, reference half: random values at the wire types,
//! encoded by the `candid` crate, optionally mutated, and decoded back by the
//! `candid` crate at the expected types. The verdict and, on acceptance, the
//! decoded values under the documented domain mapping are what the TypeScript
//! runner compares against.

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

use candid::types::value::{IDLArgs, IDLField, IDLValue, VariantValue};
use candid::types::{Type, TypeInner};
use candid::{Int, Nat, Principal, TypeEnv};
use serde_json::{json, Value};

use super::rng::Rng;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("valid hex"))
        .collect()
}

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Run one reference call, turning a panic into `Err` (recorded as a
/// `panic` verdict) and keeping its message out of the test output.
pub fn guarded<T>(work: impl FnOnce() -> T) -> Result<T, ()> {
    QUIET.with(|quiet| quiet.set(true));
    let result = catch_unwind(AssertUnwindSafe(work));
    QUIET.with(|quiet| quiet.set(false));
    result.map_err(|_| ())
}

/// Whether a panic now is an expected, guarded one.
pub fn quiet() -> bool {
    QUIET.with(Cell::get)
}

fn trace(env: &TypeEnv, ty: &Type) -> Option<Type> {
    env.trace_type(ty).ok()
}

/// Whether a resolved type's domain admits `null`: the boxed-opt rule.
pub fn admits_null(env: &TypeEnv, ty: &Type) -> bool {
    match trace(env, ty) {
        Some(ty) => matches!(
            ty.as_ref(),
            TypeInner::Opt(_) | TypeInner::Null | TypeInner::Reserved
        ),
        None => false,
    }
}

pub fn is_blob(env: &TypeEnv, inner: &Type) -> bool {
    matches!(trace(env, inner).as_deref(), Some(TypeInner::Nat8))
}

// ---------------------------------------------------------------------------
// Value generation
// ---------------------------------------------------------------------------

const NATS: &[&str] = &[
    "0",
    "1",
    "63",
    "64",
    "127",
    "128",
    "255",
    "256",
    "16383",
    "16384",
    "4294967295",
    "4294967296",
    "9223372036854775807",
    "9223372036854775808",
    "18446744073709551615",
    "18446744073709551616",
    "340282366920938463463374607431768211455",
];

pub fn random_nat_text(rng: &mut Rng) -> String {
    if rng.chance(2, 3) {
        (*rng.pick(NATS)).to_string()
    } else {
        (rng.next_u64() >> rng.below(64)).to_string()
    }
}

pub fn random_int_text(rng: &mut Rng) -> String {
    let magnitude = random_nat_text(rng);
    if magnitude != "0" && rng.chance(1, 2) {
        format!("-{magnitude}")
    } else {
        magnitude
    }
}

const FLOAT_BITS: &[u64] = &[
    0x0000_0000_0000_0000, // 0
    0x8000_0000_0000_0000, // -0
    0x3ff8_0000_0000_0000, // 1.5
    0x7ff8_0000_0000_0000, // NaN
    0x7ff0_0000_0000_0000, // inf
    0xfff0_0000_0000_0000, // -inf
    0x0000_0000_0000_0001, // smallest subnormal
    0x7fef_ffff_ffff_ffff, // max
    0x4415_af1d_78b5_8c40, // 1e20
    0x3e7a_d7f2_9abc_af48, // 1e-7
];

pub fn random_f64(rng: &mut Rng) -> f64 {
    if rng.chance(2, 3) {
        f64::from_bits(*rng.pick(FLOAT_BITS))
    } else {
        f64::from_bits(rng.next_u64())
    }
}

pub fn random_f32(rng: &mut Rng) -> f32 {
    if rng.chance(2, 3) {
        random_f64(rng) as f32
    } else {
        f32::from_bits((rng.next_u64() & 0xffff_ffff) as u32)
    }
}

const TEXTS: &[&str] = &[
    "",
    "a",
    "héllo",
    "☃",
    "\u{10ffff}",
    "\0",
    "aaaaa-aa",
    "naïve text",
];

pub fn random_text(rng: &mut Rng) -> String {
    if rng.chance(2, 3) {
        (*rng.pick(TEXTS)).to_string()
    } else {
        (0..rng.below(8))
            .map(|_| char::from(b'a' + (rng.below(26) as u8)))
            .collect()
    }
}

pub fn random_principal(rng: &mut Rng) -> Principal {
    match rng.below(4) {
        0 => Principal::management_canister(),
        1 => Principal::anonymous(),
        _ => {
            let length = rng.below(30);
            let bytes: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
            Principal::from_slice(&bytes)
        }
    }
}

const METHOD_NAMES: &[&str] = &["f", "go", "", "méthode", "get_blocks"];

/// A random value of `ty`, or `None` when the walk finds no inhabitant within
/// its depth and node budgets (an `empty`, a record that only recurses, a
/// type whose every inhabitant is huge).
pub fn random_value(env: &TypeEnv, ty: &Type, rng: &mut Rng, depth: usize) -> Option<IDLValue> {
    // The node budget bounds the walk: without it a type such as
    // `variant { a : record { T; T }; b : nat }` costs 2^depth.
    let mut budget = 4096usize;
    value_within(env, ty, rng, depth, &mut budget)
}

fn value_within(
    env: &TypeEnv,
    ty: &Type,
    rng: &mut Rng,
    depth: usize,
    budget: &mut usize,
) -> Option<IDLValue> {
    if depth > 24 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    let shallow = depth > 6;
    let ty = trace(env, ty)?;
    Some(match ty.as_ref() {
        TypeInner::Null => IDLValue::Null,
        TypeInner::Bool => IDLValue::Bool(rng.chance(1, 2)),
        TypeInner::Nat => IDLValue::Nat(random_nat_text(rng).parse::<Nat>().ok()?),
        TypeInner::Int => IDLValue::Int(random_int_text(rng).parse::<Int>().ok()?),
        TypeInner::Nat8 => IDLValue::Nat8(rng.byte()),
        TypeInner::Nat16 => IDLValue::Nat16(rng.next_u64() as u16 >> rng.below(16)),
        TypeInner::Nat32 => IDLValue::Nat32(rng.next_u64() as u32 >> rng.below(32)),
        TypeInner::Nat64 => IDLValue::Nat64(rng.next_u64() >> rng.below(64)),
        TypeInner::Int8 => IDLValue::Int8(rng.byte() as i8),
        TypeInner::Int16 => IDLValue::Int16((rng.next_u64() as i16) >> rng.below(16)),
        TypeInner::Int32 => IDLValue::Int32((rng.next_u64() as i32) >> rng.below(32)),
        TypeInner::Int64 => IDLValue::Int64((rng.next_u64() as i64) >> rng.below(64)),
        TypeInner::Float32 => IDLValue::Float32(random_f32(rng)),
        TypeInner::Float64 => IDLValue::Float64(random_f64(rng)),
        TypeInner::Text => IDLValue::Text(random_text(rng)),
        TypeInner::Reserved => IDLValue::Reserved,
        TypeInner::Principal => IDLValue::Principal(random_principal(rng)),
        TypeInner::Opt(inner) => {
            if shallow || rng.chance(1, 3) {
                IDLValue::None
            } else {
                match value_within(env, inner, rng, depth + 1, budget) {
                    Some(value) => IDLValue::Opt(Box::new(value)),
                    None => IDLValue::None,
                }
            }
        }
        TypeInner::Vec(inner) => {
            let length = if shallow { 0 } else { rng.below(4) };
            let mut items = Vec::new();
            for _ in 0..length {
                items.push(value_within(env, inner, rng, depth + 1, budget)?);
            }
            IDLValue::Vec(items)
        }
        TypeInner::Record(fields) => {
            let mut values = Vec::new();
            for field in fields {
                values.push(IDLField {
                    id: (*field.id).clone(),
                    val: value_within(env, &field.ty, rng, depth + 1, budget)?,
                });
            }
            IDLValue::Record(values)
        }
        TypeInner::Variant(fields) => {
            if fields.is_empty() {
                return None;
            }
            // Try arms from a random start; an uninhabited arm is skipped.
            let start = rng.below(fields.len());
            for offset in 0..fields.len() {
                let index = (start + offset) % fields.len();
                let field = &fields[index];
                if let Some(value) = value_within(env, &field.ty, rng, depth + 1, budget) {
                    return Some(IDLValue::Variant(VariantValue(
                        Box::new(IDLField {
                            id: (*field.id).clone(),
                            val: value,
                        }),
                        index as u64,
                    )));
                }
            }
            return None;
        }
        TypeInner::Func(_) => {
            IDLValue::Func(random_principal(rng), (*rng.pick(METHOD_NAMES)).to_string())
        }
        TypeInner::Service(_) => IDLValue::Service(random_principal(rng)),
        _ => return None,
    })
}

/// Reference encoding of `values` at `types`; `None` if the reference
/// refuses its own value (it should not, but the case is then skipped).
pub fn encode(env: &TypeEnv, types: &[Type], values: Vec<IDLValue>) -> Option<Vec<u8>> {
    let args = IDLArgs { args: values };
    guarded(|| args.to_bytes_with_types(env, types).ok())
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// Byte mutation
// ---------------------------------------------------------------------------

/// One random edit; returns its name. Position 0..4 (the magic) is fair game
/// for the generic edits, never for the LEB128 one.
pub fn mutate_once(rng: &mut Rng, bytes: &mut Vec<u8>) -> &'static str {
    if bytes.is_empty() {
        bytes.push(rng.byte());
        return "append";
    }
    match rng.weighted(&[3, 3, 2, 2, 2, 2, 4, 1]) {
        0 => {
            let at = rng.below(bytes.len());
            bytes[at] = rng.byte();
            "set_byte"
        }
        1 => {
            let at = rng.below(bytes.len());
            bytes[at] ^= 1 << rng.below(8);
            "flip_bit"
        }
        2 => {
            let at = rng.below(bytes.len());
            bytes.truncate(at);
            "truncate"
        }
        3 => {
            for _ in 0..1 + rng.below(3) {
                bytes.push(rng.byte());
            }
            "append"
        }
        4 => {
            let at = rng.below(bytes.len());
            bytes.remove(at);
            "delete_byte"
        }
        5 => {
            let at = rng.below(bytes.len() + 1);
            bytes.insert(at, rng.byte());
            "insert_byte"
        }
        6 => {
            // Rewrite a one-byte LEB128 group as its two-byte non-minimal
            // form: `b` becomes `b|0x80, 0x00` (or `0x7f`, the signed
            // extension of a negative SLEB128 group). Every positive
            // one-byte group in the message is a candidate, so the edit lands
            // on lengths, indices, type opcodes and numbers alike.
            let candidates: Vec<usize> = (4..bytes.len()).filter(|&i| bytes[i] < 0x80).collect();
            if candidates.is_empty() {
                bytes.push(0);
                return "append";
            }
            let at = *rng.pick(&candidates);
            let byte = bytes[at];
            let extension = if byte & 0x40 != 0 && rng.chance(1, 2) {
                0x7f
            } else {
                0x00
            };
            bytes[at] = byte | 0x80;
            bytes.insert(at + 1, extension);
            "overlong_leb128"
        }
        _ => {
            let at = rng.below(bytes.len());
            let byte = bytes[at];
            bytes.insert(at, byte);
            "duplicate_byte"
        }
    }
}

// ---------------------------------------------------------------------------
// Reference verdict and the domain mapping
// ---------------------------------------------------------------------------

/// Whether any value in the tree is a func reference with an empty method
/// name: the runtime refuses one by decision (round-trip symmetry with
/// `validate` and `encode`, see `ts/codec.ts`), the reference accepts it.
fn has_empty_method(value: &IDLValue) -> bool {
    match value {
        IDLValue::Func(_, method) => method.is_empty(),
        IDLValue::Opt(inner) => has_empty_method(inner),
        IDLValue::Vec(items) => items.iter().any(has_empty_method),
        IDLValue::Record(fields) => fields.iter().any(|field| has_empty_method(&field.val)),
        IDLValue::Variant(variant) => has_empty_method(&variant.0.val),
        _ => false,
    }
}

/// The reference decoder's configuration. Unconfigured, the `candid` crate
/// bounds no work (its documentation asks canister code to set a quota), and
/// a vector of a zero-sized type with a forged length of 2^40 then runs for
/// hours. The decoding quota (which charges skipped values 50x) is set well
/// above anything a generated message needs; the runtime's own element
/// budget (`maxElements`, 1,000,000) refuses those messages first.
pub const DECODING_QUOTA: usize = 2_000_000;

fn config() -> candid::DecoderConfig {
    let mut config = candid::DecoderConfig::new();
    config.set_decoding_quota(DECODING_QUOTA);
    config
}

/// Whether a reference error is its quota running out. The `candid` crate
/// reports it only as message text (`… cost exceeds the limit`), so this one
/// class is read from the reference's own wording; it is never compared with
/// anything the runtime says.
fn quota_exhausted(error: &candid::Error) -> bool {
    format!("{error:?}").contains("cost exceeds the limit")
}

/// The reference verdict for one decode case, as the golden records it.
///
/// A rejection carries a class derived from the reference's *behaviour*, not
/// its message text: `header` when the reference cannot even parse the
/// header and type table (`IDLDeserialize::new` fails), `malformed` when the
/// header parses but the message does not decode at its own wire types
/// (`IDLArgs::from_bytes` fails), and `coercion` when the message is
/// well-formed and only the expected types refuse it; `limit` when the
/// decoding quota ran out first (see `DECODING_QUOTA`).
///
/// `flags` records input properties the verdict mapping needs and only the
/// reference can see: `empty_method` when a value the reference decoded (at
/// the expected types, or at the wire types for a `coercion` rejection) is a
/// func reference with an empty method name.
pub fn reference_verdict(env: &TypeEnv, bytes: &[u8], expected: &[Type]) -> Value {
    let typed =
        guarded(|| IDLArgs::from_bytes_with_types_with_config(bytes, env, expected, &config()));
    match typed {
        Err(()) => json!({ "verdict": "panic" }),
        Ok(Ok(decoded)) => {
            let mut values = Vec::new();
            for (value, ty) in decoded.args.iter().zip(expected) {
                match domain(env, ty, value) {
                    Ok(mapped) => values.push(mapped),
                    Err(message) => {
                        return json!({ "verdict": "mapping_error", "detail": message });
                    }
                }
            }
            let mut verdict = json!({ "verdict": "accept", "values": values });
            if decoded.args.iter().any(has_empty_method) {
                verdict["flags"] = json!(["empty_method"]);
            }
            verdict
        }
        Ok(Err(error)) => {
            if quota_exhausted(&error) {
                return json!({ "verdict": "reject", "class": "limit" });
            }
            let header =
                guarded(|| candid::de::IDLDeserialize::new_with_config(bytes, &config()).is_ok())
                    .unwrap_or(false);
            if !header {
                return json!({ "verdict": "reject", "class": "header" });
            }
            match guarded(|| IDLArgs::from_bytes_with_config(bytes, &config())) {
                Ok(Ok(untyped)) => {
                    let mut verdict = json!({ "verdict": "reject", "class": "coercion" });
                    if untyped.args.iter().any(has_empty_method) {
                        verdict["flags"] = json!(["empty_method"]);
                    }
                    verdict
                }
                Ok(Err(error)) if quota_exhausted(&error) => {
                    json!({ "verdict": "reject", "class": "limit" })
                }
                _ => json!({ "verdict": "reject", "class": "malformed" }),
            }
        }
    }
}

fn number_json(value: f64) -> Value {
    if value.is_nan() {
        json!({ "$num": "nan" })
    } else {
        json!({ "$num": format!("{:016x}", value.to_bits()) })
    }
}

fn label_key(label: &candid::types::Label) -> String {
    use candid::types::Label;
    match label {
        Label::Named(name) => name.clone(),
        Label::Id(id) | Label::Unnamed(id) => format!("_{id}_"),
    }
}

/// The TypeScript domain value a reference-decoded value denotes, as JSON:
///
/// - `nat`, `int`, `nat64`, `int64` (the runtime's `bigint`) → `{"$int": decimal}`;
/// - every other number (`nat8`…`int32`, `float32`, `float64`: the runtime's
///   `number`) → `{"$num": the 16 hex digits of its IEEE-754 binary64 bits}`,
///   or `{"$num": "nan"}` for any NaN — exact, so `-0`, subnormals and
///   `float32` widening are all compared bit for bit;
/// - `blob` and every `vec nat8` → `{"$blob": hex}`;
/// - text → the string; principal and service → their canonical text;
///   func → `{"principal", "method"}`; `null`, `reserved` and an absent `opt`
///   → `null`;
/// - a present `opt` → its value, boxed as `{"some": …}` exactly when its
///   inner type admits `null`;
/// - a record → an object keyed by label (`_id_` when unnamed), or an array
///   when its ids are exactly `0..n`; a variant → `{"tag", "value"}`, or a bare
///   `{"tag"}` when the arm's type is `null`.
pub fn domain(env: &TypeEnv, ty: &Type, value: &IDLValue) -> Result<Value, String> {
    let ty = trace(env, ty).ok_or("unresolvable type")?;
    Ok(match (ty.as_ref(), value) {
        (TypeInner::Reserved, _) | (_, IDLValue::Null | IDLValue::None | IDLValue::Reserved) => {
            Value::Null
        }
        (_, IDLValue::Bool(b)) => json!(b),
        (_, IDLValue::Text(text)) => json!(text),
        (_, IDLValue::Nat(n)) => json!({ "$int": n.0.to_string() }),
        (_, IDLValue::Int(n)) => json!({ "$int": n.0.to_string() }),
        (_, IDLValue::Nat64(n)) => json!({ "$int": n.to_string() }),
        (_, IDLValue::Int64(n)) => json!({ "$int": n.to_string() }),
        (_, IDLValue::Nat8(n)) => number_json(f64::from(*n)),
        (_, IDLValue::Nat16(n)) => number_json(f64::from(*n)),
        (_, IDLValue::Nat32(n)) => number_json(f64::from(*n)),
        (_, IDLValue::Int8(n)) => number_json(f64::from(*n)),
        (_, IDLValue::Int16(n)) => number_json(f64::from(*n)),
        (_, IDLValue::Int32(n)) => number_json(f64::from(*n)),
        (_, IDLValue::Float32(f)) => number_json(f64::from(*f)),
        (_, IDLValue::Float64(f)) => number_json(*f),
        (_, IDLValue::Principal(p) | IDLValue::Service(p)) => json!(p.to_text()),
        (_, IDLValue::Func(p, method)) => json!({ "principal": p.to_text(), "method": method }),
        (_, IDLValue::Blob(bytes)) => json!({ "$blob": hex(bytes) }),
        (TypeInner::Opt(inner), IDLValue::Opt(v)) => {
            let present = domain(env, inner, v)?;
            if admits_null(env, inner) {
                json!({ "some": present })
            } else {
                present
            }
        }
        (TypeInner::Vec(inner), IDLValue::Vec(items)) => {
            if is_blob(env, inner) {
                let mut bytes = Vec::new();
                for item in items {
                    match item {
                        IDLValue::Nat8(byte) => bytes.push(*byte),
                        other => return Err(format!("a vec nat8 element decoded as {other:?}")),
                    }
                }
                return Ok(json!({ "$blob": hex(&bytes) }));
            }
            let mut out = Vec::new();
            for item in items {
                out.push(domain(env, inner, item)?);
            }
            Value::Array(out)
        }
        (TypeInner::Record(fields), IDLValue::Record(values)) => {
            let tuple = !fields.is_empty()
                && fields
                    .iter()
                    .enumerate()
                    .all(|(index, field)| field.id.get_id() == index as u32);
            let mut array = Vec::new();
            let mut object = serde_json::Map::new();
            for field in values {
                let typed = fields
                    .iter()
                    .find(|candidate| candidate.id.get_id() == field.id.get_id())
                    .ok_or("a decoded field is not a field of its type")?;
                let mapped = domain(env, &typed.ty, &field.val)?;
                if tuple {
                    array.push(mapped);
                } else {
                    object.insert(label_key(&typed.id), mapped);
                }
            }
            if tuple {
                Value::Array(array)
            } else {
                Value::Object(object)
            }
        }
        (TypeInner::Variant(fields), IDLValue::Variant(variant)) => {
            let arm = &variant.0;
            let typed = fields
                .iter()
                .find(|candidate| candidate.id.get_id() == arm.id.get_id())
                .ok_or("a decoded arm is not an arm of its type")?;
            let tag = label_key(&typed.id);
            match trace(env, &typed.ty).as_deref() {
                Some(TypeInner::Null) => json!({ "tag": tag }),
                _ => json!({ "tag": tag, "value": domain(env, &typed.ty, &arm.val)? }),
            }
        }
        (other, value) => return Err(format!("no domain mapping for {value:?} at {other:?}")),
    })
}
