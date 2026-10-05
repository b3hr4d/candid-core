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

/// The bytes of a hex string. A regression vector may also write a run as
/// `(<hex>*<count>)` (`4449444c(6e7f*3)…` is the hex with `6e7f` written out
/// three times), so a boundary vector of a million bytes stays a short line;
/// `compare.ts`'s `fromHex` reads the same notation. Generated cases are plain
/// hex.
pub fn unhex(text: &str) -> Vec<u8> {
    fn pairs(text: &str, out: &mut Vec<u8>) {
        assert!(text.len() % 2 == 0, "an even number of hex digits");
        for i in (0..text.len()).step_by(2) {
            out.push(u8::from_str_radix(&text[i..i + 2], 16).expect("valid hex"));
        }
    }
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        pairs(&rest[..open], &mut out);
        let close = open + rest[open..].find(')').expect("a run closes with `)`");
        let (unit, count) = rest[open + 1..close]
            .split_once('*')
            .expect("a run is `(<hex>*<count>)`");
        let count: usize = count.parse().expect("a run count");
        let mut bytes = Vec::new();
        pairs(unit, &mut bytes);
        for _ in 0..count {
            out.extend_from_slice(&bytes);
        }
        rest = &rest[close + 1..];
    }
    pairs(rest, &mut out);
    out
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
    match rng.weighted(&[3, 3, 2, 2, 2, 2, 4, 1, 3]) {
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
        7 => {
            let at = rng.below(bytes.len());
            let byte = bytes[at];
            bytes.insert(at, byte);
            "duplicate_byte"
        }
        _ => mutate_table(rng, bytes).unwrap_or_else(|| {
            let at = rng.below(bytes.len());
            bytes[at] = rng.byte();
            "set_byte"
        }),
    }
}

/// The byte ranges of one field, arm or method in a type table entry: its
/// key (a field id's LEB128 group, or a method name's length and bytes)
/// starts at `start` and ends at `key_end`, and its type index ends at `end`.
struct Item {
    start: usize,
    key_end: usize,
    end: usize,
}

/// The entries a structural table edit can target, by opcode.
enum Keyed {
    Record(Vec<Item>),
    Variant(Vec<Item>),
    Service(Vec<Item>),
}

/// The keyed entries of a message's type table with their byte ranges, or
/// `None` when the table does not parse.
fn keyed_entries(bytes: &[u8]) -> Option<Vec<Keyed>> {
    let mut cursor = Cursor { bytes, at: 0 };
    for expected in *b"DIDL" {
        if cursor.byte()? != expected {
            return None;
        }
    }
    let count = cursor.leb()?;
    if count > bytes.len() as u64 {
        return None;
    }
    let mut keyed = Vec::new();
    for _ in 0..count {
        let opcode = cursor.sleb()?;
        match opcode {
            -18 | -19 => {
                cursor.sleb()?;
            }
            -20 | -21 => {
                let mut items = Vec::new();
                for _ in 0..cursor.leb()? {
                    let start = cursor.at;
                    cursor.leb()?;
                    let key_end = cursor.at;
                    cursor.sleb()?;
                    items.push(Item {
                        start,
                        key_end,
                        end: cursor.at,
                    });
                }
                keyed.push(if opcode == -20 {
                    Keyed::Record(items)
                } else {
                    Keyed::Variant(items)
                });
            }
            -22 => {
                for _ in 0..2 {
                    for _ in 0..cursor.leb()? {
                        cursor.sleb()?;
                    }
                }
                let annotations = cursor.byte()?;
                cursor.skip(u64::from(annotations))?;
            }
            -23 => {
                let mut items = Vec::new();
                for _ in 0..cursor.leb()? {
                    let start = cursor.at;
                    let length = cursor.leb()?;
                    cursor.skip(length)?;
                    let key_end = cursor.at;
                    cursor.sleb()?;
                    items.push(Item {
                        start,
                        key_end,
                        end: cursor.at,
                    });
                }
                keyed.push(Keyed::Service(items));
            }
            opcode if opcode < -24 => {
                let length = cursor.leb()?;
                cursor.skip(length)?;
            }
            _ => return None,
        }
    }
    Some(keyed)
}

