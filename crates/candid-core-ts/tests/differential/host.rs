//! The `validate` target, reference half: a JavaScript domain value (written
//! as a small tagged descriptor the runner rebuilds exactly), converted to a
//! HostValue under the documented mapping, and judged by candid-core's
//! `validate_host_value` against the same Contract type the TypeScript side
//! builds its schema from.
//!
//! # Descriptors
//!
//! `["n"]` null · `["b", bool]` · `["i", decimal]` bigint ·
//! `["f", 16 hex digits of the binary64 bits]` number · `["s", text]` string ·
//! `["y", hex]` Uint8Array · `["a", [items]]` array ·
//! `["o", [[key, value], …]]` plain object (own enumerable data properties, in
//! that order).
//!
//! # The mapping (descriptor → HostValue), directed by the expected type
//!
//! The conversion picks the HostValue kind the runtime's domain type denotes
//! at the expected type and never decides validity itself: a shape that is not
//! that domain type converts *blind* (by shape alone: null → `null`, boolean
//! → `bool`, bigint → `int`, number → `float64`, string → `text`, Uint8Array
//! → `vec` of `nat8`, array → `vec`, object → `record` keyed by label id),
//! which the reference then refuses as a kind mismatch.
//!
//! - `reserved`: any value → `reserved` (the runtime's `reserved` accepts
//!   anything).
//! - `opt T`: null → absent; when `T` admits null, exactly `{some: v}` →
//!   present `v`, anything else blind; otherwise any other value → present.
//! - `nat`/`int`/`nat64`/`int64`: bigint → that kind, with the canonical
//!   decimal JavaScript's `BigInt` would hold (`-0` is `0`).
//! - `nat8`…`int32`: an integral number in range → that kind; otherwise blind.
//! - `float64`: number → its bits; `float32`: number → the bits of its
//!   round-to-nearest `f32` (the runtime's `float32` accepts any number;
//!   representability is the codec's concern).
//! - `text`: string; `principal`: string → `principal` (canonical text is the
//!   reference's check); `service`: string → `service`; `func`: an object with
//!   exactly `principal` and `method` strings → `func`.
//! - `vec nat8` (blob): Uint8Array → `vec` of `nat8`; `vec T`: array → `vec`.
//! - record: tuple-shaped types take arrays (index = id), other records take
//!   objects keyed as the runtime keys them (a named field by its name, a
//!   numbered one as `_N_`); a key that is no field of the type becomes a
//!   field with an id the type does not use, its value converted blind.
//! - variant: an object whose keys are `tag` (a string) and optionally `value`
//!   → `variant` with the arm's id (an unused id when the tag is no arm), the
//!   payload converted at the arm (blind if the arm is unknown), or `null` when
//!   `value` is absent.

use candid::types::value::IDLValue;
use candid::types::{Type, TypeInner};
use candid::TypeEnv;
use serde_json::{json, Value};

use super::rng::Rng;
use super::wire::{
    admits_null, hex, is_blob, random_f64, random_int_text, random_nat_text, random_principal,
    random_text,
};

fn trace(env: &TypeEnv, ty: &Type) -> Option<Type> {
    env.trace_type(ty).ok()
}

pub fn label_id(key: &str) -> u32 {
    if let Some(digits) = key
        .strip_prefix('_')
        .and_then(|rest| rest.strip_suffix('_'))
    {
        if !digits.is_empty()
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'))
        {
            if let Ok(id) = digits.parse::<u32>() {
                return id;
            }
        }
    }
    candid::idl_hash(key)
}

fn label_key(label: &candid::types::Label) -> String {
    use candid::types::Label;
    match label {
        Label::Named(name) => name.clone(),
        Label::Id(id) | Label::Unnamed(id) => format!("_{id}_"),
    }
}

fn num(value: f64) -> Value {
    json!(["f", format!("{:016x}", value.to_bits())])
}

fn big(text: String) -> Value {
    json!(["i", text])
}

