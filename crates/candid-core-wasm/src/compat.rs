//! The service-level compatibility check behind `checkCompatible`: is the
//! live service a Candid subtype of the written one, method by method?
//!
//! # The relation
//!
//! `live <: written` under the Candid spec's subtyping rules, evaluated on
//! the two canonical Contract graphs the compiler produced — no second type
//! representation, and no `candid` crate at run time. Every method the
//! written service declares must exist in the live one with the same mode,
//! its arguments contravariant (`written args <: live args`) and its results
//! covariant (`live results <: written results`), compared as tuples: an
//! extra trailing value on the sub side is ignored, a missing one is allowed
//! only where the super side's type is `null`, `reserved` or an `opt`.
//! Methods only the live service has are ignored, and a class's init
//! arguments play no part.
//!
//! # Soundness
//!
//! The relation is coinductive: a pair under test is assumed to hold when it
//! is met again, which is what makes recursive types terminate. Upstream
//! `candid` keeps that assumption set across a *failed* probe of an `opt`
//! position, so a pair proven under an assumption that later failed stays
//! "proven" and can answer a later, unrelated question (its unsound memo,
//! recorded as a known divergence in this crate's differential tests). Here
//! the verdict never depends on such a probe: `t <: opt t'` holds for every
//! `t` (the spec's special opt rule), so the relation itself has no failure
//! to absorb. The probe only decides whether the special rule was the one
//! that applied, which is reported as a warning, and it runs as a
//! transaction: when it fails, every pair it assumed or proved is rolled
//! back, together with every warning it recorded. A pair that fails is
//! cached as failed for the rest of the method; that is sound, because
//! assumptions only ever make more pairs hold. Each written method starts
//! from empty state — no assumption, no cached failure, no step spent — so
//! its diagnostics, paths and bounds included, never depend on which other
//! methods the written service declares.
//!
//! # Warnings
//!
//! A `special_opt_rule` warning is reported at every path where a value
//! decodes as `null`, not only the first: a pair proven once (say the two
//! sides of `type Memo = opt blob`, used by many fields) is not walked again,
//! but the warnings its walk recorded are re-reported under each further path
//! it is met at. The one exception is recursion: a pair met again while its
//! own walk is still in progress is a cycle, and its warnings are reported at
//! the paths of that first unfolding only.
//!
//! # Bounds
//!
//! The walk is recursive, so its depth is bounded ([`MAX_CHECK_DEPTH`] pairs
//! on one path), and so are its work ([`MAX_CHECK_STEPS`] pairs expanded)
//! and its warnings ([`MAX_CHECK_WARNINGS`]), each per method. Reaching any
//! of them fails the method closed with `resource_limit_exceeded`; the
//! compiler's own limits keep every Contract it accepts well below all three
//! for interfaces of ordinary shape. Because the bounds are per method, the
//! live side cannot multiply the work: a check costs at most the written
//! service's method count times one method's bounds.

use std::collections::{BTreeMap, HashMap, HashSet};

use candid_core::{
    Actor, Contract, Field, MethodMode, PrimitiveType, ServiceMethod, SourceInfo, SourceLabel,
    TypeNode,
};
use serde_json::{json, Value};

/// The most pairs one path of the walk may hold before the method fails
/// closed: one and a half times the compiler's Candid nesting bound of 256,
/// so every type the compiler accepts is decided when the two sides share
/// its shape, and only recursive types whose cycles differ in length on the
/// two sides can reach it. Measured on a native build at this bound: under
/// 1 MiB of stack unoptimized, and between 256 and 512 KiB optimized. The
/// wasm artifact reaches it under Node without exhausting its stack, which
/// the package's tests assert.
pub const MAX_CHECK_DEPTH: usize = 384;

/// The most pairs the check of one method may expand.
pub const MAX_CHECK_STEPS: usize = 1_000_000;

/// The most `special_opt_rule` warnings one method may report. Re-reporting a
/// proven pair's warnings under every path it is met at can multiply them
/// along a shared, non-recursive type graph (a record of two fields of a
/// record of two fields, and so on); this bound keeps that finite.
pub const MAX_CHECK_WARNINGS: usize = 1_000;