/// A structurally invalid type table (issue #196 review): two neighbouring
/// fields, arms or methods of one entry get the same key (a duplicate field
/// id, variant id or method name) or swap places (keys no longer strictly
/// increasing). Both decoders must refuse every one. `None` when the table
/// has no entry with two keys.
pub fn mutate_table(rng: &mut Rng, bytes: &mut Vec<u8>) -> Option<&'static str> {
    let keyed = keyed_entries(bytes)?;
    let candidates: Vec<&Keyed> = keyed
        .iter()
        .filter(|entry| match entry {
            Keyed::Record(items) | Keyed::Variant(items) | Keyed::Service(items) => {
                items.len() >= 2
            }
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // A kind first, then an entry of it: services are rare in a table, and
    // their edits must not be.
    let kind = |entry: &Keyed| match entry {
        Keyed::Record(_) => 0,
        Keyed::Variant(_) => 1,
        Keyed::Service(_) => 2,
    };
    let mut kinds: Vec<u8> = candidates.iter().map(|entry| kind(entry)).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let chosen = *rng.pick(&kinds);
    let of_kind: Vec<&Keyed> = candidates
        .into_iter()
        .filter(|entry| kind(entry) == chosen)
        .collect();
    let entry = *rng.pick(&of_kind);
    let (items, duplicate, swap) = match entry {
        Keyed::Record(items) => (items, "duplicate_field_id", "unsorted_field_ids"),
        Keyed::Variant(items) => (items, "duplicate_variant_id", "unsorted_variant_ids"),
        Keyed::Service(items) => (items, "duplicate_method_name", "unsorted_method_names"),
    };
    let j = 1 + rng.below(items.len() - 1);
    let (first, second) = (&items[j - 1], &items[j]);
    if rng.chance(1, 2) {
        let key = bytes[first.start..first.key_end].to_vec();
        bytes.splice(second.start..second.key_end, key);
        Some(duplicate)
    } else {
        let mut swapped = bytes[second.start..second.end].to_vec();
        swapped.extend_from_slice(&bytes[first.start..first.end]);
        bytes.splice(first.start..second.end, swapped);
        Some(swap)
    }
}

// ---------------------------------------------------------------------------
// Generation bounds: a scan of the message at its own wire types
// ---------------------------------------------------------------------------

/// The deepest nesting, in composite levels at the *expected* types, a
/// generated decode or validate case may reach. The runtime's default
/// `maxDepth` (256) charges a Contract-loaded value two steps per level, the
/// `rec` hop and the constructor (issue #231), and refuses the first node at
/// level 128 (step 257), while the `candid` crate has no depth budget at all
/// (its stack is its bound). The two cannot be configured alike, so random
/// generation stays strictly below the runtime's bound, and the boundary
/// itself is pinned only by exact regression vectors.
pub const GEN_LEVELS: usize = 127;

/// The most values (as the scan counts them) a generated decode case's
/// message may hold, before the expansion factor: the runtime's `maxElements`
/// (1,000,000) charges every value read, decoded or skipped, each `rec` hop
/// and each `opt` a coercion inserts, while the `candid` crate has no element
/// budget. Random generation stays far below it (see `within_bounds`); the
/// boundary is pinned by exact regression vectors.
pub const GEN_ELEMENTS: usize = 250_000;

/// The runtime's `maxTypeTableEntries`: it refuses a table *claiming* more
/// entries before reading one. The `candid` crate has the same budget
/// (`DecoderConfig::set_max_type_len`, checked on the claimed count before any
/// entry is read; 10,000 unless configured), and `config` sets it to this
/// value, so the two refuse exactly the same claims and a refusal on it is a
/// verdict compared like any other (`TABLE_LIMIT_CLASS`). Generated messages
/// claim no more; exact vectors pin the boundary (`table_entries_100000`,
/// `table_entries_100001`).
pub const GEN_TABLE_ENTRIES: u64 = 100_000;

/// The class of the reference's refusal on its type-table budget, aligned
/// with the runtime's (see `GEN_TABLE_ENTRIES`): the judge agrees it only
/// with the runtime's `resource_limit_exceeded` on `type_table_entries`.
const TABLE_LIMIT_CLASS: &str = "resource_limit_exceeded/type_table_entries";

/// The longest length (a `vec` count, a `text`, principal or method name
/// byte length) a generated message may claim. The `candid` crate charges a
/// claimed length to its decoding quota before it looks for the bytes (a
/// primitive `vec` of `n` elements costs about `11 n` at once), so a forged
/// length would exhaust any fixed quota and leave the message unjudged; the
/// quota is sized from this bound instead (see `quota`).
pub const GEN_LENGTH: u64 = 1_000_000;

/// What a message holds, read from its bytes at its own wire types and never
/// at the expected types: what the generator bounds (see `within_bounds`).
/// The scan mirrors the wire format, not either decoder; it stops at the
/// first byte it cannot read, where both decoders stop too.
#[derive(Default)]
pub struct Scan {
    /// The entry count the type table claims (0 when even that is unreadable).
    pub claimed_types: u64,
    /// The deepest nesting of composite values (`opt`, `vec`, `record`,
    /// `variant`, an absent `opt` included) the scan read.
    pub levels: usize,
    /// How many values the scan read.
    pub values: usize,
    /// The longest length the scan read (see `GEN_LENGTH`).
    pub max_length: u64,
    /// The scan stopped at its own walk bound (`SCAN_LEVELS`, `SCAN_VALUES`):
    /// the message holds more than any bound here admits (a forged length of
    /// a zero-sized type, a record that only recurses).
    pub unbounded: bool,
}

impl Scan {
    /// Why a decode case's message is outside the generation bounds that
    /// the scan alone decides, or `None`: checked *before* the reference
    /// runs, since such a message (a forged length of a zero-sized type) can
    /// cost the reference hours. A case outside is redrawn, never judged:
    /// within the bounds the runtime's budgets cannot decide a verdict, and
    /// the reference's decoding quota (sized from this scan) cannot run out.
    /// `expansion` is the most `opt` levels a coercion can insert above a
    /// wire value at the expected types (see `expansion`).
    pub fn outside_bounds(&self, expansion: usize) -> Option<&'static str> {
        if self.unbounded {
            Some("unbounded")
        } else if self.claimed_types > GEN_TABLE_ENTRIES {
            Some("table_entries")
        } else if self.max_length > GEN_LENGTH {
            Some("length")
        } else if (self.values + 16).saturating_mul(4 * (1 + expansion)) > GEN_ELEMENTS {
            Some("values")
        } else {
            None
        }
    }

    /// Whether a decode case nests too deep at the expected types, checked
    /// once the reference has run. The runtime walks the *expected* types,
    /// which nest deeper than the wire where a coercion inserts an `opt` (an
    /// expected `opt` reading a non-`opt` wire value). Where the reference
    /// decoded the message, its values at the expected types (`decoded`, see
    /// `reference_judgement`) measure that nesting exactly; elsewhere the
    /// bound is static: up to `expansion` `opt` levels above every wire level.
    pub fn too_deep(&self, expansion: usize, decoded: Option<usize>) -> bool {
        let nesting = match decoded {
            Some(levels) => levels.max(self.levels) + 2,
            None => (self.levels + 2).saturating_mul(1 + expansion),
        };
        nesting > GEN_LEVELS
    }
}

