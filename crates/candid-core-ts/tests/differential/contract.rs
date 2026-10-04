//! The `schemaFromContract` target, reference half: a compiled Contract
//! document, edited by a short list of JSON operations the TypeScript runner
//! replays exactly, then judged by `Contract::from_json`.
//!
//! # Operations
//!
//! `{"op": "set", "path": [...], "value": v}` replaces the value at `path`
//! (an existing object key or array index, or a new object key);
//! `{"op": "delete", "path": [...]}` removes an object key or splices out an
//! array element; `{"op": "insert", "path": [..., index], "value": v}` splices
//! `v` into the array at `index`. Path segments are object keys (strings) or
//! array indices (numbers).
//!
//! # Identities are restamped
//!
//! `schemaFromContract` deliberately does not verify the `identities` hashes
//! (that needs canonicalization, candid-core's job; see `ts/contract.ts`),
//! while `Contract::from_json` refuses any document whose identities are not
//! its own. Comparing the two on edited documents would therefore only ever
//! report that difference. So unless an operation edits `identities` itself,
//! the reference half recomputes them for the edited graph (through
//! `ContractDraft::build`, which validates and canonicalizes the same way)
//! before calling `Contract::from_json` — the comparison is acceptance of the
//! *graph*, which is what both sides claim to check. The TypeScript half loads
//! the edited document with its original identities, which it never reads.

use serde_json::{json, Value};

use super::rng::Rng;
use super::wire::guarded;

fn paths(value: &Value, prefix: &mut Vec<Value>, out: &mut Vec<Vec<Value>>) {
    out.push(prefix.clone());
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                prefix.push(json!(key));
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                prefix.push(json!(index));
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn get<'a>(value: &'a Value, path: &[Value]) -> Option<&'a Value> {
    let mut node = value;
    for segment in path {
        node = match segment {
            Value::String(key) => node.as_object()?.get(key)?,
            Value::Number(index) => node.as_array()?.get(index.as_u64()? as usize)?,
            _ => return None,
        };
    }
    Some(node)
}

fn get_mut<'a>(value: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    let mut node = value;
    for segment in path {
        node = match segment {
            Value::String(key) => node.as_object_mut()?.get_mut(key)?,
            Value::Number(index) => node.as_array_mut()?.get_mut(index.as_u64()? as usize)?,
            _ => return None,
        };
    }
    Some(node)
}

/// Apply one operation; an operation whose path does not resolve is a no-op
/// on both sides.
pub fn apply(document: &mut Value, op: &Value) {
    let Some(path) = op["path"].as_array() else {
        return;
    };
    let Some((last, parent_path)) = path.split_last() else {
        return;
    };
    let Some(parent) = get_mut(document, parent_path) else {
        return;
    };
    match (op["op"].as_str(), last, parent) {
        (Some("set"), Value::String(key), Value::Object(map)) => {
            map.insert(key.clone(), op["value"].clone());
        }
        (Some("set"), Value::Number(index), Value::Array(items)) => {
            if let Some(slot) = items.get_mut(index.as_u64().unwrap_or(u64::MAX) as usize) {
                *slot = op["value"].clone();
            }
        }
        (Some("delete"), Value::String(key), Value::Object(map)) => {
            map.remove(key);
        }
        (Some("delete"), Value::Number(index), Value::Array(items)) => {
            let index = index.as_u64().unwrap_or(u64::MAX) as usize;
            if index < items.len() {
                items.remove(index);
            }
        }
        (Some("insert"), Value::Number(index), Value::Array(items)) => {
            let index = index.as_u64().unwrap_or(u64::MAX) as usize;
            if index <= items.len() {
                items.insert(index, op["value"].clone());
            }
        }
        _ => {}
    }
}

const STRINGS: &[&str] = &[
    "",
    "primitive",
    "opt",
    "vec",
    "record",
    "variant",
    "func",
    "service",
    "class",
    "nat",
    "int",
    "text",
    "empty",
    "reserved",
    "nat8",
    "bogus",
    "query",
    "oneway",
    "composite_query",
    "update",
    "T0",
    "_0_",
    "actor",
];