/// Which of the two Contracts a type reference belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Side {
    Live,
    Written,
}

impl Side {
    fn other(self) -> Self {
        match self {
            Side::Live => Side::Written,
            Side::Written => Side::Live,
        }
    }
}

/// One Contract's type arena plus the field names its source spelled.
struct Graph<'a> {
    types: &'a [TypeNode],
    names: BTreeMap<(u32, u32), &'a str>,
}

impl<'a> Graph<'a> {
    fn new(contract: &'a Contract, source_info: Option<&'a SourceInfo>) -> Self {
        let mut names = BTreeMap::new();
        if let Some(source_info) = source_info {
            for provenance in source_info.field_labels() {
                if let SourceLabel::Named { name } = &provenance.label {
                    names.insert((provenance.container, provenance.id), name.as_str());
                }
            }
        }
        Self {
            types: contract.types(),
            names,
        }
    }

    fn node(&self, reference: u32) -> &'a TypeNode {
        &self.types[reference as usize]
    }
}

/// One step of a path into a method's type, innermost first while a failure
/// travels up, reversed once when it is rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// A record field or variant arm: its source name, or its numeric id.
    Field(Option<String>, u32),
    /// The element type of a `vec`.
    Element,
    /// The content type of an `opt`.
    Opt,
    /// An argument of a func type (the method's own at the root).
    Argument(usize),
    /// A result of a func type (the method's own at the root).
    Result(usize),
    /// A method of a service type.
    Method(String),
}

/// Why a pair does not hold, relative to that pair.
#[derive(Debug, Clone)]
struct Failure {
    /// Innermost first.
    segments: Vec<Segment>,
    message: String,
}

/// The walk could not finish within its bounds.
#[derive(Debug, Clone)]
struct Exhausted {
    resource: &'static str,
    limit: usize,
    observed: usize,
}

enum Stop {
    Fails(Failure),
    Exhausted(Exhausted),
}

type Outcome = Result<(), Stop>;

fn fails(message: impl Into<String>) -> Outcome {
    Err(Stop::Fails(Failure {
        segments: Vec::new(),
        message: message.into(),
    }))
}

/// Prepend a segment to a failure travelling up out of a child pair.
fn within(segment: Segment, outcome: Outcome) -> Outcome {
    match outcome {
        Err(Stop::Fails(mut failure)) => {
            failure.segments.push(segment);
            Err(Stop::Fails(failure))
        }
        other => other,
    }
}

/// A special-opt-rule application, recorded where the walk met it.
#[derive(Debug, Clone)]
struct Warning {
    /// Outermost first, relative to the method.
    path: Vec<Segment>,
    message: String,
}

/// `(sub side, sub reference, super reference)`: the super side is always
/// the other Contract.
type Pair = (Side, u32, u32);

/// The warnings a proven pair's walk recorded: `warnings[start..end]`, every
/// one of whose paths begins with the pair's own path of `prefix` segments.
#[derive(Debug, Clone, Copy)]
struct Proven {
    start: usize,
    end: usize,
    prefix: usize,
}

struct Checker<'a> {
    live: Graph<'a>,
    written: Graph<'a>,
    /// Pairs assumed or proven within the current method, with the order
    /// they were added in so a failed probe can roll back exactly its own.
    assumed: HashSet<Pair>,
    log: Vec<Pair>,
    /// The assumed pairs whose walk finished, with the warnings it recorded;
    /// an assumed pair absent here is still on the stack.
    proven: HashMap<Pair, Proven>,
    /// Pairs known not to hold, within the current method.
    failed: BTreeMap<Pair, Failure>,
    warnings: Vec<Warning>,
    /// The path from the method root to the pair under test, outermost
    /// first; warnings copy it.
    path: Vec<Segment>,
    depth: usize,
    steps: usize,
}