/// The domain descriptor of a conforming value (the same mapping the decode
/// target uses, in descriptor form).
pub fn descriptor(env: &TypeEnv, ty: &Type, value: &IDLValue) -> Option<Value> {
    let ty = trace(env, ty)?;
    Some(match (ty.as_ref(), value) {
        (TypeInner::Reserved, _) | (_, IDLValue::Null | IDLValue::None | IDLValue::Reserved) => {
            json!(["n"])
        }
        (_, IDLValue::Bool(b)) => json!(["b", b]),
        (_, IDLValue::Text(text)) => json!(["s", text]),
        (_, IDLValue::Nat(n)) => big(n.0.to_string()),
        (_, IDLValue::Int(n)) => big(n.0.to_string()),
        (_, IDLValue::Nat64(n)) => big(n.to_string()),
        (_, IDLValue::Int64(n)) => big(n.to_string()),
        (_, IDLValue::Nat8(n)) => num(f64::from(*n)),
        (_, IDLValue::Nat16(n)) => num(f64::from(*n)),
        (_, IDLValue::Nat32(n)) => num(f64::from(*n)),
        (_, IDLValue::Int8(n)) => num(f64::from(*n)),
        (_, IDLValue::Int16(n)) => num(f64::from(*n)),
        (_, IDLValue::Int32(n)) => num(f64::from(*n)),
        (_, IDLValue::Float32(f)) => num(f64::from(*f)),
        (_, IDLValue::Float64(f)) => num(*f),
        (_, IDLValue::Principal(p) | IDLValue::Service(p)) => json!(["s", p.to_text()]),
        (_, IDLValue::Func(p, method)) => json!([
            "o",
            [["principal", ["s", p.to_text()]], ["method", ["s", method]]]
        ]),
        (TypeInner::Opt(inner), IDLValue::Opt(v)) => {
            let present = descriptor(env, inner, v)?;
            if admits_null(env, inner) {
                json!(["o", [["some", present]]])
            } else {
                present
            }
        }
        (TypeInner::Vec(inner), IDLValue::Vec(items)) => {
            if is_blob(env, inner) {
                let bytes: Option<Vec<u8>> = items
                    .iter()
                    .map(|item| match item {
                        IDLValue::Nat8(byte) => Some(*byte),
                        _ => None,
                    })
                    .collect();
                json!(["y", hex(&bytes?)])
            } else {
                let items: Option<Vec<Value>> = items
                    .iter()
                    .map(|item| descriptor(env, inner, item))
                    .collect();
                json!(["a", items?])
            }
        }
        (TypeInner::Record(fields), IDLValue::Record(values)) => {
            let tuple = !fields.is_empty()
                && fields
                    .iter()
                    .enumerate()
                    .all(|(index, field)| field.id.get_id() == index as u32);
            let mut mapped = Vec::new();
            for (field, value) in fields.iter().zip(values) {
                let inner = descriptor(env, &field.ty, &value.val)?;
                if tuple {
                    mapped.push(inner);
                } else {
                    mapped.push(json!([label_key(&field.id), inner]));
                }
            }
            if tuple {
                json!(["a", mapped])
            } else {
                json!(["o", mapped])
            }
        }
        (TypeInner::Variant(fields), IDLValue::Variant(variant)) => {
            let arm = &variant.0;
            let typed = fields
                .iter()
                .find(|candidate| candidate.id.get_id() == arm.id.get_id())?;
            let tag = json!(["tag", ["s", label_key(&typed.id)]]);
            match trace(env, &typed.ty).as_deref() {
                Some(TypeInner::Null) => json!(["o", [tag]]),
                _ => json!(["o", [tag, ["value", descriptor(env, &typed.ty, &arm.val)?]]]),
            }
        }
        _ => return None,
    })
}

