use crate::limits::Limits;
use crate::model::{Contract, PrimitiveType, TypeNode, TypeRef};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTypeRef {
    pub contract_id: String,
    pub type_ref: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractMethodRef {
    pub contract_id: String,
    pub method_name: String,
}

/// One field of a [`HostValue`] record, addressed by its authoritative Candid
/// field ID.
///
/// The fields are private so a record entry cannot be assembled by struct
/// literal, which would let a caller reach [`HostValue::record`] with a value
/// whose extent was never measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostFieldValue {
    id: u32,
    value: HostValue,
}

impl HostFieldValue {
    /// Pair a field ID with a value.
    ///
    /// This is deliberately infallible: a field carries no bound of its own,
    /// and the enclosing [`HostValue::record`] call is what measures the
    /// combined extent against the caller's [`Limits`].
    pub fn new(id: u32, value: HostValue) -> Self {
        Self { id, value }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn value(&self) -> &HostValue {
        &self.value
    }
}

/// A locally canonical tagged HostValue.
///
/// This type serializes as the portable tagged JSON ABI, but deliberately does
/// not implement `Deserialize`. JSON callers must use
/// [`HostValue::from_json_with_limits`], which decodes a private raw DTO and
/// checks locally canonical scalar encodings before exposing this value.
///
/// Every value carries its own [`Extent`], so the container constructors can
/// reject an over-deep or over-large value in constant time per level. That
/// bound is what makes the recursive operations on this type safe: `Drop`,
/// `Clone`, `PartialEq`, `Debug`, and `Serialize` all recurse once per level,
/// and all of them abort the process on a value deep enough to exhaust the
/// stack. Bounding construction is the only chokepoint that covers all five,
/// so there is deliberately no way to obtain a `HostValue` whose depth was
/// never checked against a caller-supplied limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostValue {
    kind: HostValueKind,
    extent: Extent,
}

/// The measured size of a value: how deeply it nests and how many nodes it
/// contains.
///
/// Both counters use the same units as the JSON decode path in
/// [`HostValueLocalValidationState::canonicalize_value`], so one value reports
/// the same `observed` figure whether it was rejected while being decoded or
/// while being constructed. `depth` counts container edges below a node, so a
/// scalar and an empty container are both `0`; `elements` counts nodes
/// inclusive of the node itself, so a scalar is `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    depth: u32,
    elements: u32,
}

impl Extent {
    const LEAF: Self = Self {
        depth: 0,
        elements: 1,
    };

    fn of(kind: &HostValueKind) -> Self {
        match kind {
            HostValueKind::Opt { value: Some(value) } => Self::enclosing([value.extent]),
            HostValueKind::Vec { values } => {
                Self::enclosing(values.iter().map(|value| value.extent))
            }
            HostValueKind::Record { fields } => {
                Self::enclosing(fields.iter().map(|field| field.value.extent))
            }
            HostValueKind::Variant { value, .. } => Self::enclosing([value.extent]),
            _ => Self::LEAF,
        }
    }

    /// An empty container yields depth `0`, not `1`: it encloses nothing, so it
    /// adds no edge, exactly as the decode path charges it no child recursion.
    fn enclosing(children: impl IntoIterator<Item = Self>) -> Self {
        let mut deepest: Option<u32> = None;
        let mut elements = 1u32;
        for child in children {
            deepest = Some(deepest.map_or(child.depth, |depth| depth.max(child.depth)));
            elements = elements.saturating_add(child.elements);
        }
        Self {
            depth: deepest.map_or(0, |depth| depth.saturating_add(1)),
            elements,
        }
    }
}

// Hand-written so the wire shape stays exactly what `#[serde(transparent)]`
// produced before `HostValue` gained its cached extent: the tagged ABI of
// ADR 0006, with no envelope and no extent field.
impl Serialize for HostValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.kind.serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum HostValueKind {
    Null,
    Bool { value: bool },
    Nat { value: String },
    Int { value: String },
    Nat8 { value: u8 },
    Nat16 { value: u16 },
    Nat32 { value: u32 },
    Nat64 { value: String },
    Int8 { value: i8 },
    Int16 { value: i16 },
    Int32 { value: i32 },
    Int64 { value: String },
    Float32 { bits: String },
    Float64 { bits: String },
    Text { value: String },
    Reserved,
    Principal { value: String },
    Opt { value: Option<Box<HostValue>> },
    Vec { values: Vec<HostValue> },
    Record { fields: Vec<HostFieldValue> },
    Variant { id: u32, value: Box<HostValue> },
    Service { principal: String },
    Func { principal: String, method: String },
}

impl HostValue {
    pub fn from_json_with_limits(input: &str, limits: &Limits) -> Result<Self, HostValueJsonError> {
        Self::from_json_with_context(input, &crate::RuntimeContext::new(limits.clone()))
    }