impl<'a> Checker<'a> {
    fn graph(&self, side: Side) -> &Graph<'a> {
        match side {
            Side::Live => &self.live,
            Side::Written => &self.written,
        }
    }

    /// The name of field `id` of a record or variant: the written source's
    /// spelling when the written side's node names it, else the live one's.
    fn field_name(&self, sub: (Side, u32), sup: (Side, u32), id: u32) -> Option<String> {
        let (written, live) = if sub.0 == Side::Written {
            (sub.1, sup.1)
        } else {
            (sup.1, sub.1)
        };
        self.written
            .names
            .get(&(written, id))
            .or_else(|| self.live.names.get(&(live, id)))
            .map(|name| (*name).to_string())
    }

    /// Resolve a node to whether it is `null`, `reserved` or an `opt` —
    /// the types a record field or tuple value may be absent at.
    fn opt_like(&self, side: Side, reference: u32) -> bool {
        matches!(
            self.graph(side).node(reference),
            TypeNode::Opt { .. }
                | TypeNode::Primitive {
                    primitive: PrimitiveType::Null | PrimitiveType::Reserved,
                }
        )
    }

    /// `sub <: sup`, where `sub` is a reference into `side` and `sup` one
    /// into the other Contract.
    #[inline(never)]
    fn sub(&mut self, side: Side, sub: u32, sup: u32) -> Outcome {
        let pair = (side, sub, sup);
        if let Some(failure) = self.failed.get(&pair) {
            return Err(Stop::Fails(failure.clone()));
        }
        if self.assumed.contains(&pair) {
            return match self.proven.get(&pair).copied() {
                Some(proven) => self.repeat(proven),
                None => Ok(()),
            };
        }
        self.steps += 1;
        if self.steps > MAX_CHECK_STEPS {
            return Err(Stop::Exhausted(Exhausted {
                resource: "check_steps",
                limit: MAX_CHECK_STEPS,
                observed: self.steps,
            }));
        }
        if self.depth >= MAX_CHECK_DEPTH {
            return Err(Stop::Exhausted(Exhausted {
                resource: "check_depth",
                limit: MAX_CHECK_DEPTH,
                observed: self.depth + 1,
            }));
        }
        self.assumed.insert(pair);
        self.log.push(pair);
        self.depth += 1;
        let start = self.warnings.len();
        let outcome = self.expand(side, sub, sup);
        self.depth -= 1;
        match &outcome {
            Ok(()) => {
                let proven = Proven {
                    start,
                    end: self.warnings.len(),
                    prefix: self.path.len(),
                };
                self.proven.insert(pair, proven);
            }
            Err(Stop::Fails(failure)) => {
                self.failed.insert(pair, failure.clone());
            }
            Err(Stop::Exhausted(_)) => {}
        }
        outcome
    }

    /// Report a proven pair's warnings again under the current path. Its
    /// range stays valid: a failed probe truncates only warnings recorded
    /// after it began, and forgets every pair proven in that time.
    #[inline(never)]
    fn repeat(&mut self, proven: Proven) -> Outcome {
        for index in proven.start..proven.end {
            let mut path = self.path.clone();
            path.extend_from_slice(&self.warnings[index].path[proven.prefix..]);
            let message = self.warnings[index].message.clone();
            self.record(Warning { path, message })?;
        }
        Ok(())
    }

    /// A child pair one segment below the current one.
    #[inline(never)]
    fn child(&mut self, segment: Segment, side: Side, sub: u32, sup: u32) -> Outcome {
        self.path.push(segment.clone());
        let outcome = self.sub(side, sub, sup);
        self.path.pop();
        within(segment, outcome)
    }

    /// Run `sub <: sup` as a transaction: on failure, forget every pair and
    /// warning it added. Only a bound being reached escapes it.
    #[inline(never)]
    fn probe(&mut self, segment: Segment, side: Side, sub: u32, sup: u32) -> Result<bool, Stop> {
        let assumed = self.log.len();
        let warnings = self.warnings.len();
        let outcome = self.child(segment, side, sub, sup);
        match outcome {
            Ok(()) => Ok(true),
            Err(Stop::Fails(_)) => {
                for pair in self.log.drain(assumed..) {
                    self.assumed.remove(&pair);
                    self.proven.remove(&pair);
                }
                self.warnings.truncate(warnings);
                Ok(false)
            }
            Err(exhausted) => Err(exhausted),
        }
    }

    fn warn(&mut self, message: String) -> Outcome {
        let path = self.path.clone();
        self.record(Warning { path, message })
    }

    fn record(&mut self, warning: Warning) -> Outcome {
        if self.warnings.len() >= MAX_CHECK_WARNINGS {
            return Err(Stop::Exhausted(Exhausted {
                resource: "check_warnings",
                limit: MAX_CHECK_WARNINGS,
                observed: self.warnings.len() + 1,
            }));
        }
        self.warnings.push(warning);
        Ok(())
    }

    /// Dispatch one pair to its rule. The recursion runs through here, so
    /// every rule is its own small function and every message is built out
    /// of line: what stays on the stack per level is a few words, which is
    /// what lets [`MAX_CHECK_DEPTH`] levels fit a browser's stack.
    fn expand(&mut self, side: Side, sub: u32, sup: u32) -> Outcome {
        let left: &'a TypeNode = self.graph(side).node(sub);
        let right: &'a TypeNode = self.graph(side.other()).node(sup);
        use PrimitiveType::{Empty, Int, Nat, Null, Reserved};
        match (left, right) {
            (
                _,
                TypeNode::Primitive {
                    primitive: Reserved,
                },
            ) => Ok(()),
            (TypeNode::Primitive { primitive: Empty }, _) => Ok(()),
            (TypeNode::Primitive { primitive: Nat }, TypeNode::Primitive { primitive: Int }) => {
                Ok(())
            }
            (TypeNode::Primitive { primitive: a }, TypeNode::Primitive { primitive: b })
                if a == b =>
            {
                Ok(())
            }
            (TypeNode::Vec { inner: a }, TypeNode::Vec { inner: b }) => {
                self.child(Segment::Element, side, *a, *b)
            }
            (TypeNode::Primitive { primitive: Null }, TypeNode::Opt { .. }) => Ok(()),
            (_, TypeNode::Opt { inner }) => self.opt_rule(side, sub, left, *inner),
            (TypeNode::Record { fields: have }, TypeNode::Record { fields: want }) => {
                self.record_rule(side, sub, sup, have, want)
            }
            (TypeNode::Variant { fields: have }, TypeNode::Variant { fields: want }) => {
                self.variant_rule(side, sub, sup, have, want)
            }
            (TypeNode::Func { .. }, TypeNode::Func { .. }) => self.func_rule(side, left, right),
            (TypeNode::Service { methods: have }, TypeNode::Service { methods: want }) => {
                self.service_rule(side, have, want)
            }
            _ => mismatch(left, right),
        }
    }

    /// `t <: opt t'` holds for every `t` (the special opt rule), so the
    /// verdict is settled. What remains is whether a value keeps its content
    /// — `opt t <: opt t'` with `t <: t'`, or `t <: opt t'` with `t <: t'`
    /// and `t'` not opt-like — or is read as `null`, which is what the
    /// warning reports.
    #[inline(never)]
    fn opt_rule(&mut self, side: Side, sub: u32, left: &'a TypeNode, inner: u32) -> Outcome {
        let kept = match left {
            TypeNode::Opt { inner: content } => self.probe(Segment::Opt, side, *content, inner)?,
            _ if self.opt_like(side.other(), inner) => false,
            _ => self.probe(Segment::Opt, side, sub, inner)?,
        };
        if kept {
            Ok(())
        } else {
            self.warn(special_opt_rule(left))
        }
    }

    /// Every field of the super record present in the sub record as a
    /// subtype, or opt-like where the sub record has none.
    #[inline(never)]
    fn record_rule(
        &mut self,
        side: Side,
        sub: u32,
        sup: u32,
        have: &'a [Field],
        want: &'a [Field],
    ) -> Outcome {
        for field in want {
            let name = self.field_name((side, sub), (side.other(), sup), field.id);
            match have.iter().find(|candidate| candidate.id == field.id) {
                Some(found) => {
                    self.child(Segment::Field(name, field.id), side, found.ty, field.ty)?
                }
                None if self.opt_like(side.other(), field.ty) => {}
                None => {
                    let missing = self.graph(side.other()).node(field.ty);
                    return within(Segment::Field(name, field.id), field_missing(missing));
                }
            }
        }
        Ok(())
    }

    /// Every arm of the sub variant present in the super variant, as a
    /// subtype.
    #[inline(never)]
    fn variant_rule(
        &mut self,
        side: Side,
        sub: u32,
        sup: u32,
        have: &'a [Field],
        want: &'a [Field],
    ) -> Outcome {
        for arm in have {
            let name = self.field_name((side, sub), (side.other(), sup), arm.id);
            match want.iter().find(|candidate| candidate.id == arm.id) {
                Some(found) => self.child(Segment::Field(name, arm.id), side, arm.ty, found.ty)?,
                None => return within(Segment::Field(name, arm.id), arm_unknown()),
            }
        }
        Ok(())
    }

    /// Equal modes, arguments contravariant, results covariant.
    #[inline(never)]
    fn func_rule(&mut self, side: Side, left: &'a TypeNode, right: &'a TypeNode) -> Outcome {
        let (
            TypeNode::Func {
                args: sub_args,
                results: sub_results,
                mode: sub_mode,
            },
            TypeNode::Func {
                args: sup_args,
                results: sup_results,
                mode: sup_mode,
            },
        ) = (left, right)
        else {
            unreachable!("dispatched on two funcs");
        };
        if sub_mode != sup_mode {
            return mode_differs(*sub_mode, *sup_mode);
        }
        self.tuple(side.other(), sup_args, sub_args, Segment::Argument)?;
        self.tuple(side, sub_results, sup_results, Segment::Result)
    }

    /// Every method of the super service present in the sub service, as a
    /// subtype.
    #[inline(never)]
    fn service_rule(
        &mut self,
        side: Side,
        have: &'a [ServiceMethod],
        want: &'a [ServiceMethod],
    ) -> Outcome {
        for method in want {
            let segment = Segment::Method(method.name.clone());
            match have.iter().find(|candidate| candidate.name == method.name) {
                Some(found) => self.child(segment, side, found.function, method.function)?,
                None => return within(segment, method_absent()),
            }
        }
        Ok(())
    }

    /// Tuple subtyping, `sub <: sup` with `sub` in `side`: every `sup` value
    /// present in `sub` as a subtype, or opt-like where `sub` has none.
    #[inline(never)]
    fn tuple(
        &mut self,
        side: Side,
        sub: &'a [u32],
        sup: &'a [u32],
        segment: fn(usize) -> Segment,
    ) -> Outcome {
        for (index, want) in sup.iter().enumerate() {
            match sub.get(index) {
                Some(have) => self.child(segment(index), side, *have, *want)?,
                None if self.opt_like(side.other(), *want) => {}
                None => {
                    let missing = self.graph(side.other()).node(*want);
                    return within(segment(index), value_missing(missing));
                }
            }
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn mismatch(left: &TypeNode, right: &TypeNode) -> Outcome {
    fails(format!(
        "{} is not a subtype of {}",
        describe(left),
        describe(right)
    ))
}

#[cold]
#[inline(never)]
fn field_missing(missing: &TypeNode) -> Outcome {
    fails(format!(
        "the record has no such field, and its type {} is not null, reserved or an opt",
        describe(missing),
    ))
}

#[cold]
#[inline(never)]
fn value_missing(missing: &TypeNode) -> Outcome {
    fails(format!(
        "there is no value at this position, and its type {} is not null, reserved or an opt",
        describe(missing),
    ))
}

#[cold]
#[inline(never)]
fn arm_unknown() -> Outcome {
    fails("the other variant has no such arm")
}

#[cold]
#[inline(never)]
fn method_absent() -> Outcome {
    fails("the service has no such method")
}

#[cold]
#[inline(never)]
fn mode_differs(sub: MethodMode, sup: MethodMode) -> Outcome {
    fails(format!(
        "the function is {} on one side and {} on the other",
        mode_name(sub),
        mode_name(sup),
    ))
}

#[cold]
#[inline(never)]
fn special_opt_rule(left: &TypeNode) -> String {
    format!(
        "{} is not a subtype of the opt's content, so the special opt rule applies: a value here decodes as null",
        describe(left),
    )
}

fn describe(node: &TypeNode) -> String {
    match node {
        TypeNode::Primitive { primitive } => primitive_name(*primitive).to_string(),
        TypeNode::Opt { .. } => "an opt".to_string(),
        TypeNode::Vec { .. } => "a vec".to_string(),
        TypeNode::Record { .. } => "a record".to_string(),
        TypeNode::Variant { .. } => "a variant".to_string(),
        TypeNode::Func { .. } => "a func".to_string(),
        TypeNode::Service { .. } => "a service".to_string(),
        TypeNode::Class { .. } => "a service class".to_string(),
    }
}

fn primitive_name(primitive: PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Null => "null",
        PrimitiveType::Bool => "bool",
        PrimitiveType::Nat => "nat",
        PrimitiveType::Int => "int",
        PrimitiveType::Nat8 => "nat8",
        PrimitiveType::Nat16 => "nat16",
        PrimitiveType::Nat32 => "nat32",
        PrimitiveType::Nat64 => "nat64",
        PrimitiveType::Int8 => "int8",
        PrimitiveType::Int16 => "int16",
        PrimitiveType::Int32 => "int32",
        PrimitiveType::Int64 => "int64",
        PrimitiveType::Float32 => "float32",
        PrimitiveType::Float64 => "float64",
        PrimitiveType::Text => "text",
        PrimitiveType::Reserved => "reserved",
        PrimitiveType::Empty => "empty",
        PrimitiveType::Principal => "principal",
    }
}

fn mode_name(mode: MethodMode) -> &'static str {
    match mode {
        MethodMode::Update => "update",
        MethodMode::Query => "query",
        MethodMode::CompositeQuery => "composite_query",
        MethodMode::Oneway => "oneway",
    }
}