/// The most `opt` constructors one coercion can stack above a wire value
/// when reading at `types`: the longest run of `opt` constructors, through
/// declarations, at any position reachable from them (an expected `opt`
/// reads a non-`opt` wire value at its inner type). `None` when a run never
/// ends — an `opt`-only cycle such as `T = opt T`, which the generator never
/// draws (see `opt_cycle`).
pub fn expansion(env: &TypeEnv, types: &[Type]) -> Option<usize> {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack: Vec<Type> = types.to_vec();
    let mut most = 0;
    while let Some(ty) = stack.pop() {
        most = most.max(opt_run(env, &ty, &mut Vec::new())?);
        match ty.as_ref() {
            TypeInner::Var(name) => {
                if seen.insert(name.clone()) {
                    stack.push(env.find_type(name).ok()?.clone());
                }
            }
            TypeInner::Opt(inner) | TypeInner::Vec(inner) => stack.push(inner.clone()),
            TypeInner::Record(fields) | TypeInner::Variant(fields) => {
                stack.extend(fields.iter().map(|field| field.ty.clone()));
            }
            // A func or service value is a reference: nothing is decoded at
            // its argument or method types.
            _ => {}
        }
    }
    Some(most)
}

fn opt_run(env: &TypeEnv, ty: &Type, names: &mut Vec<String>) -> Option<usize> {
    match ty.as_ref() {
        TypeInner::Var(name) => {
            if names.contains(name) {
                return None;
            }
            names.push(name.clone());
            let body = env.find_type(name).ok()?.clone();
            let run = opt_run(env, &body, names);
            names.pop();
            run
        }
        TypeInner::Opt(inner) => Some(1 + opt_run(env, inner, names)?),
        _ => Some(0),
    }
}