pub fn random_scalar(rng: &mut Rng) -> Value {
    match rng.below(9) {
        0 => json!(["n"]),
        1 => json!(["b", rng.chance(1, 2)]),
        2 => big(random_int_text(rng)),
        3 => num(random_f64(rng)),
        4 => num((rng.below(600) as f64) - 300.0),
        5 => json!(["s", random_text(rng)]),
        6 => json!(["s", principal_text(rng)]),
        7 => json!(["y", hex(&[rng.byte(), rng.byte()])]),
        _ => json!(["o", []]),
    }
}

/// Canonical principal text, or a near miss of one.
fn principal_text(rng: &mut Rng) -> String {
    let text = random_principal(rng).to_text();
    match rng.below(6) {
        0 => text.to_uppercase(),
        1 => text.replace('-', ""),
        2 if text.len() > 2 => text[..text.len() - 1].to_string(),
        3 => format!("{text}-a"),
        _ => text,
    }
}

/// Paths (descriptor index chains) to every scalar leaf of a conforming
/// descriptor, each with the type it was generated at.
fn scalar_leaves(
    env: &TypeEnv,
    ty: &Type,
    desc: &Value,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, Type)>,
) {
    let Some(resolved) = trace(env, ty) else {
        return;
    };
    let tag = desc.get(0).and_then(Value::as_str).unwrap_or("");
    match (resolved.as_ref(), tag) {
        (TypeInner::Opt(inner), _) if tag != "n" => {
            if admits_null(env, inner) {
                if tag == "o" {
                    path.extend([1, 0, 1]);
                    scalar_leaves(env, inner, &desc[1][0][1], path, out);
                    path.truncate(path.len() - 3);
                }
            } else {
                scalar_leaves(env, inner, desc, path, out);
            }
        }
        (TypeInner::Vec(inner), "a") => {
            for (index, item) in desc[1].as_array().into_iter().flatten().enumerate() {
                path.extend([1, index]);
                scalar_leaves(env, inner, item, path, out);
                path.truncate(path.len() - 2);
            }
        }
        (TypeInner::Record(fields), "a" | "o") => {
            for (index, item) in desc[1].as_array().into_iter().flatten().enumerate() {
                let (key_id, inner, prefix) = if tag == "a" {
                    (index as u32, item, vec![1, index])
                } else {
                    (
                        label_id(item[0].as_str().unwrap_or("")),
                        &item[1],
                        vec![1, index, 1],
                    )
                };
                if let Some(field) = fields.iter().find(|field| field.id.get_id() == key_id) {
                    let depth = prefix.len();
                    path.extend(prefix);
                    scalar_leaves(env, &field.ty, inner, path, out);
                    path.truncate(path.len() - depth);
                }
            }
        }
        (TypeInner::Variant(arms), "o") => {
            let list = desc[1].as_array().cloned().unwrap_or_default();
            let tag_id = list
                .iter()
                .find(|entry| entry[0] == "tag")
                .and_then(|entry| entry[1][1].as_str().map(label_id));
            for (index, entry) in list.iter().enumerate() {
                if entry[0] != "value" {
                    continue;
                }
                if let Some(arm) = arms.iter().find(|arm| Some(arm.id.get_id()) == tag_id) {
                    path.extend([1, index, 1]);
                    scalar_leaves(env, &arm.ty, &entry[1], path, out);
                    path.truncate(path.len() - 3);
                }
            }
        }
        (
            TypeInner::Record(_) | TypeInner::Variant(_) | TypeInner::Vec(_) | TypeInner::Opt(_),
            _,
        ) => {}
        _ => out.push((path.clone(), resolved.clone())),
    }
}