/// A name as a path segment: `.name` when identifier-shaped, else quoted.
fn name_segment(name: &str, prefix: &str) -> String {
    let identifier = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '$'
        });
    if identifier {
        format!("{prefix}{name}")
    } else {
        let quoted = serde_json::to_string(name).expect("strings serialize");
        format!("{}[{quoted}]", prefix.trim_end_matches('.'))
    }
}

/// Render a path, outermost first: `$args[0]`, `$results[1]` at the root,
/// then `.name` or `["name"]` (or `[id]` for an unnamed label) per field or
/// arm, `[*]` per vec element, `?` per opt content, and `::args[i]`,
/// `::results[i]` or `::name` into a func or service type.
fn render(path: &[Segment]) -> String {
    let mut text = String::from("$");
    for (index, segment) in path.iter().enumerate() {
        let nested = if index == 0 { "" } else { "::" };
        match segment {
            Segment::Field(Some(name), _) => text.push_str(&name_segment(name, ".")),
            Segment::Field(None, id) => text.push_str(&format!("[{id}]")),
            Segment::Element => text.push_str("[*]"),
            Segment::Opt => text.push('?'),
            Segment::Argument(position) => text.push_str(&format!("{nested}args[{position}]")),
            Segment::Result(position) => text.push_str(&format!("{nested}results[{position}]")),
            Segment::Method(name) => text.push_str(&name_segment(name, "::")),
        }
    }
    text
}

