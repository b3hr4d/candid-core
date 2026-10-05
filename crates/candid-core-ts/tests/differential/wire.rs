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

/// Whether a resolved type is a composite the deep walk can descend into.
fn composite(env: &TypeEnv, ty: &Type) -> bool {
    matches!(
        trace(env, ty).as_deref(),
        Some(TypeInner::Opt(_) | TypeInner::Vec(_) | TypeInner::Record(_) | TypeInner::Variant(_))
    )
}

/// A value of `ty` whose composites nest `levels` deep along one path where
/// the type allows it (a deep environment's recursive shape or chain), every
/// other position shallow: an absent `opt`, an empty `vec`, a variant's
/// non-composite arm. Each `opt`, `vec`, `record` or `variant` on the path is
/// one level (a shallow composite at the bottom may add one more), as
/// `Scan::levels` reads them back from the bytes.
pub fn deep_value(env: &TypeEnv, ty: &Type, rng: &mut Rng, levels: usize) -> Option<IDLValue> {
    let mut budget = 4096usize;
    let ty = trace(env, ty)?;
    Some(match ty.as_ref() {
        TypeInner::Opt(inner) if levels > 1 => {
            IDLValue::Opt(Box::new(deep_value(env, inner, rng, levels - 1)?))
        }
        TypeInner::Opt(_) => IDLValue::None,
        TypeInner::Vec(inner) if levels > 1 => {
            IDLValue::Vec(vec![deep_value(env, inner, rng, levels - 1)?])
        }
        TypeInner::Vec(_) => IDLValue::Vec(Vec::new()),
        TypeInner::Record(fields) => {
            // The first composite field carries the depth.
            let deep = fields.iter().position(|field| composite(env, &field.ty));
            let mut values = Vec::new();
            for (index, field) in fields.iter().enumerate() {
                let val = if Some(index) == deep && levels > 1 {
                    deep_value(env, &field.ty, rng, levels - 1)?
                } else {
                    value_within(env, &field.ty, rng, 7, &mut budget)?
                };
                values.push(IDLField {
                    id: (*field.id).clone(),
                    val,
                });
            }
            IDLValue::Record(values)
        }
        TypeInner::Variant(fields) => {
            let wanted = levels > 1;
            let index = fields
                .iter()
                .position(|field| composite(env, &field.ty) == wanted)
                .unwrap_or(0);
            let field = fields.get(index)?;
            let val = if wanted {
                deep_value(env, &field.ty, rng, levels - 1)?
            } else {
                value_within(env, &field.ty, rng, 7, &mut budget)?
            };
            IDLValue::Variant(VariantValue(
                Box::new(IDLField {
                    id: (*field.id).clone(),
                    val,
                }),
                index as u64,
            ))
        }
        _ => value_within(env, &ty, rng, 7, &mut budget)?,
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

// ---------------------------------------------------------------------------
// Input properties: a scan of the message at its own wire types
// ---------------------------------------------------------------------------

/// The deepest composite nesting the runtime's default `maxDepth` (256 depth
/// steps) always admits. The runtime charges a value at most two steps per
/// Candid level — the constructor, and at most one `rec` hop to reach it (a
/// Contract-loaded schema needs one) — plus one for a leaf, so 127 levels
/// cost at most 2 × 127 + 1 = 255 steps: a depth refusal of a message that
/// nests no deeper is a runtime defect, not policy. Deeper, the limit is the
/// runtime's documented policy (the reference bounds only its stack).
pub const DEEP_LEVELS: usize = 127;

/// Half the runtime's default `maxElements` (1,000,000), for the same
/// reason: every value read (decoded or skipped) and at most one `rec` hop
/// per value charge that budget.
pub const MANY_VALUES: usize = 500_000;

/// What a message holds, read from its bytes at its own wire types and never
/// at the expected types: the input properties the verdict mapping ties every
/// intended or reference-side attribution to (issue #196 review). The scan
/// mirrors the wire format, not either decoder; it stops at the first byte it
/// cannot read, so a property later in a malformed message goes unseen.
#[derive(Default)]
pub struct Scan {
    /// A LEB128 or SLEB128 group longer than the value needs (`80 00`, or a
    /// signed group whose last byte only repeats the sign).
    pub non_minimal_leb128: bool,
    /// A future-type value with a non-zero reference count.
    pub future_references: bool,
    /// A func value whose method name is not well-formed UTF-8.
    pub invalid_method_utf8: bool,
    /// A func value with an empty method name: the runtime refuses one by
    /// decision (round-trip symmetry with `validate` and `encode`, see
    /// `ts/codec.ts`), the reference accepts it — or never reaches it, when
    /// an `opt` above absorbs a later failure.
    pub empty_method: bool,
    /// Bytes where the wire type is `empty`: no value has that type, so the
    /// scan stops there.
    pub empty_value: bool,
    /// A record the reference rewrites to `empty` (see `empty_records`)
    /// reachable from a func or service type in the table.
    pub empty_record_in_reference: bool,
    /// The deepest nesting of composite values (`opt`, `vec`, `record`,
    /// `variant`, an absent `opt` included) the scan read; `reference_verdict`
    /// raises it to the nesting of the values the reference decoded at the
    /// expected types, which a coercion can make deeper.
    pub levels: usize,
    /// How many values the scan read.
    pub values: usize,
}

impl Scan {
    /// The flags this scan contributes to a reference verdict.
    fn flags(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        for (set, name) in [
            (self.non_minimal_leb128, "non_minimal_leb128"),
            (self.future_references, "future_references"),
            (self.invalid_method_utf8, "invalid_method_utf8"),
            (self.empty_method, "empty_method"),
            (self.empty_value, "wire_empty_value"),
            (self.empty_record_in_reference, "wire_empty_record"),
            (self.levels > DEEP_LEVELS, "deep_nesting"),
            (self.values > MANY_VALUES, "many_values"),
        ] {
            if set {
                flags.push(name);
            }
        }
        flags
    }
}

enum Entry {
    Opt(i64),
    Vec(i64),
    Record(Vec<i64>),
    Variant(Vec<i64>),
    Func(Vec<i64>),
    Service(Vec<i64>),
    Future,
}

impl Entry {
    fn children(&self) -> &[i64] {
        match self {
            Entry::Opt(inner) | Entry::Vec(inner) => std::slice::from_ref(inner),
            Entry::Record(types)
            | Entry::Variant(types)
            | Entry::Func(types)
            | Entry::Service(types) => types,
            Entry::Future => &[],
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    non_minimal: bool,
}

impl Cursor<'_> {
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    fn skip(&mut self, count: u64) -> Option<&[u8]> {
        let count = usize::try_from(count).ok()?;
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    /// One LEB128 group of any length; its value saturates at `u64::MAX`.
    fn leb(&mut self) -> Option<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        let mut count = 0usize;
        loop {
            let byte = self.byte()?;
            count += 1;
            let low = u64::from(byte & 0x7f);
            if shift < 64 && (low << shift) >> shift == low {
                value |= low << shift;
            } else if low != 0 {
                value = u64::MAX;
            }
            shift = shift.saturating_add(7);
            if byte & 0x80 == 0 {
                if count > 1 && byte == 0 {
                    self.non_minimal = true;
                }
                return Some(value);
            }
        }
    }

    /// One SLEB128 group of any length; its value saturates.
    fn sleb(&mut self) -> Option<i64> {
        let mut value = 0i64;
        let mut shift = 0u32;
        let mut previous = 0u8;
        let mut count = 0usize;
        loop {
            let byte = self.byte()?;
            count += 1;
            if shift < 63 {
                value |= i64::from(byte & 0x7f) << shift;
            }
            shift = shift.saturating_add(7);
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    value |= -1i64 << shift;
                }
                if count > 1
                    && ((byte == 0x00 && previous & 0x40 == 0)
                        || (byte == 0x7f && previous & 0x40 != 0))
                {
                    self.non_minimal = true;
                }
                return Some(value);
            }
            previous = byte;
        }
    }
}

/// The parsed header: the table entries and the argument types. `None` for a
/// header that does not parse.
fn read_header(cursor: &mut Cursor) -> Option<(Vec<Entry>, Vec<i64>)> {
    for expected in *b"DIDL" {
        if cursor.byte()? != expected {
            return None;
        }
    }
    let count = cursor.leb()?;
    if count > cursor.bytes.len() as u64 {
        return None;
    }
    let mut entries = Vec::new();
    for _ in 0..count {
        let opcode = cursor.sleb()?;
        entries.push(match opcode {
            -18 => Entry::Opt(cursor.sleb()?),
            -19 => Entry::Vec(cursor.sleb()?),
            -20 | -21 => {
                let mut types = Vec::new();
                for _ in 0..cursor.leb()? {
                    cursor.leb()?;
                    types.push(cursor.sleb()?);
                }
                if opcode == -20 {
                    Entry::Record(types)
                } else {
                    Entry::Variant(types)
                }
            }
            -22 => {
                let mut types = Vec::new();
                for _ in 0..2 {
                    for _ in 0..cursor.leb()? {
                        types.push(cursor.sleb()?);
                    }
                }
                let annotations = cursor.byte()?;
                cursor.skip(u64::from(annotations))?;
                Entry::Func(types)
            }
            -23 => {
                let mut types = Vec::new();
                for _ in 0..cursor.leb()? {
                    let length = cursor.leb()?;
                    cursor.skip(length)?;
                    types.push(cursor.sleb()?);
                }
                Entry::Service(types)
            }
            opcode if opcode < -24 => {
                let length = cursor.leb()?;
                cursor.skip(length)?;
                Entry::Future
            }
            _ => return None,
        });
    }
    let mut args = Vec::new();
    for _ in 0..cursor.leb()? {
        args.push(cursor.sleb()?);
    }
    let valid =
        |ty: i64| (0..entries.len() as i64).contains(&ty) || (-17..=-1).contains(&ty) || ty == -24;
    let refs_valid = entries
        .iter()
        .all(|entry| entry.children().iter().all(|&ty| valid(ty)));
    (refs_valid && args.iter().all(|&ty| valid(ty))).then_some((entries, args))
}

/// The record entries the reference rewrites to `empty` while parsing the
/// table (`TypeEnv::replace_empty`): a record some field of which is, through
/// record fields only, the record itself — uninhabited.
fn empty_records(entries: &[Entry]) -> Vec<bool> {
    // 0 = unvisited, 1 = in progress, 2 = empty, 3 = not empty.
    fn empty(entries: &[Entry], state: &mut [u8], index: usize) -> bool {
        match state[index] {
            1 | 2 => {
                state[index] = 2;
                return true;
            }
            3 => return false,
            _ => {}
        }
        state[index] = 1;
        let result = match &entries[index] {
            Entry::Record(fields) => fields.iter().any(|&field| {
                usize::try_from(field).is_ok_and(|field| empty(entries, state, field))
            }),
            _ => false,
        };
        state[index] = if result { 2 } else { 3 };
        result
    }
    let mut state = vec![0u8; entries.len()];
    (0..entries.len())
        .map(|index| {
            matches!(entries[index], Entry::Record(_)) && empty(entries, &mut state, index)
        })
        .collect()
}

/// Whether a record the reference rewrites to `empty` is reachable from a
/// func or service entry: only there does the reference compare types (its
/// subtype check of reference values), and only there can the rewrite decide
/// a verdict (`decode:reference:empty-normalization`).
fn empty_record_in_reference(entries: &[Entry]) -> bool {
    let empty = empty_records(entries);
    let mut seen = vec![false; entries.len()];
    let mut stack: Vec<usize> = Vec::new();
    for entry in entries {
        if matches!(entry, Entry::Func(_) | Entry::Service(_)) {
            stack.extend(
                entry
                    .children()
                    .iter()
                    .filter_map(|&ty| usize::try_from(ty).ok()),
            );
        }
    }
    while let Some(index) = stack.pop() {
        if std::mem::replace(&mut seen[index], true) {
            continue;
        }
        if empty[index] {
            return true;
        }
        stack.extend(
            entries[index]
                .children()
                .iter()
                .filter_map(|&ty| usize::try_from(ty).ok()),
        );
    }
    false
}

/// The scan's walk bounds: it runs on the generator's large stack, and a
/// recursive record that only recurses would otherwise never consume a byte.
const SCAN_LEVELS: usize = 10_000;
const SCAN_VALUES: usize = 4_000_000;

struct Walker<'a, 'b> {
    cursor: Cursor<'a>,
    entries: &'b [Entry],
    scan: Scan,
}

impl Walker<'_, '_> {
    fn principal(&mut self) -> Option<()> {
        if self.cursor.byte()? != 1 {
            return None;
        }
        let length = self.cursor.leb()?;
        self.cursor.skip(length)?;
        Some(())
    }

    /// Read one value of wire type `ty`; `None` where the scan stops.
    fn value(&mut self, ty: i64, level: usize) -> Option<()> {
        self.scan.values += 1;
        if self.scan.values > SCAN_VALUES || level > SCAN_LEVELS {
            return None;
        }
        let Ok(index) = usize::try_from(ty) else {
            return match ty {
                -1 | -16 => Some(()),
                -2 | -5 | -9 => self.cursor.skip(1).map(drop),
                -6 | -10 => self.cursor.skip(2).map(drop),
                -7 | -11 | -13 => self.cursor.skip(4).map(drop),
                -8 | -12 | -14 => self.cursor.skip(8).map(drop),
                -3 => self.cursor.leb().map(drop),
                -4 => self.cursor.sleb().map(drop),
                -15 => {
                    let length = self.cursor.leb()?;
                    self.cursor.skip(length).map(drop)
                }
                -24 => self.principal(),
                _ => {
                    self.scan.empty_value = true;
                    None
                }
            };
        };
        let entries = self.entries;
        let entry = &entries[index];
        if matches!(
            entry,
            Entry::Opt(_) | Entry::Vec(_) | Entry::Record(_) | Entry::Variant(_)
        ) {
            self.scan.levels = self.scan.levels.max(level + 1);
        }
        match entry {
            Entry::Opt(inner) => match self.cursor.byte()? {
                0 => Some(()),
                1 => self.value(*inner, level + 1),
                _ => None,
            },
            Entry::Vec(inner) => {
                for _ in 0..self.cursor.leb()? {
                    self.value(*inner, level + 1)?;
                }
                Some(())
            }
            Entry::Record(fields) => {
                for &field in fields {
                    self.value(field, level + 1)?;
                }
                Some(())
            }
            Entry::Variant(arms) => {
                let arm = usize::try_from(self.cursor.leb()?).ok()?;
                self.value(*arms.get(arm)?, level + 1)
            }
            Entry::Func(_) => {
                if self.cursor.byte()? != 1 {
                    return None;
                }
                self.principal()?;
                let length = self.cursor.leb()?;
                if length == 0 {
                    self.scan.empty_method = true;
                }
                if std::str::from_utf8(self.cursor.skip(length)?).is_err() {
                    self.scan.invalid_method_utf8 = true;
                }
                Some(())
            }
            Entry::Service(_) => self.principal(),
            Entry::Future => {
                let length = self.cursor.leb()?;
                if self.cursor.leb()? != 0 {
                    self.scan.future_references = true;
                }
                self.cursor.skip(length).map(drop)
            }
        }
    }
}

/// Scan a message at its own wire types (see `Scan`).
pub fn scan(bytes: &[u8]) -> Scan {
    let mut cursor = Cursor {
        bytes,
        at: 0,
        non_minimal: false,
    };
    let Some((entries, args)) = read_header(&mut cursor) else {
        return Scan {
            non_minimal_leb128: cursor.non_minimal,
            ..Scan::default()
        };
    };
    let mut walker = Walker {
        cursor,
        entries: &entries,
        scan: Scan {
            empty_record_in_reference: empty_record_in_reference(&entries),
            ..Scan::default()
        },
    };
    for ty in args {
        if walker.value(ty, 0).is_none() {
            break;
        }
    }
    let mut scan = walker.scan;
    scan.non_minimal_leb128 = walker.cursor.non_minimal;
    scan
}

/// The reference decoder's configuration. Unconfigured, the `candid` crate
/// bounds no work (its documentation asks canister code to set a quota), and
/// a vector of a zero-sized type with a forged length of 2^40 then runs for
/// hours. The decoding quota (which charges skipped values 50x) is far above
/// what any unmutated generated message needs; a forged length can still
/// exhaust it below the runtime's own element budget (`maxElements`,
/// 1,000,000) — about ten thousand skipped `null`s do — and the verdict
/// mapping reports that as the harness policy it is
/// (`decode:intended:reference-quota`).
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
/// `flags` records input properties the verdict mapping ties attributions
/// to, never anything the runtime says: the scan's properties of the bytes
/// themselves (see `Scan::flags`).
pub fn reference_verdict(env: &TypeEnv, bytes: &[u8], expected: &[Type]) -> Value {
    let (mut verdict, decoded_levels) = reference_verdict_unflagged(env, bytes, expected);
    let mut scan = scan(bytes);
    // The runtime walks the expected types, which can nest deeper than the
    // wire values (an `opt` the coercion inserts at every level), so the
    // depth flag also reads the values the reference decoded.
    scan.levels = scan.levels.max(decoded_levels);
    let flags = scan.flags();
    if !flags.is_empty() {
        verdict["flags"] = json!(flags);
    }
    verdict
}

/// The composite nesting of a decoded value, counted as `Scan::levels`
/// counts it on the wire (an absent `opt` included).
fn value_levels(value: &IDLValue) -> usize {
    match value {
        IDLValue::None => 1,
        IDLValue::Opt(inner) => 1 + value_levels(inner),
        IDLValue::Vec(items) => 1 + items.iter().map(value_levels).max().unwrap_or(0),
        IDLValue::Blob(_) => 1,
        IDLValue::Record(fields) => {
            1 + fields
                .iter()
                .map(|field| value_levels(&field.val))
                .max()
                .unwrap_or(0)
        }
        IDLValue::Variant(variant) => 1 + value_levels(&variant.0.val),
        _ => 0,
    }
}

/// The verdict, and the composite nesting of the values the reference
/// decoded (0 when it decoded none).
fn reference_verdict_unflagged(env: &TypeEnv, bytes: &[u8], expected: &[Type]) -> (Value, usize) {
    let typed =
        guarded(|| IDLArgs::from_bytes_with_types_with_config(bytes, env, expected, &config()));
    let decoded = match typed {
        Err(()) => return (json!({ "verdict": "panic" }), 0),
        Ok(Err(error)) => return (reference_rejection(bytes, &error), 0),
        Ok(Ok(decoded)) => decoded,
    };
    let levels = decoded.args.iter().map(value_levels).max().unwrap_or(0);
    let mut values = Vec::new();
    for (value, ty) in decoded.args.iter().zip(expected) {
        match domain(env, ty, value) {
            Ok(mapped) => values.push(mapped),
            Err(message) => {
                return (
                    json!({ "verdict": "mapping_error", "detail": message }),
                    levels,
                );
            }
        }
    }
    (json!({ "verdict": "accept", "values": values }), levels)
}

/// The verdict for a typed decode the reference refused, classified by its
/// behaviour (see `reference_verdict`).
fn reference_rejection(bytes: &[u8], error: &candid::Error) -> Value {
    if quota_exhausted(error) {
        return json!({ "verdict": "reject", "class": "limit" });
    }
    let header = guarded(|| candid::de::IDLDeserialize::new_with_config(bytes, &config()).is_ok())
        .unwrap_or(false);
    if !header {
        return json!({ "verdict": "reject", "class": "header" });
    }
    match guarded(|| IDLArgs::from_bytes_with_config(bytes, &config())) {
        Ok(Ok(_)) => json!({ "verdict": "reject", "class": "coercion" }),
        Ok(Err(error)) if quota_exhausted(&error) => {
            json!({ "verdict": "reject", "class": "limit" })
        }
        _ => json!({ "verdict": "reject", "class": "malformed" }),
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