/// A value at or just past the edge of `ty`'s domain.
fn boundary_for(rng: &mut Rng, ty: &Type) -> Value {
    let ints = |rng: &mut Rng, values: &[&str]| big((*rng.pick(values)).to_string());
    let nums = |rng: &mut Rng, values: &[f64]| num(*rng.pick(values));
    match ty.as_ref() {
        TypeInner::Nat => ints(rng, &["-1", "0", "340282366920938463463374607431768211456"]),
        TypeInner::Int => ints(rng, &["-340282366920938463463374607431768211457", "0"]),
        TypeInner::Nat64 => ints(
            rng,
            &["-1", "0", "18446744073709551615", "18446744073709551616"],
        ),
        TypeInner::Int64 => ints(
            rng,
            &[
                "-9223372036854775809",
                "-9223372036854775808",
                "9223372036854775807",
                "9223372036854775808",
            ],
        ),
        TypeInner::Nat8 => nums(rng, &[-1.0, 0.0, 255.0, 256.0, 1.5, -0.0]),
        TypeInner::Nat16 => nums(rng, &[-1.0, 65535.0, 65536.0, 0.5]),
        TypeInner::Nat32 => nums(rng, &[-1.0, 4_294_967_295.0, 4_294_967_296.0]),
        TypeInner::Int8 => nums(rng, &[-129.0, -128.0, 127.0, 128.0]),
        TypeInner::Int16 => nums(rng, &[-32769.0, -32768.0, 32767.0, 32768.0]),
        TypeInner::Int32 => nums(
            rng,
            &[
                -2_147_483_649.0,
                -2_147_483_648.0,
                2_147_483_647.0,
                2_147_483_648.0,
            ],
        ),
        TypeInner::Float32 | TypeInner::Float64 => nums(
            rng,
            &[
                f64::NAN,
                f64::INFINITY,
                -0.0,
                1e300,
                f64::MIN_POSITIVE / 4.0,
            ],
        ),
        TypeInner::Principal => json!(["s", principal_text(rng)]),
        TypeInner::Text => json!(["s", *rng.pick(&["", "\u{0}", "\u{10ffff}"])]),
        TypeInner::Bool => nums(rng, &[0.0, 1.0]),
        TypeInner::Null => json!(["o", []]),
        _ => random_scalar(rng),
    }
}

/// Replace one scalar leaf of a conforming descriptor with a boundary value
/// for its type: in range, one past the range, the wrong numeric kind.
pub fn boundary(rng: &mut Rng, env: &TypeEnv, ty: &Type, value: &mut Value) {
    let mut leaves = Vec::new();
    scalar_leaves(env, ty, value, &mut Vec::new(), &mut leaves);
    if leaves.is_empty() {
        return;
    }
    let (path, leaf_ty) = rng.pick(&leaves).clone();
    let replacement = boundary_for(rng, &leaf_ty);
    let mut node = value;
    for index in path {
        node = &mut node[index];
    }
    *node = replacement;
}