/// The methods of a Contract's actor service, in the Contract's order.
pub(crate) fn actor_methods(contract: &Contract) -> Option<Vec<(String, u32)>> {
    let service = match contract.actor()? {
        Actor::Service { service } => *service,
        Actor::Class { class } => match &contract.types()[*class as usize] {
            TypeNode::Class { service, .. } => *service,
            _ => return None,
        },
    };
    match &contract.types()[service as usize] {
        TypeNode::Service { methods } => Some(
            methods
                .iter()
                .map(|method| (method.name.clone(), method.function))
                .collect(),
        ),
        _ => None,
    }
}

/// One side of a check, compiled.
pub struct Input<'a> {
    pub contract: &'a Contract,
    pub source_info: Option<&'a SourceInfo>,
}

/// The diagnostics of `live <: written`, in written-method name order (code
/// point), each method's errors before its warnings. The check passes
/// exactly when no diagnostic has severity `error`. Both Contracts must have
/// an actor; the caller refuses one that does not.
pub fn check(written: &Input<'_>, live: &Input<'_>) -> Vec<Value> {
    let written_methods = actor_methods(written.contract).expect("the caller checked the actor");
    let live_methods = actor_methods(live.contract).expect("the caller checked the actor");
    let mut checker = Checker {
        live: Graph::new(live.contract, live.source_info),
        written: Graph::new(written.contract, written.source_info),
        assumed: HashSet::new(),
        log: Vec::new(),
        proven: HashMap::new(),
        failed: BTreeMap::new(),
        warnings: Vec::new(),
        path: Vec::new(),
        depth: 0,
        steps: 0,
    };
    let mut ordered = written_methods.clone();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    let mut diagnostics = Vec::new();
    for (name, written_function) in ordered {
        let Some((_, live_function)) = live_methods
            .iter()
            .find(|(live_name, _)| *live_name == name)
        else {
            diagnostics.push(json!({
                "code": "method_missing",
                "severity": "error",
                "method": name,
                "message": format!("the live service has no method {name:?}"),
            }));
            continue;
        };
        let (
            TypeNode::Func {
                args: written_args,
                results: written_results,
                mode: written_mode,
            },
            TypeNode::Func {
                args: live_args,
                results: live_results,
                mode: live_mode,
            },
        ) = (
            checker.written.node(written_function),
            checker.live.node(*live_function),
        )
        else {
            unreachable!("a service method's type is a func in a valid Contract");
        };
        if written_mode != live_mode {
            diagnostics.push(json!({
                "code": "mode_changed",
                "severity": "error",
                "method": name,
                "message": format!(
                    "the written method is {} and the live one is {}",
                    mode_name(*written_mode),
                    mode_name(*live_mode),
                ),
            }));
        }
        checker.assumed.clear();
        checker.log.clear();
        checker.proven.clear();
        checker.failed.clear();
        checker.warnings.clear();
        checker.path.clear();
        checker.depth = 0;
        checker.steps = 0;
        let outcome = checker
            .tuple(Side::Written, written_args, live_args, Segment::Argument)
            .and_then(|()| {
                checker.tuple(Side::Live, live_results, written_results, Segment::Result)
            });
        match outcome {
            Ok(()) => {}
            Err(Stop::Fails(failure)) => {
                let mut path = failure.segments;
                path.reverse();
                diagnostics.push(json!({
                    "code": "method_incompatible",
                    "severity": "error",
                    "method": name,
                    "path": render(&path),
                    "message": failure.message,
                }));
            }
            Err(Stop::Exhausted(exhausted)) => {
                diagnostics.push(json!({
                    "code": "resource_limit_exceeded",
                    "severity": "error",
                    "method": name,
                    "message": format!(
                        "the check of this method stopped at its {} bound of {}",
                        exhausted.resource, exhausted.limit,
                    ),
                    "resource_limit": {
                        "resource": exhausted.resource,
                        "limit": exhausted.limit,
                        "observed": exhausted.observed,
                    },
                }));
                // Nothing a bounded walk recorded is complete; report the
                // bound alone for this method.
                checker.warnings.clear();
            }
        }
        for warning in checker.warnings.drain(..) {
            diagnostics.push(json!({
                "code": "special_opt_rule",
                "severity": "warning",
                "method": name,
                "path": render(&warning.path),
                "message": warning.message,
            }));
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_render_names_ids_and_nesting() {
        let path = vec![
            Segment::Result(0),
            Segment::Field(Some("Ok".into()), 1),
            Segment::Opt,
            Segment::Element,
            Segment::Field(None, 3),
            Segment::Field(Some("two words".into()), 9),
            Segment::Argument(1),
            Segment::Method("get".into()),
            Segment::Method("not ident".into()),
            Segment::Result(2),
        ];
        assert_eq!(
            render(&path),
            r#"$results[0].Ok?[*][3]["two words"]::args[1]::get::["not ident"]::results[2]"#
        );
        assert_eq!(render(&[Segment::Argument(0)]), "$args[0]");
        assert_eq!(render(&[]), "$");
    }
}