    pub fn from_json_with_context(
        input: &str,
        context: &crate::RuntimeContext,
    ) -> Result<Self, HostValueJsonError> {
        let limits = &context.limits;
        if input.len() > limits.max_value_bytes {
            return Err(HostValueJsonError::Limit {
                limit: limits.max_value_bytes,
                observed: input.len(),
            });
        }
        let mut budget = context.budget();
        budget
            .checkpoint()
            .map_err(|error| host_json_budget_error(error, "$"))?;
        check_value_nesting(input, &mut budget)?;
        let raw: RawHostValue = serde_json::from_str(input)
            .map_err(|error| HostValueJsonError::Malformed(error.to_string()))?;
        budget
            .checkpoint()
            .map_err(|error| host_json_budget_error(error, "$"))?;
        HostValueLocalValidationState::new(&mut budget).canonicalize(raw)
    }

    pub fn null() -> Self {
        Self::scalar(HostValueKind::Null)
    }

    pub fn boolean(value: bool) -> Self {
        Self::scalar(HostValueKind::Bool { value })
    }

    pub fn nat(value: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let value = value.into();
        Self::require(canonical_nat(&value), "non-canonical nat")?;
        Ok(Self::scalar(HostValueKind::Nat { value }))
    }

    pub fn int(value: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let value = value.into();
        Self::require(canonical_int(&value), "non-canonical int")?;
        Ok(Self::scalar(HostValueKind::Int { value }))
    }

    pub fn nat8(value: u8) -> Self {
        Self::scalar(HostValueKind::Nat8 { value })
    }

    pub fn nat16(value: u16) -> Self {
        Self::scalar(HostValueKind::Nat16 { value })
    }

    pub fn nat32(value: u32) -> Self {
        Self::scalar(HostValueKind::Nat32 { value })
    }

    pub fn nat64(value: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let value = value.into();
        Self::require(
            canonical_nat(&value) && value.parse::<u64>().is_ok(),
            "non-canonical nat64",
        )?;
        Ok(Self::scalar(HostValueKind::Nat64 { value }))
    }

    pub fn int8(value: i8) -> Self {
        Self::scalar(HostValueKind::Int8 { value })
    }

    pub fn int16(value: i16) -> Self {
        Self::scalar(HostValueKind::Int16 { value })
    }

    pub fn int32(value: i32) -> Self {
        Self::scalar(HostValueKind::Int32 { value })
    }

    pub fn int64(value: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let value = value.into();
        Self::require(
            canonical_int(&value) && value.parse::<i64>().is_ok(),
            "non-canonical int64",
        )?;
        Ok(Self::scalar(HostValueKind::Int64 { value }))
    }

    pub fn float32(bits: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let bits = bits.into();
        Self::require(canonical_hex(&bits, 8), "non-canonical float32 bits")?;
        Ok(Self::scalar(HostValueKind::Float32 { bits }))
    }

    pub fn float64(bits: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let bits = bits.into();
        Self::require(canonical_hex(&bits, 16), "non-canonical float64 bits")?;
        Ok(Self::scalar(HostValueKind::Float64 { bits }))
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self::scalar(HostValueKind::Text {
            value: value.into(),
        })
    }

    pub fn reserved() -> Self {
        Self::scalar(HostValueKind::Reserved)
    }

    pub fn principal(value: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let value = value.into();
        Self::require_canonical_principal(&value)?;
        Ok(Self::scalar(HostValueKind::Principal { value }))
    }

    /// Wrap a value in an `opt`, failing closed past `limits.max_value_depth`
    /// or `limits.max_value_elements`.
    ///
    /// Container construction is fallible because every recursive operation on
    /// the result — including `Drop`, `Clone`, and `Debug`, none of which can
    /// signal failure — walks one stack frame per level. Refusing to build the
    /// value is the only point at which a policy can still be applied.
    pub fn opt(value: Option<Self>, limits: &Limits) -> Result<Self, HostValueJsonError> {
        Self::bounded(
            HostValueKind::Opt {
                value: value.map(Box::new),
            },
            limits,
        )
    }

    /// Build a `vec`, failing closed past `limits.max_value_depth` or
    /// `limits.max_value_elements`.
    pub fn vector(values: Vec<Self>, limits: &Limits) -> Result<Self, HostValueJsonError> {
        Self::bounded(HostValueKind::Vec { values }, limits)
    }

    /// Build a `record`, failing closed past `limits.max_value_depth` or
    /// `limits.max_value_elements`.
    ///
    /// This does not check for duplicate field IDs or agreement with any
    /// Contract type; that remains [`validate_host_value`]'s contract-directed
    /// job.
    pub fn record(
        fields: Vec<HostFieldValue>,
        limits: &Limits,
    ) -> Result<Self, HostValueJsonError> {
        Self::bounded(HostValueKind::Record { fields }, limits)
    }

    /// Build a `variant`, failing closed past `limits.max_value_depth` or
    /// `limits.max_value_elements`.
    pub fn variant(id: u32, value: Self, limits: &Limits) -> Result<Self, HostValueJsonError> {
        Self::bounded(
            HostValueKind::Variant {
                id,
                value: Box::new(value),
            },
            limits,
        )
    }

    pub fn service(principal: impl Into<String>) -> Result<Self, HostValueJsonError> {
        let principal = principal.into();
        Self::require_canonical_principal(&principal)?;
        Ok(Self::scalar(HostValueKind::Service { principal }))
    }