/// One random edit somewhere in a descriptor tree.
pub fn mutate(rng: &mut Rng, value: &mut Value) {
    // Walk down to a random node.
    let mut node = value;
    loop {
        let descend = match node.get(0).and_then(Value::as_str) {
            Some("a") => node[1].as_array().map_or(0, Vec::len),
            Some("o") => node[1].as_array().map_or(0, Vec::len),
            _ => 0,
        };
        if descend == 0 || rng.chance(1, 3) {
            break;
        }
        let index = rng.below(descend);
        node = if node[0] == "a" {
            &mut node[1][index]
        } else {
            &mut node[1][index][1]
        };
    }
    let kind = node
        .get(0)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match (kind.as_str(), rng.below(4)) {
        ("i", 0) => {
            // bigint → number of the same magnitude where representable.
            let text = node[1].as_str().unwrap_or("0").to_string();
            *node = num(text.parse::<f64>().unwrap_or(0.0));
        }
        ("i", 1) => *node = big(random_nat_text(rng)),
        ("i", 2) => *node = big(format!("-{}", random_nat_text(rng)).replace("-0", "0")),
        ("f", 0) => {
            let bits = u64::from_str_radix(node[1].as_str().unwrap_or("0"), 16).unwrap_or(0);
            let value = f64::from_bits(bits);
            *node = if value.is_finite() && value.fract() == 0.0 {
                big(format!("{value:.0}"))
            } else {
                big("0".to_string())
            };
        }
        ("f", 1) => {
            let bits = u64::from_str_radix(node[1].as_str().unwrap_or("0"), 16).unwrap_or(0);
            *node = num(f64::from_bits(bits) + 0.5);
        }
        ("s", 0 | 1) => *node = json!(["s", principal_text(rng)]),
        ("o", 0) => {
            if let Some(entries) = node[1].as_array_mut() {
                if !entries.is_empty() {
                    let index = rng.below(entries.len());
                    entries.remove(index);
                }
            }
        }
        ("o", 1) => {
            let key = *rng.pick(&["a", "value", "some", "tag", "_0_", "extra", "_97_"]);
            let inner = random_scalar(rng);
            if let Some(entries) = node[1].as_array_mut() {
                // Never a duplicate key: a JavaScript object cannot hold one.
                if entries.iter().all(|entry| entry[0] != key) {
                    entries.push(json!([key, inner]));
                }
            }
        }
        ("o", 2) => {
            // Unwrap a boxed or single-key object.
            if let Some(entries) = node[1].as_array() {
                if entries.len() == 1 {
                    *node = entries[0][1].clone();
                }
            }
        }
        ("a", 0) => {
            if let Some(items) = node[1].as_array_mut() {
                if !items.is_empty() {
                    items.pop();
                }
            }
        }
        ("a", 1) => {
            if let Some(items) = node[1].as_array_mut() {
                if let Some(first) = items.first().cloned() {
                    items.push(first);
                }
            }
        }
        (_, 3) => *node = json!(["o", [["some", node.clone()]]]),
        _ => *node = random_scalar(rng),
    }
}

/// A bigint descriptor's decimal as JavaScript's `BigInt` reads it: one
/// canonical spelling (no leading zeros, no `-0`), which is the only one the
/// HostValue ABI accepts.
fn canonical_decimal(desc: &Value) -> Value {
    let text = desc[1].as_str().unwrap_or("0");
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        json!("0")
    } else if negative {
        json!(format!("-{digits}"))
    } else {
        json!(digits)
    }
}