/// Whether a declaration of the environment is an `opt`-only cycle
/// (`T = opt T`, or `T = opt U; U = opt T`). Reading a non-`opt` wire value
/// there unwraps the expected type without end on both sides: the reference
/// until its stack guard, the runtime until `maxDepth`. Neither judges it, so
/// the generator redraws such an environment (counted as `opt_cycle`).
pub fn opt_cycle(env: &TypeEnv, names: &[String]) -> bool {
    names
        .iter()
        .any(|name| opt_run(env, &TypeInner::Var(name.clone()).into(), &mut Vec::new()).is_none())
}

enum Entry {
    Opt(i64),
    Vec(i64),
    Record(Vec<i64>),
    Variant(Vec<i64>),
    Func,
    Service,
    Future,
}

impl Entry {
    fn children(&self) -> &[i64] {
        match self {
            Entry::Opt(inner) | Entry::Vec(inner) => std::slice::from_ref(inner),
            Entry::Record(types) | Entry::Variant(types) => types,
            Entry::Func | Entry::Service | Entry::Future => &[],
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
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
        loop {
            let byte = self.byte()?;
            let low = u64::from(byte & 0x7f);
            if shift < 64 && (low << shift) >> shift == low {
                value |= low << shift;
            } else if low != 0 {
                value = u64::MAX;
            }
            shift = shift.saturating_add(7);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
    }

    /// One SLEB128 group of any length; its value saturates.
    fn sleb(&mut self) -> Option<i64> {
        let mut value = 0i64;
        let mut shift = 0u32;
        loop {
            let byte = self.byte()?;
            if shift < 63 {
                value |= i64::from(byte & 0x7f) << shift;
            }
            shift = shift.saturating_add(7);
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    value |= -1i64 << shift;
                }
                return Some(value);
            }
        }
    }
}

/// The parsed header: the table entries and the argument types. `None` for a
/// header that does not parse; `claimed` receives the table's claimed size.
fn read_header(cursor: &mut Cursor, claimed: &mut u64) -> Option<(Vec<Entry>, Vec<i64>)> {
    for expected in *b"DIDL" {
        if cursor.byte()? != expected {
            return None;
        }
    }
    let count = cursor.leb()?;
    *claimed = count;
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
                for _ in 0..2 {
                    for _ in 0..cursor.leb()? {
                        cursor.sleb()?;
                    }
                }
                let annotations = cursor.byte()?;
                cursor.skip(u64::from(annotations))?;
                Entry::Func
            }
            -23 => {
                for _ in 0..cursor.leb()? {
                    let length = cursor.leb()?;
                    cursor.skip(length)?;
                    cursor.sleb()?;
                }
                Entry::Service
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
        let length = self.length()?;
        self.cursor.skip(length)?;
        Some(())
    }

    /// A length (a count or a byte length), recorded in `max_length`.
    fn length(&mut self) -> Option<u64> {
        let length = self.cursor.leb()?;
        self.scan.max_length = self.scan.max_length.max(length);
        Some(length)
    }

    /// Read one value of wire type `ty`; `None` where the scan stops.
    fn value(&mut self, ty: i64, level: usize) -> Option<()> {
        self.scan.values += 1;
        if self.scan.values > SCAN_VALUES || level > SCAN_LEVELS {
            self.scan.unbounded = true;
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
                    let length = self.length()?;
                    self.cursor.skip(length).map(drop)
                }
                -24 => self.principal(),
                // `empty`: no value has that type.
                _ => None,
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
                for _ in 0..self.length()? {
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
            Entry::Func => {
                if self.cursor.byte()? != 1 {
                    return None;
                }
                self.principal()?;
                let length = self.length()?;
                self.cursor.skip(length).map(drop)
            }
            Entry::Service => self.principal(),
            Entry::Future => {
                let length = self.length()?;
                self.cursor.leb()?;
                self.cursor.skip(length).map(drop)
            }
        }
    }
}