    pub fn func(
        principal: impl Into<String>,
        method: impl Into<String>,
    ) -> Result<Self, HostValueJsonError> {
        let principal = principal.into();
        Self::require_canonical_principal(&principal)?;
        Ok(Self::scalar(HostValueKind::Func {
            principal,
            method: method.into(),
        }))
    }

    /// Wrap a kind that encloses nothing, so its extent needs no measuring.
    fn scalar(kind: HostValueKind) -> Self {
        debug_assert!(
            Extent::of(&kind) == Extent::LEAF,
            "scalar() used for a kind that encloses children",
        );
        Self {
            kind,
            extent: Extent::LEAF,
        }
    }

    /// Wrap a kind whose children were already bounded by the caller.
    ///
    /// Used only by the JSON decode path, which enforces depth and elements
    /// through the budget as it walks, so re-checking here would report the
    /// same exhaustion twice under two different code paths.
    fn measured(kind: HostValueKind) -> Self {
        let extent = Extent::of(&kind);
        Self { kind, extent }
    }

    /// Measure an enclosing kind and reject it if it breaches the caller's
    /// policy.
    ///
    /// Constant time per level: each child already carries its own extent, so a
    /// chain built one level at a time costs O(depth) overall rather than
    /// O(depth^2).
    fn bounded(kind: HostValueKind, limits: &Limits) -> Result<Self, HostValueJsonError> {
        let extent = Extent::of(&kind);
        if usize::try_from(extent.depth).unwrap_or(usize::MAX) > limits.max_value_depth {
            return Err(HostValueJsonError::ValueLimit {
                resource: "value_depth",
                limit: limits.max_value_depth,
                observed: usize::try_from(extent.depth).unwrap_or(usize::MAX),
                path: "$".to_string(),
            });
        }
        if usize::try_from(extent.elements).unwrap_or(usize::MAX) > limits.max_value_elements {
            return Err(HostValueJsonError::ValueLimit {
                resource: "value_elements",
                limit: limits.max_value_elements,
                observed: usize::try_from(extent.elements).unwrap_or(usize::MAX),
                path: "$".to_string(),
            });
        }
        Ok(Self { kind, extent })
    }

    fn require(condition: bool, message: &str) -> Result<(), HostValueJsonError> {
        if condition {
            Ok(())
        } else {
            Err(HostValueJsonError::Malformed(format!("$: {message}")))
        }
    }

    fn require_canonical_principal(value: &str) -> Result<(), HostValueJsonError> {
        let principal = candid_parser::Principal::from_text(value).map_err(|error| {
            HostValueJsonError::Malformed(format!("$: invalid principal {value:?}: {error}"))
        })?;
        Self::require(principal.to_text() == value, "non-canonical principal")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostValueJsonError {
    Malformed(String),
    Limit {
        limit: usize,
        observed: usize,
    },
    ValueLimit {
        resource: &'static str,
        limit: usize,
        observed: usize,
        path: String,
    },
    Deadline {
        path: String,
    },
    Cancelled {
        path: String,
    },
}

impl fmt::Display for HostValueJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(message) => write!(formatter, "malformed HostValue JSON: {message}"),
            Self::Limit { limit, observed } => write!(
                formatter,
                "HostValue JSON uses {observed} bytes; limit is {limit}"
            ),
            Self::ValueLimit {
                resource,
                limit,
                observed,
                path,
            } => write!(
                formatter,
                "HostValue JSON at {path} uses {observed} {resource}; limit is {limit}"
            ),
            Self::Deadline { path } => {
                write!(
                    formatter,
                    "HostValue JSON validation deadline elapsed at {path}"
                )
            }
            Self::Cancelled { path } => {
                write!(formatter, "HostValue JSON validation cancelled at {path}")
            }
        }
    }
}

impl std::error::Error for HostValueJsonError {}

fn host_json_budget_error(error: crate::budget::BudgetError, path: &str) -> HostValueJsonError {
    match error {
        crate::budget::BudgetError::Cancelled => HostValueJsonError::Cancelled {
            path: path.to_string(),
        },
        crate::budget::BudgetError::DeadlineExceeded => HostValueJsonError::Deadline {
            path: path.to_string(),
        },
        crate::budget::BudgetError::ResourceLimit {
            resource,
            limit,
            observed,
        } => HostValueJsonError::ValueLimit {
            resource,
            limit,
            observed,
            path: path.to_string(),
        },
    }
}

/// Poll cancellation every this many input bytes rather than every byte, so a
/// scan of a `max_value_bytes`-sized document stays responsive without paying
/// an atomic load per byte.
const NESTING_CHECKPOINT_INTERVAL: usize = 4096;