fn host_blind(desc: &Value) -> Value {
    match desc.get(0).and_then(Value::as_str) {
        Some("b") => json!({ "kind": "bool", "value": desc[1] }),
        Some("i") => json!({ "kind": "int", "value": canonical_decimal(desc) }),
        Some("f") => json!({ "kind": "float64", "bits": desc[1] }),
        Some("s") => json!({ "kind": "text", "value": desc[1] }),
        Some("y") => {
            let bytes = super::wire::unhex(desc[1].as_str().unwrap_or(""));
            let values: Vec<Value> = bytes
                .iter()
                .map(|byte| json!({ "kind": "nat8", "value": byte }))
                .collect();
            json!({ "kind": "vec", "values": values })
        }
        Some("a") => {
            let values: Vec<Value> = desc[1]
                .as_array()
                .map(|items| items.iter().map(host_blind).collect())
                .unwrap_or_default();
            json!({ "kind": "vec", "values": values })
        }
        Some("o") => {
            let fields: Vec<Value> = desc[1]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .map(|entry| {
                            json!({
                                "id": label_id(entry[0].as_str().unwrap_or("")),
                                "value": host_blind(&entry[1]),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({ "kind": "record", "fields": fields })
        }
        _ => json!({ "kind": "null" }),
    }
}

fn entries(desc: &Value) -> Option<&Vec<Value>> {
    if desc.get(0).and_then(Value::as_str) == Some("o") {
        desc[1].as_array()
    } else {
        None
    }
}

fn entry<'a>(entries: &'a [Value], key: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|entry| entry[0].as_str() == Some(key))
        .map(|entry| &entry[1])
}

fn integral_in(desc: &Value, min: f64, max: f64) -> Option<i64> {
    if desc.get(0).and_then(Value::as_str) != Some("f") {
        return None;
    }
    let bits = u64::from_str_radix(desc[1].as_str()?, 16).ok()?;
    let value = f64::from_bits(bits);
    if value.is_finite() && value.fract() == 0.0 && value >= min && value <= max {
        Some(value as i64)
    } else {
        None
    }
}

/// The id of the field or arm whose domain key is `key`: its source name
/// when named, `_N_` when numbered. Keys are matched against the type's own
/// labels, never hashed: `""` hashes to 0, the id of a field the runtime
/// keys `_0_`, and two different keys must not denote one field.
fn key_id(fields: &[candid::types::Field], key: &str) -> Option<u32> {
    fields
        .iter()
        .find(|field| label_key(&field.id) == key)
        .map(|field| field.id.get_id())
}

/// Ids no field of the type uses, for keys that denote no field (so the
/// reference sees a field set or an arm the type does not have).
struct FreshIds {
    used: Vec<u32>,
    next: u32,
}

impl FreshIds {
    fn new(used: impl Iterator<Item = u32>) -> Self {
        FreshIds {
            used: used.collect(),
            next: u32::MAX,
        }
    }

    fn next(&mut self) -> u32 {
        while self.used.contains(&self.next) {
            self.next -= 1;
        }
        let id = self.next;
        self.used.push(id);
        id
    }
}

/// HostValue JSON for `desc` at `ty`, under the mapping in the module docs.
pub fn host_value(env: &TypeEnv, ty: &Type, desc: &Value) -> Value {
    let Some(resolved) = trace(env, ty) else {
        return host_blind(desc);
    };
    let tag = desc.get(0).and_then(Value::as_str).unwrap_or("");
    match resolved.as_ref() {
        TypeInner::Reserved => json!({ "kind": "reserved" }),
        TypeInner::Opt(inner) => {
            if tag == "n" {
                json!({ "kind": "opt", "value": null })
            } else if admits_null(env, inner) {
                match entries(desc) {
                    Some(list) if list.len() == 1 && list[0][0] == "some" => {
                        json!({ "kind": "opt", "value": host_value(env, inner, &list[0][1]) })
                    }
                    _ => host_blind(desc),
                }
            } else {
                json!({ "kind": "opt", "value": host_value(env, inner, desc) })
            }
        }
        TypeInner::Null if tag == "n" => json!({ "kind": "null" }),
        TypeInner::Nat | TypeInner::Int | TypeInner::Nat64 | TypeInner::Int64 if tag == "i" => {
            let kind = match resolved.as_ref() {
                TypeInner::Nat => "nat",
                TypeInner::Int => "int",
                TypeInner::Nat64 => "nat64",
                _ => "int64",
            };
            json!({ "kind": kind, "value": canonical_decimal(desc) })
        }
        TypeInner::Nat8
        | TypeInner::Nat16
        | TypeInner::Nat32
        | TypeInner::Int8
        | TypeInner::Int16
        | TypeInner::Int32 => {
            let (kind, min, max) = match resolved.as_ref() {
                TypeInner::Nat8 => ("nat8", 0.0, 255.0),
                TypeInner::Nat16 => ("nat16", 0.0, 65535.0),
                TypeInner::Nat32 => ("nat32", 0.0, 4_294_967_295.0),
                TypeInner::Int8 => ("int8", -128.0, 127.0),
                TypeInner::Int16 => ("int16", -32768.0, 32767.0),
                _ => ("int32", -2_147_483_648.0, 2_147_483_647.0),
            };
            match integral_in(desc, min, max) {
                Some(value) => json!({ "kind": kind, "value": value }),
                None => host_blind(desc),
            }
        }
        TypeInner::Float64 if tag == "f" => json!({ "kind": "float64", "bits": desc[1] }),
        TypeInner::Float32 if tag == "f" => {
            let bits = u64::from_str_radix(desc[1].as_str().unwrap_or("0"), 16).unwrap_or(0);
            let narrowed = f64::from_bits(bits) as f32;
            json!({ "kind": "float32", "bits": format!("{:08x}", narrowed.to_bits()) })
        }
        TypeInner::Text if tag == "s" => json!({ "kind": "text", "value": desc[1] }),
        TypeInner::Principal if tag == "s" => json!({ "kind": "principal", "value": desc[1] }),
        TypeInner::Service(_) if tag == "s" => json!({ "kind": "service", "principal": desc[1] }),
        TypeInner::Func(_) => match entries(desc) {
            Some(list)
                if list.len() == 2
                    && entry(list, "principal").and_then(|v| v.get(0)) == Some(&json!("s"))
                    && entry(list, "method").and_then(|v| v.get(0)) == Some(&json!("s")) =>
            {
                json!({
                    "kind": "func",
                    "principal": entry(list, "principal").map(|v| v[1].clone()),
                    "method": entry(list, "method").map(|v| v[1].clone()),
                })
            }
            _ => host_blind(desc),
        },
        TypeInner::Vec(inner) => {
            if is_blob(env, inner) {
                if tag == "y" {
                    host_blind(desc)
                } else {
                    // Only a Uint8Array is a blob in the domain; an array
                    // converts with its elements blind, so it never matches.
                    match tag {
                        "a" => json!({
                            "kind": "vec",
                            "values": desc[1].as_array().map(|items| items
                                .iter()
                                .map(|item| match item.get(0).and_then(Value::as_str) {
                                    Some("f") => json!({ "kind": "float64", "bits": item[1] }),
                                    _ => host_blind(item),
                                })
                                .collect::<Vec<_>>()).unwrap_or_default(),
                        }),
                        _ => host_blind(desc),
                    }
                }
            } else if tag == "a" {
                let values: Vec<Value> = desc[1]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| host_value(env, inner, item))
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "kind": "vec", "values": values })
            } else {
                host_blind(desc)
            }
        }
        TypeInner::Record(fields) => {
            let tuple = !fields.is_empty()
                && fields
                    .iter()
                    .enumerate()
                    .all(|(index, field)| field.id.get_id() == index as u32);
            let field_at = |id: u32, value: &Value| -> Value {
                match fields.iter().find(|field| field.id.get_id() == id) {
                    Some(field) => host_value(env, &field.ty, value),
                    None => host_blind(value),
                }
            };
            if tuple && tag == "a" {
                let converted: Vec<Value> = desc[1]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .enumerate()
                            .map(|(index, item)| {
                                json!({ "id": index, "value": field_at(index as u32, item) })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "kind": "record", "fields": converted })
            } else if !tuple && tag == "o" {
                let mut fresh = FreshIds::new(fields.iter().map(|field| field.id.get_id()));
                let converted: Vec<Value> = entries(desc)
                    .map(|list| {
                        list.iter()
                            .map(|entry| {
                                let key = entry[0].as_str().unwrap_or("");
                                let id = key_id(fields, key).unwrap_or_else(|| fresh.next());
                                json!({ "id": id, "value": field_at(id, &entry[1]) })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "kind": "record", "fields": converted })
            } else {
                host_blind(desc)
            }
        }
        TypeInner::Variant(arms) => match entries(desc) {
            Some(list)
                if entry(list, "tag").and_then(|v| v.get(0)) == Some(&json!("s"))
                    && list
                        .iter()
                        .all(|entry| entry[0] == "tag" || entry[0] == "value")
                    && list.len() <= 2 =>
            {
                let tag_text = entry(list, "tag").and_then(|v| v[1].as_str()).unwrap_or("");
                let id = key_id(arms, tag_text).unwrap_or_else(|| {
                    FreshIds::new(arms.iter().map(|arm| arm.id.get_id())).next()
                });
                let payload = match entry(list, "value") {
                    None => json!({ "kind": "null" }),
                    Some(value) => match arms.iter().find(|arm| arm.id.get_id() == id) {
                        Some(arm) => host_value(env, &arm.ty, value),
                        None => host_blind(value),
                    },
                };
                json!({ "kind": "variant", "id": id, "value": payload })
            }
            _ => host_blind(desc),
        },
        _ => host_blind(desc),
    }
}