fn random_json(rng: &mut Rng, document: &Value) -> Value {
    match rng.below(10) {
        0 => json!(null),
        1 => json!(rng.chance(1, 2)),
        2 => json!([]),
        3 => json!({}),
        4 | 5 => {
            let candidates = [-1i64, 0, 1, 2, 3, 7, 100, 4_294_967_295, 4_294_967_296];
            json!(*rng.pick(&candidates))
        }
        6 => json!(1.5),
        7 => json!(rng.pick(STRINGS)),
        _ => {
            // A copy of a sibling or random node of the same document: keeps
            // edits structurally plausible.
            let mut all = Vec::new();
            paths(document, &mut Vec::new(), &mut all);
            let pick = rng.pick(&all).clone();
            get(document, &pick).cloned().unwrap_or(Value::Null)
        }
    }
}

const KINDS: &[&str] = &[
    "primitive",
    "opt",
    "vec",
    "record",
    "variant",
    "func",
    "service",
];
const PRIMITIVE_NAMES: &[&str] = &[
    "null",
    "bool",
    "nat",
    "int",
    "nat8",
    "nat16",
    "nat32",
    "nat64",
    "int8",
    "int16",
    "int32",
    "int64",
    "float32",
    "float64",
    "text",
    "reserved",
    "empty",
    "principal",
];
const MODE_NAMES: &[&str] = &["update", "query", "composite_query", "oneway"];

/// A plausible replacement for a leaf: same JSON type, nearby or
/// boundary value, or a sibling enum string, so most edits survive the
/// reference's serde layer and reach graph validation.
fn plausible(rng: &mut Rng, document: &Value, path: &[Value], target: &Value) -> Value {
    let key = path.iter().rev().find_map(Value::as_str).unwrap_or("");
    match target {
        Value::Number(number) => {
            let n = number.as_i64().unwrap_or(0);
            let nodes = document["types"].as_array().map_or(0, Vec::len) as i64;
            let candidates = [
                n - 1,
                n + 1,
                0,
                nodes - 1,
                nodes,
                nodes + 1,
                n + 97,
                4_294_967_295,
            ];
            json!((*rng.pick(&candidates)).max(0))
        }
        Value::String(_) => match key {
            "kind" => json!(rng.pick(KINDS)),
            "primitive" => json!(rng.pick(PRIMITIVE_NAMES)),
            "mode" => json!(rng.pick(MODE_NAMES)),
            _ => json!(rng.pick(STRINGS)),
        },
        Value::Bool(b) => json!(!b),
        _ => random_json(rng, document),
    }
}

/// One to three random operations on `document`, applied in place.
pub fn random_ops(rng: &mut Rng, document: &mut Value) -> Vec<Value> {
    let mut ops = Vec::new();
    for _ in 0..1 + rng.below(3) {
        let mut all = Vec::new();
        paths(document, &mut Vec::new(), &mut all);
        all.retain(|path| !path.is_empty());
        // Edits to `producer` and `identities` are kept rare: the TypeScript
        // loader reads neither, by design.
        let preferred: Vec<Vec<Value>> = all
            .iter()
            .filter(|path| path[0] != "producer" && path[0] != "identities")
            .cloned()
            .collect();
        let pool = if preferred.is_empty() || rng.chance(1, 20) {
            &all
        } else {
            &preferred
        };
        let leaves: Vec<Vec<Value>> = pool
            .iter()
            .filter(|path| {
                matches!(
                    get(document, path),
                    Some(Value::Number(_) | Value::String(_) | Value::Bool(_))
                )
            })
            .cloned()
            .collect();
        let elements: Vec<Vec<Value>> = pool
            .iter()
            .filter(|path| matches!(path.last(), Some(Value::Number(_))))
            .cloned()
            .collect();
        let choice = rng.weighted(&[12, 3, 2, 2, 1, 1, 1]);
        let source = match choice {
            0 if !leaves.is_empty() => &leaves,
            1..=3 if !elements.is_empty() => &elements,
            _ => pool,
        };
        let path = rng.pick(source).clone();
        let target = get(document, &path).cloned().unwrap_or(Value::Null);
        let in_array = matches!(path.last(), Some(Value::Number(_)));
        let op = match (choice, &target) {
            // Replace a leaf with a plausible value of its own JSON type.
            (0, Value::Number(_) | Value::String(_) | Value::Bool(_)) => json!({
                "op": "set", "path": path, "value": plausible(rng, document, &path, &target)
            }),
            // Remove an array element (a type node, a field, a declaration…).
            (1, _) if in_array => json!({ "op": "delete", "path": path }),
            // Duplicate an element next to itself, or move a copy elsewhere.
            (2, _) if in_array => {
                let mut insert_path = path.clone();
                let index = insert_path.pop().and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let length = get(document, &insert_path)
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                let at = if rng.chance(1, 2) {
                    index + 1
                } else {
                    rng.below(length + 1)
                };
                insert_path.push(json!(at));
                json!({ "op": "insert", "path": insert_path, "value": target })
            }
            // Swap two elements of the same array (two sets).
            (3, _) if in_array => {
                let mut other = path.clone();
                other.pop();
                let length = get(document, &other)
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                other.push(json!(rng.below(length.max(1))));
                let other_value = get(document, &other).cloned().unwrap_or(Value::Null);
                let first = json!({ "op": "set", "path": path, "value": other_value });
                apply(document, &first);
                ops.push(first);
                json!({ "op": "set", "path": other, "value": target })
            }
            // Delete an object key or element, or add an unknown key.
            (4, _) => json!({ "op": "delete", "path": path }),
            (5, Value::Object(_)) => {
                let mut key_path = path.clone();
                key_path.push(json!(rng.pick(&["extra", "kind", "inner", "fields", "id"])));
                json!({ "op": "set", "path": key_path, "value": random_json(rng, document) })
            }
            // Anything else: a wild value.
            _ => json!({ "op": "set", "path": path, "value": random_json(rng, document) }),
        };
        apply(document, &op);
        ops.push(op);
    }
    ops
}