/// Scan a message at its own wire types (see `Scan`).
pub fn scan(bytes: &[u8]) -> Scan {
    let mut cursor = Cursor { bytes, at: 0 };
    let mut claimed_types = 0;
    let Some((entries, args)) = read_header(&mut cursor, &mut claimed_types) else {
        return Scan {
            claimed_types,
            ..Scan::default()
        };
    };
    let mut walker = Walker {
        cursor,
        entries: &entries,
        scan: Scan {
            claimed_types,
            ..Scan::default()
        },
    };
    for ty in args {
        if walker.value(ty, 0).is_none() {
            break;
        }
    }
    walker.scan
}

// ---------------------------------------------------------------------------
// Reference verdict and the domain mapping
// ---------------------------------------------------------------------------

/// The least decoding quota. Unconfigured, the `candid` crate bounds no work
/// (its documentation asks canister code to set a quota), and a vector of a
/// zero-sized type with a forged length of 2^40 then runs for hours. The
/// generator keeps such messages out (`Scan::outside_bounds`); the quota is
/// the backstop, and a message that exhausts it is `inconclusive`, never a
/// rejection.
const QUOTA_FLOOR: usize = 20_000_000;

/// The decoding quota for a message: at least `QUOTA_FLOOR`, and at least
/// 1,000 per value, 4 per byte and 16 per unit of the longest claimed length,
/// charged 50x as a skipped value would be. The `candid` crate charges a
/// value a few dozen units beyond its bytes (a principal 30, a record field
/// 4, an expected field's name its length) and a claimed length at most
/// about 11 per unit, so this quota does not run out on a message within the
/// generation bounds.
fn quota(scan: &Scan, bytes: usize) -> usize {
    let length = usize::try_from(scan.max_length).unwrap_or(usize::MAX);
    scan.values
        .saturating_mul(1_000)
        .saturating_add(bytes.saturating_mul(4))
        .saturating_add(length.saturating_mul(16))
        .saturating_mul(50)
        .max(QUOTA_FLOOR)
}

fn config(quota: usize) -> candid::DecoderConfig {
    let mut config = candid::DecoderConfig::new();
    config.set_decoding_quota(quota);
    config.set_max_type_len(usize::try_from(GEN_TABLE_ENTRIES).expect("fits"));
    config
}

/// Whether a refusal is the reference's type-table budget (see
/// `GEN_TABLE_ENTRIES`), which the `candid` crate reports only as message
/// text (`type table size exceeded`).
fn table_limit(error: &candid::Error) -> bool {
    format!("{error:?}").contains("type table size exceeded")
}

/// The reference's own budget a refusal came from, if any: its decoding
/// quota (`… cost exceeds the limit`) or its stack guard (`Recursion limit
/// exceeded`, `Recursion depth overflow`). The `candid` crate reports both
/// only as message text, so these classes are read from its wording; they are
/// never compared with anything the runtime says. A budget refusal means the
/// reference did not judge the message: the case is `inconclusive`.
fn budget(error: &candid::Error) -> Option<&'static str> {
    let text = format!("{error:?}");
    if text.contains("cost exceeds the limit") {
        Some("quota")
    } else if text.contains("Recursion limit exceeded") || text.contains("Recursion depth overflow")
    {
        Some("stack")
    } else {
        None
    }
}

fn inconclusive(budget: &str) -> Value {
    json!({ "verdict": "inconclusive", "budget": budget })
}