/// Reject stack-hostile JSON nesting before `serde_json`'s recursive decoder
/// sees it.
///
/// This is the HostValue analogue of `check_source_nesting`: an iterative,
/// constant-stack scan that runs ahead of a recursive parser so hostile nesting
/// is rejected by a crate-owned budget check instead of by exhausting the
/// stack. `serde_json` counts recursion per JSON container too, so `observed`
/// here is directly comparable to its fixed 128-frame ceiling; keeping
/// `max_value_nesting` below that ceiling means this check always fires first
/// while the ceiling remains in place as an unmodified second line of defence.
///
/// The scan deliberately does not validate JSON syntax. Unbalanced or malformed
/// input keeps `serde_json`'s established diagnostic rather than gaining a
/// second, competing one.
///
/// Scanning raw bytes is sound for UTF-8: a multi-byte sequence never contains
/// an ASCII byte, so no `{`, `[`, `"`, or `\` can be mistaken for structure
/// inside a multi-byte character.
fn check_value_nesting(
    input: &str,
    budget: &mut crate::budget::Budget<'_>,
) -> Result<(), HostValueJsonError> {
    let limit = budget.limits().max_value_nesting;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    // A comparison against a running target rather than `index % INTERVAL`:
    // this loop runs once per input byte, and a division there is a measurable
    // fraction of the whole scan.
    let mut next_checkpoint = 0usize;

    for (index, byte) in input.as_bytes().iter().enumerate() {
        if index == next_checkpoint {
            budget
                .checkpoint()
                .map_err(|error| host_json_budget_error(error, "$"))?;
            next_checkpoint = index.saturating_add(NESTING_CHECKPOINT_INTERVAL);
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if depth > limit {
                    return Err(HostValueJsonError::ValueLimit {
                        resource: "value_nesting",
                        limit,
                        observed: depth,
                        path: "$".to_string(),
                    });
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostFieldValue {
    id: u32,
    value: RawHostValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawHostValue {
    Null,
    Bool { value: bool },
    Nat { value: String },
    Int { value: String },
    Nat8 { value: u8 },
    Nat16 { value: u16 },
    Nat32 { value: u32 },
    Nat64 { value: String },
    Int8 { value: i8 },
    Int16 { value: i16 },
    Int32 { value: i32 },
    Int64 { value: String },
    Float32 { bits: String },
    Float64 { bits: String },
    Text { value: String },
    Reserved,
    Principal { value: String },
    Opt { value: Option<Box<RawHostValue>> },
    Vec { values: Vec<RawHostValue> },
    Record { fields: Vec<RawHostFieldValue> },
    Variant { id: u32, value: Box<RawHostValue> },
    Service { principal: String },
    Func { principal: String, method: String },
}

struct HostValueLocalValidationState<'a, 'limits> {
    budget: &'a mut crate::budget::Budget<'limits>,
}

impl<'a, 'limits> HostValueLocalValidationState<'a, 'limits> {
    fn new(budget: &'a mut crate::budget::Budget<'limits>) -> Self {
        Self { budget }
    }

    fn canonicalize(mut self, raw: RawHostValue) -> Result<HostValue, HostValueJsonError> {
        self.canonicalize_value(raw, "$", 0)
    }

    fn canonicalize_value(
        &mut self,
        raw: RawHostValue,
        path: &str,
        depth: usize,
    ) -> Result<HostValue, HostValueJsonError> {
        self.budget
            .checkpoint()
            .map_err(|error| host_json_budget_error(error, path))?;
        let limits = self.budget.limits().clone();
        if depth > limits.max_value_depth {
            return Err(HostValueJsonError::ValueLimit {
                resource: "value_depth",
                limit: limits.max_value_depth,
                observed: depth,
                path: path.to_string(),
            });
        }
        self.budget
            .charge("value_elements", limits.max_value_elements, 1)
            .map_err(|error| host_json_budget_error(error, path))?;
        self.budget
            .charge("canonicalization_work", limits.max_canonicalization_work, 1)
            .map_err(|error| host_json_budget_error(error, path))?;

        let value = match raw {
            RawHostValue::Null => HostValueKind::Null,
            RawHostValue::Bool { value } => HostValueKind::Bool { value },
            RawHostValue::Nat { value } => {
                self.charge(&value, path)?;
                self.require(canonical_nat(&value), path, "non-canonical nat")?;
                HostValueKind::Nat { value }
            }
            RawHostValue::Int { value } => {
                self.charge(&value, path)?;
                self.require(canonical_int(&value), path, "non-canonical int")?;
                HostValueKind::Int { value }
            }
            RawHostValue::Nat8 { value } => HostValueKind::Nat8 { value },
            RawHostValue::Nat16 { value } => HostValueKind::Nat16 { value },
            RawHostValue::Nat32 { value } => HostValueKind::Nat32 { value },
            RawHostValue::Nat64 { value } => {
                self.charge(&value, path)?;
                self.require(
                    canonical_nat(&value) && value.parse::<u64>().is_ok(),
                    path,
                    "non-canonical nat64",
                )?;
                HostValueKind::Nat64 { value }
            }
            RawHostValue::Int8 { value } => HostValueKind::Int8 { value },
            RawHostValue::Int16 { value } => HostValueKind::Int16 { value },
            RawHostValue::Int32 { value } => HostValueKind::Int32 { value },
            RawHostValue::Int64 { value } => {
                self.charge(&value, path)?;
                self.require(
                    canonical_int(&value) && value.parse::<i64>().is_ok(),
                    path,
                    "non-canonical int64",
                )?;
                HostValueKind::Int64 { value }
            }
            RawHostValue::Float32 { bits } => {
                self.charge(&bits, path)?;
                self.require(canonical_hex(&bits, 8), path, "non-canonical float32 bits")?;
                HostValueKind::Float32 { bits }
            }
            RawHostValue::Float64 { bits } => {
                self.charge(&bits, path)?;
                self.require(canonical_hex(&bits, 16), path, "non-canonical float64 bits")?;
                HostValueKind::Float64 { bits }
            }
            RawHostValue::Text { value } => {
                self.charge(&value, path)?;
                HostValueKind::Text { value }
            }
            RawHostValue::Reserved => HostValueKind::Reserved,
            RawHostValue::Principal { value } => {
                self.charge(&value, path)?;
                self.require_canonical_principal(&value, path)?;
                HostValueKind::Principal { value }
            }
            RawHostValue::Opt { value } => HostValueKind::Opt {
                value: value
                    .map(|value| {
                        self.canonicalize_value(*value, &format!("{path}.value"), depth + 1)
                    })
                    .transpose()?
                    .map(Box::new),
            },
            RawHostValue::Vec { values } => HostValueKind::Vec {
                values: values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.canonicalize_value(
                            value,
                            &format!("{path}.values[{index}]"),
                            depth + 1,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            RawHostValue::Record { fields } => HostValueKind::Record {
                fields: fields
                    .into_iter()
                    .map(|field| {
                        Ok(HostFieldValue::new(
                            field.id,
                            self.canonicalize_value(
                                field.value,
                                &format!("{path}.fields[{}]", field.id),
                                depth + 1,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, HostValueJsonError>>()?,
            },
            RawHostValue::Variant { id, value } => HostValueKind::Variant {
                id,
                value: Box::new(self.canonicalize_value(
                    *value,
                    &format!("{path}.value"),
                    depth + 1,
                )?),
            },
            RawHostValue::Service { principal } => {
                self.charge(&principal, path)?;
                self.require_canonical_principal(&principal, path)?;
                HostValueKind::Service { principal }
            }
            RawHostValue::Func { principal, method } => {
                self.charge(&principal, path)?;
                self.charge(&method, path)?;
                self.require_canonical_principal(&principal, path)?;
                HostValueKind::Func { principal, method }
            }
        };
        Ok(HostValue::measured(value))
    }

    fn charge(&mut self, value: &str, path: &str) -> Result<(), HostValueJsonError> {
        let limit = self.budget.limits().max_value_bytes;
        self.budget
            .charge("value_bytes", limit, value.len())
            .map_err(|error| host_json_budget_error(error, path))?;
        Ok(())
    }

    fn require(
        &self,
        condition: bool,
        path: &str,
        message: &str,
    ) -> Result<(), HostValueJsonError> {
        if condition {
            Ok(())
        } else {
            Err(HostValueJsonError::Malformed(format!("{path}: {message}")))
        }
    }

    fn require_canonical_principal(
        &self,
        value: &str,
        path: &str,
    ) -> Result<(), HostValueJsonError> {
        let principal = candid_parser::Principal::from_text(value).map_err(|error| {
            HostValueJsonError::Malformed(format!("{path}: invalid principal {value:?}: {error}"))
        })?;
        self.require(
            principal.to_text() == value,
            path,
            "non-canonical principal",
        )
    }
}

/// Compatibility name for the shared diagnostic item in the HostValue
/// validation domain.
///
/// HostValue violations are [`crate::Diagnostic`] values that always carry
/// `path` and never carry `phase`/`severity`, so their serialized shape is
/// unchanged: `{code, path, message, resource_limit?}`.
pub type HostValueViolation = crate::Diagnostic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostValueValidationError {
    pub violations: Vec<HostValueViolation>,
}

impl fmt::Display for HostValueValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "HostValue validation failed with {} violation(s)",
            self.violations.len()
        )
    }
}

impl std::error::Error for HostValueValidationError {}

pub fn validate_host_value(
    contract: &Contract,
    selector: &ContractTypeRef,
    value: &HostValue,
    limits: &Limits,
) -> Result<(), HostValueValidationError> {
    validate_host_value_with_context(
        contract,
        selector,
        value,
        &crate::RuntimeContext::new(limits.clone()),
    )
}

pub fn validate_host_value_with_context(
    contract: &Contract,
    selector: &ContractTypeRef,
    value: &HostValue,
    context: &crate::RuntimeContext,
) -> Result<(), HostValueValidationError> {
    if selector.contract_id != contract.contract_id() {
        return Err(single(
            "value_contract_id_mismatch",
            "$",
            format!(
                "expected Contract {}, found {}",
                contract.contract_id(),
                selector.contract_id
            ),
        ));
    }
    if selector.type_ref as usize >= contract.types().len() {
        return Err(single(
            "value_type_ref_out_of_bounds",
            "$",
            format!(
                "type reference {} is outside the Contract",
                selector.type_ref
            ),
        ));
    }

    let mut budget = context.budget();
    let mut state = HostValueValidationState {
        contract,
        budget: &mut budget,
    };
    state.validate_node(selector.type_ref, value, "$", 0)
}

struct HostValueValidationState<'a, 'budget, 'limits> {
    contract: &'a Contract,
    budget: &'budget mut crate::budget::Budget<'limits>,
}

impl HostValueValidationState<'_, '_, '_> {
    fn validate_node(
        &mut self,
        reference: TypeRef,
        value: &HostValue,
        path: &str,
        depth: usize,
    ) -> Result<(), HostValueValidationError> {
        self.budget
            .checkpoint()
            .map_err(|error| host_validation_budget_error(error, path))?;
        let limits = self.budget.limits().clone();
        if depth > limits.max_value_depth {
            return Err(resource_single(
                "value_depth",
                limits.max_value_depth,
                depth,
                path,
                format!("value depth exceeds limit {}", limits.max_value_depth),
            ));
        }

        self.charge_element(path)?;
        self.charge_string_bytes(value, path)?;

        match (&self.contract.types()[reference as usize], &value.kind) {
            (TypeNode::Primitive { primitive }, value) => {
                validate_primitive(*primitive, value, path)?;
            }
            (TypeNode::Opt { inner }, HostValueKind::Opt { value }) => {
                if let Some(value) = value {
                    self.preflight_children(1, path)?;
                    let child_path = format!("{path}.value");
                    self.validate_node(*inner, value, &child_path, depth + 1)?;
                }
            }
            (TypeNode::Vec { inner }, HostValueKind::Vec { values }) => {
                self.preflight_children(values.len(), path)?;
                for (index, value) in values.iter().enumerate() {
                    let child_path = format!("{path}.values[{index}]");
                    self.validate_node(*inner, value, &child_path, depth + 1)?;
                }
            }
            (TypeNode::Record { fields }, HostValueKind::Record { fields: values }) => {
                self.preflight_children(values.len(), path)?;
                for (index, field) in values.iter().enumerate() {
                    for other in &values[index + 1..] {
                        self.charge_work(path)?;
                        if other.id == field.id {
                            return Err(single(
                                "duplicate_host_field",
                                path,
                                format!("record field ID {} occurs more than once", field.id),
                            ));
                        }
                    }
                }
                let mut field_set_matches = fields.len() == values.len();
                if field_set_matches {
                    'expected_fields: for field in fields {
                        for value in values {
                            self.charge_work(path)?;
                            if value.id == field.id {
                                continue 'expected_fields;
                            }
                        }
                        field_set_matches = false;
                        break;
                    }
                }
                if !field_set_matches {
                    let expected_ids =
                        self.sorted_field_ids(fields.len(), |index| fields[index].id, path)?;
                    let actual_ids =
                        self.sorted_field_ids(values.len(), |index| values[index].id, path)?;
                    return Err(single(
                        "record_field_set_mismatch",
                        path,
                        format!("expected field IDs {}, found {}", expected_ids, actual_ids),
                    ));
                }
                for field in fields {
                    let mut matching_value = None;
                    for value in values {
                        self.charge_work(path)?;
                        if value.id == field.id {
                            matching_value = Some(value);
                            break;
                        }
                    }
                    let value = matching_value.expect("record field set was checked above");
                    let child_path = format!("{path}.fields[{}]", field.id);
                    self.validate_node(field.ty, &value.value, &child_path, depth + 1)?;
                }
            }
            (TypeNode::Variant { fields }, HostValueKind::Variant { id, value }) => {
                // Charge each comparison, exactly like the record arm above.
                // A variant type may hold up to `max_fields` entries, and a
                // `vec variant` value can present one lookup per element, so an
                // uncharged `fields.iter().find(...)` is `O(elements * fields)`
                // of free, uninterruptible scan work. Charging bounds it to
                // `max_canonicalization_work` and lets a deadline interrupt it.
                let mut matched = None;
                for field in fields {
                    self.charge_work(path)?;
                    if field.id == *id {
                        matched = Some(field);
                        break;
                    }
                }
                let Some(field) = matched else {
                    return Err(single(
                        "unknown_variant_id",
                        path,
                        format!("variant ID {id} does not exist in the expected type"),
                    ));
                };
                self.preflight_children(1, path)?;
                let child_path = format!("{path}.value");
                self.validate_node(field.ty, value, &child_path, depth + 1)?;
            }
            (TypeNode::Service { .. }, HostValueKind::Service { principal }) => {
                validate_principal(principal, path)?;
            }
            (TypeNode::Func { .. }, HostValueKind::Func { principal, method }) => {
                validate_principal(principal, path)?;
                if method.is_empty() {
                    return Err(single(
                        "empty_function_method",
                        path,
                        "function method names must not be empty",
                    ));
                }
            }
            (TypeNode::Class { .. }, _) => {
                return Err(single(
                    "class_has_no_host_value",
                    path,
                    "service constructors are not first-class Candid values",
                ));
            }
            (expected, actual) => {
                return Err(single(
                    "host_value_kind_mismatch",
                    path,
                    format!(
                        "expected {}, found {}",
                        type_node_kind(expected),
                        host_value_kind(actual)
                    ),
                ));
            }
        }
        Ok(())
    }

    fn charge_element(&mut self, path: &str) -> Result<(), HostValueValidationError> {
        let limit = self.budget.limits().max_value_elements;
        self.budget
            .charge("value_elements", limit, 1)
            .map_err(|error| host_validation_budget_error(error, path))?;
        Ok(())
    }

    fn check_deadline(&self, path: &str) -> Result<(), HostValueValidationError> {
        self.budget
            .checkpoint()
            .map_err(|error| host_validation_budget_error(error, path))
    }

    fn charge_work(&mut self, path: &str) -> Result<(), HostValueValidationError> {
        self.check_deadline(path)?;
        let limit = self.budget.limits().max_canonicalization_work;
        self.budget
            .charge("canonicalization_work", limit, 1)
            .map_err(|error| host_validation_budget_error(error, path))?;
        Ok(())
    }

    fn preflight_children(
        &mut self,
        child_count: usize,
        path: &str,
    ) -> Result<(), HostValueValidationError> {
        let limit = self.budget.limits().max_value_elements;
        let observed = self
            .budget
            .consumed("value_elements")
            .saturating_add(child_count);
        if observed > limit {
            return Err(resource_single(
                "value_elements",
                limit,
                observed,
                path,
                format!("value elements exceed limit {limit}"),
            ));
        }
        Ok(())
    }

    fn charge_string_bytes(
        &mut self,
        value: &HostValue,
        path: &str,
    ) -> Result<(), HostValueValidationError> {
        let limit = self.budget.limits().max_value_bytes;
        self.budget
            .charge("value_bytes", limit, value_string_bytes(value))
            .map_err(|error| host_validation_budget_error(error, path))?;
        Ok(())
    }

    fn sorted_field_ids(
        &mut self,
        length: usize,
        id_at: impl Fn(usize) -> u32,
        path: &str,
    ) -> Result<String, HostValueValidationError> {
        let mut output = String::from("[");
        let mut previous = None;
        for position in 0..length {
            let mut next = None;
            for index in 0..length {
                self.charge_work(path)?;
                let id = id_at(index);
                let after_previous = previous.map_or(true, |previous| id > previous);
                let before_next = next.map_or(true, |next| id < next);
                if after_previous && before_next {
                    next = Some(id);
                }
            }
            let Some(id) = next else {
                break;
            };
            if position > 0 {
                output.push_str(", ");
            }
            output.push_str(&id.to_string());
            previous = Some(id);
        }
        output.push(']');
        Ok(output)
    }
}

fn validate_primitive(
    primitive: PrimitiveType,
    value: &HostValueKind,
    path: &str,
) -> Result<(), HostValueValidationError> {
    let valid = match (primitive, value) {
        (PrimitiveType::Null, HostValueKind::Null)
        | (PrimitiveType::Bool, HostValueKind::Bool { .. })
        | (PrimitiveType::Nat8, HostValueKind::Nat8 { .. })
        | (PrimitiveType::Nat16, HostValueKind::Nat16 { .. })
        | (PrimitiveType::Nat32, HostValueKind::Nat32 { .. })
        | (PrimitiveType::Int8, HostValueKind::Int8 { .. })
        | (PrimitiveType::Int16, HostValueKind::Int16 { .. })
        | (PrimitiveType::Int32, HostValueKind::Int32 { .. })
        | (PrimitiveType::Reserved, HostValueKind::Reserved) => true,
        (PrimitiveType::Nat, HostValueKind::Nat { value }) => canonical_nat(value),
        (PrimitiveType::Int, HostValueKind::Int { value }) => canonical_int(value),
        (PrimitiveType::Nat64, HostValueKind::Nat64 { value }) => {
            canonical_nat(value) && value.parse::<u64>().is_ok()
        }
        (PrimitiveType::Int64, HostValueKind::Int64 { value }) => {
            canonical_int(value) && value.parse::<i64>().is_ok()
        }
        (PrimitiveType::Float32, HostValueKind::Float32 { bits }) => canonical_hex(bits, 8),
        (PrimitiveType::Float64, HostValueKind::Float64 { bits }) => canonical_hex(bits, 16),
        (PrimitiveType::Text, HostValueKind::Text { .. }) => true,
        (PrimitiveType::Principal, HostValueKind::Principal { value }) => {
            validate_principal(value, path)?;
            true
        }
        (PrimitiveType::Empty, _) => {
            return Err(single(
                "empty_has_no_value",
                path,
                "the Candid empty type has no constructible HostValue",
            ));
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(single(
            "host_value_kind_mismatch",
            path,
            format!(
                "expected primitive {primitive:?}, found {} or a non-canonical representation",
                host_value_kind(value)
            ),
        ))
    }
}

fn value_string_bytes(value: &HostValue) -> usize {
    match &value.kind {
        HostValueKind::Nat { value }
        | HostValueKind::Int { value }
        | HostValueKind::Nat64 { value }
        | HostValueKind::Int64 { value }
        | HostValueKind::Text { value }
        | HostValueKind::Principal { value } => value.len(),
        HostValueKind::Float32 { bits } | HostValueKind::Float64 { bits } => bits.len(),
        HostValueKind::Service { principal } => principal.len(),
        HostValueKind::Func { principal, method } => principal.len().saturating_add(method.len()),
        _ => 0,
    }
}

fn canonical_nat(value: &str) -> bool {
    value == "0"
        || (!value.starts_with('0')
            && !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn canonical_int(value: &str) -> bool {
    if let Some(magnitude) = value.strip_prefix('-') {
        magnitude != "0" && canonical_nat(magnitude)
    } else {
        canonical_nat(value)
    }
}

fn canonical_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_principal(value: &str, path: &str) -> Result<(), HostValueValidationError> {
    let principal = candid_parser::Principal::from_text(value).map_err(|error| {
        single(
            "invalid_principal",
            path,
            format!("invalid principal {value:?}: {error}"),
        )
    })?;
    if principal.to_text() != value {
        return Err(single(
            "invalid_principal",
            path,
            format!("principal {value:?} is not in canonical textual form"),
        ));
    }
    Ok(())
}

fn single(
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) -> HostValueValidationError {
    HostValueValidationError {
        violations: vec![crate::Diagnostic::violation(code, path, message)],
    }
}

fn resource_single(
    resource: &str,
    limit: usize,
    observed: usize,
    path: impl Into<String>,
    message: impl Into<String>,
) -> HostValueValidationError {
    HostValueValidationError {
        violations: vec![
            crate::Diagnostic::violation("resource_limit_exceeded", path, message)
                .with_resource_limit(crate::ResourceLimitInfo {
                    resource: resource.to_string(),
                    limit: crate::limits::portable_count(limit),
                    observed: crate::limits::portable_count(observed),
                }),
        ],
    }
}

fn host_validation_budget_error(
    error: crate::budget::BudgetError,
    path: &str,
) -> HostValueValidationError {
    match error {
        crate::budget::BudgetError::Cancelled => single(
            "operation_cancelled",
            path,
            "HostValue validation was cancelled",
        ),
        crate::budget::BudgetError::DeadlineExceeded => single(
            "operation_deadline_exceeded",
            path,
            "HostValue validation deadline has elapsed",
        ),
        crate::budget::BudgetError::ResourceLimit {
            resource,
            limit,
            observed,
        } => resource_single(
            resource,
            limit,
            observed,
            path,
            format!("resource {resource} exceeded limit {limit}; observed {observed}"),
        ),
    }
}

fn type_node_kind(node: &TypeNode) -> &'static str {
    match node {
        TypeNode::Primitive { .. } => "primitive",
        TypeNode::Opt { .. } => "opt",
        TypeNode::Vec { .. } => "vec",
        TypeNode::Record { .. } => "record",
        TypeNode::Variant { .. } => "variant",
        TypeNode::Func { .. } => "func",
        TypeNode::Service { .. } => "service",
        TypeNode::Class { .. } => "class",
    }
}

fn host_value_kind(value: &HostValueKind) -> &'static str {
    match value {
        HostValueKind::Null => "null",
        HostValueKind::Bool { .. } => "bool",
        HostValueKind::Nat { .. } => "nat",
        HostValueKind::Int { .. } => "int",
        HostValueKind::Nat8 { .. } => "nat8",
        HostValueKind::Nat16 { .. } => "nat16",
        HostValueKind::Nat32 { .. } => "nat32",
        HostValueKind::Nat64 { .. } => "nat64",
        HostValueKind::Int8 { .. } => "int8",
        HostValueKind::Int16 { .. } => "int16",
        HostValueKind::Int32 { .. } => "int32",
        HostValueKind::Int64 { .. } => "int64",
        HostValueKind::Float32 { .. } => "float32",
        HostValueKind::Float64 { .. } => "float64",
        HostValueKind::Text { .. } => "text",
        HostValueKind::Reserved => "reserved",
        HostValueKind::Principal { .. } => "principal",
        HostValueKind::Opt { .. } => "opt",
        HostValueKind::Vec { .. } => "vec",
        HostValueKind::Record { .. } => "record",
        HostValueKind::Variant { .. } => "variant",
        HostValueKind::Service { .. } => "service",
        HostValueKind::Func { .. } => "func",
    }
}

impl Contract {
    pub fn bind_type(
        &self,
        type_ref: TypeRef,
    ) -> Result<ContractTypeRef, HostValueValidationError> {
        if type_ref as usize >= self.types().len() {
            return Err(single(
                "value_type_ref_out_of_bounds",
                "$",
                format!("type reference {type_ref} is outside the Contract"),
            ));
        }
        Ok(ContractTypeRef {
            contract_id: self.contract_id().to_string(),
            type_ref,
        })
    }

    pub fn bind_method(
        &self,
        method: impl Into<String>,
    ) -> Result<ContractMethodRef, HostValueValidationError> {
        let method = method.into();
        let service = match self.actor() {
            Some(crate::model::Actor::Service { service }) => *service,
            Some(crate::model::Actor::Class { class }) => match &self.types()[*class as usize] {
                TypeNode::Class { service, .. } => *service,
                _ => unreachable!("validated class actor targets a class"),
            },
            None => {
                return Err(single(
                    "actorless_contract",
                    "$",
                    "an actorless Contract has no methods",
                ));
            }
        };
        match &self.types()[service as usize] {
            TypeNode::Service { methods } if methods.iter().any(|entry| entry.name == method) => {
                Ok(ContractMethodRef {
                    contract_id: self.contract_id().to_string(),
                    method_name: method,
                })
            }
            _ => Err(single(
                "unknown_method",
                "$",
                format!("method {method:?} does not exist in the actor service"),
            )),
        }
    }
}