/// The reference verdict on an edited document (identities restamped unless
/// an operation touched them).
pub fn reference_verdict(edited: &Value, ops: &[Value]) -> Value {
    let touches_identities = ops
        .iter()
        .any(|op| op["path"].get(0) == Some(&json!("identities")));
    let mut document = edited.clone();
    if !touches_identities {
        let mut draft = serde_json::Map::new();
        for key in ["types", "declarations", "actor"] {
            if let Some(value) = document.get(key) {
                draft.insert(key.to_string(), value.clone());
            }
        }
        let built = guarded(|| {
            serde_json::from_value::<candid_core::ContractDraft>(Value::Object(draft))
                .ok()
                .and_then(|draft| draft.build().ok())
        })
        .ok()
        .flatten();
        if let (Some(contract), Some(map)) = (built, document.as_object_mut()) {
            if map.contains_key("identities") {
                map.insert(
                    "identities".to_string(),
                    serde_json::to_value(contract.identities()).expect("identities serialize"),
                );
            }
        }
    }
    let text = document.to_string();
    match guarded(|| candid_core::Contract::from_json(&text)) {
        Err(_) => json!({ "verdict": "panic" }),
        Ok(Ok(_)) => json!({ "verdict": "accept" }),
        Ok(Err(candid_core::ContractJsonError::MalformedJson(message))) => {
            // `MalformedJson` is one variant for every shape error; its class
            // here is refined from the serde message's fixed prefix (the
            // reference's own wording, used only to name the class, never
            // compared with anything the runtime says).
            let class = if message.starts_with("unknown field") {
                "json_unknown_field"
            } else if message.starts_with("missing field") {
                "json_missing_field"
            } else if message.starts_with("unknown variant") {
                "json_unknown_variant"
            } else if message.starts_with("invalid type") || message.starts_with("invalid value") {
                "json_invalid_type"
            } else {
                "malformed_json"
            };
            json!({ "verdict": "reject", "class": class })
        }
        Ok(Err(candid_core::ContractJsonError::InvalidContract(error))) => {
            let code = error
                .violations
                .first()
                .map_or("unknown".to_string(), |violation| violation.code.clone());
            json!({ "verdict": "reject", "class": code })
        }
    }
}

fn touches_metadata(op: &Value) -> bool {
    matches!(
        op["path"].get(0).and_then(Value::as_str),
        Some("producer" | "identities")
    )
}

/// The reference verdict for `ops` applied to `base`. When an operation edits
/// `producer` or `identities` — the two parts the TypeScript loader does not
/// read, by design — a second verdict, `graph`, judges the document with only
/// the other operations applied, so the runner can tell a refusal caused by
/// the metadata edit (an intended difference) from one the graph causes.
pub fn judge(base: &Value, ops: &[Value]) -> Value {
    let mut edited = base.clone();
    for op in ops {
        apply(&mut edited, op);
    }
    let mut verdict = reference_verdict(&edited, ops);
    if ops.iter().any(touches_metadata) {
        let graph_ops: Vec<Value> = ops
            .iter()
            .filter(|op| !touches_metadata(op))
            .cloned()
            .collect();
        let mut graph = base.clone();
        for op in &graph_ops {
            apply(&mut graph, op);
        }
        verdict["graph"] = reference_verdict(&graph, &graph_ops);
    }
    verdict
}