/// The reference verdict for one decode case, as the golden records it.
///
/// A rejection carries a class derived from the reference's *behaviour*, not
/// its message text: `header` when the reference cannot even parse the
/// header and type table (`IDLDeserialize::new` fails), `malformed` when the
/// header parses but the message does not decode at its own wire types
/// (`IDLArgs::from_bytes` fails), and `coercion` when the message is
/// well-formed and only the expected types refuse it. A refusal on the
/// reference's own budget is no verdict at all: `inconclusive`, with the
/// budget named (see `budget`). The one reference budget configured like the
/// runtime's is the type table's claimed size (`GEN_TABLE_ENTRIES`): a refusal
/// there is a verdict, `TABLE_LIMIT_CLASS`. The reference has no depth or
/// element budget that could be configured like the runtime's; the generator
/// keeps cases below the runtime's (see `GEN_LEVELS`).
pub fn reference_verdict(env: &TypeEnv, bytes: &[u8], expected: &[Type]) -> Value {
    reference_judgement(env, bytes, expected).0
}

/// The reference verdict (see `reference_verdict`) and, when the reference
/// accepted, the composite nesting of the values it decoded at the expected
/// types (`value_levels`): what the generator bounds a case by where the
/// reference decodes it (see `Scan::outside_bounds`).
pub fn reference_judgement(
    env: &TypeEnv,
    bytes: &[u8],
    expected: &[Type],
) -> (Value, Option<usize>) {
    let quota = quota(&scan(bytes), bytes.len());
    let typed = guarded(|| {
        IDLArgs::from_bytes_with_types_with_config(bytes, env, expected, &config(quota))
    });
    let decoded = match typed {
        Err(()) => return (json!({ "verdict": "panic" }), None),
        Ok(Err(error)) => return (reference_rejection(bytes, &error, quota), None),
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
                    Some(levels),
                );
            }
        }
    }
    let values = Value::Array(values);
    let canonical = canonical_json(&values);
    let verdict = if canonical.len() > VALUES_RECORDED_BYTES {
        json!({ "verdict": "accept", "values_digest": values_digest(&canonical) })
    } else {
        json!({ "verdict": "accept", "values": values })
    };
    (verdict, Some(levels))
}

/// The longest decoded-values JSON a corpus line records in full: a width
/// vector decodes a million values, megabytes the golden need not hold, so
/// past this the line records `values_digest` instead and the judge compares
/// the digest of the runtime's values (`compare.ts`'s `valuesDigest`).
/// Generated cases stay far below it.
const VALUES_RECORDED_BYTES: usize = 65_536;

/// JSON with every object's keys in UTF-16 code-unit order (JavaScript's
/// default sort), so this text and `compare.ts`'s `canonicalJson` are equal
/// for equal values: the domain mapping holds no numbers, only strings,
/// booleans, `null`, arrays and objects.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let inner: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        other => other.to_string(),
    }
}

/// `fnv1a32:<8 hex digits>/<length>` of canonical JSON, hashed over its
/// UTF-16 code units as `compare.ts`'s `digest` hashes a string.
fn values_digest(canonical: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    let mut length = 0usize;
    for unit in canonical.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
        length += 1;
    }
    format!("fnv1a32:{hash:08x}/{length}")
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

/// The verdict for a typed decode the reference refused, classified by its
/// behaviour (see `reference_verdict`).
fn reference_rejection(bytes: &[u8], error: &candid::Error, quota: usize) -> Value {
    if let Some(budget) = budget(error) {
        return inconclusive(budget);
    }
    if table_limit(error) {
        return json!({ "verdict": "reject", "class": TABLE_LIMIT_CLASS });
    }
    match guarded(|| candid::de::IDLDeserialize::new_with_config(bytes, &config(quota)).err()) {
        Err(()) => return json!({ "verdict": "panic" }),
        Ok(Some(error)) => {
            return match budget(&error) {
                Some(budget) => inconclusive(budget),
                None => json!({ "verdict": "reject", "class": "header" }),
            };
        }
        Ok(None) => {}
    }
    match guarded(|| IDLArgs::from_bytes_with_config(bytes, &config(quota))) {
        Err(()) => json!({ "verdict": "panic" }),
        Ok(Ok(_)) => json!({ "verdict": "reject", "class": "coercion" }),
        Ok(Err(error)) => match budget(&error) {
            Some(budget) => inconclusive(budget),
            None => json!({ "verdict": "reject", "class": "malformed" }),
        },
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
