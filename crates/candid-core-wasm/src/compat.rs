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
//! transaction: when it fails, every pair it proved is forgotten, together
//! with every warning it recorded. A pair that fails is cached as failed for
//! the rest of the method; that is sound, because assumptions only ever make
//! more pairs hold. Each written method starts from empty state — no
//! assumption, no cached failure, no step spent — so its diagnostics, paths
//! and per-method bounds included, do not depend on which other methods the
//! written service declares until the check as a whole reaches one of its
//! aggregate bounds (see [Bounds](#bounds)).
//!
//! # Warnings
//!
//! A `special_opt_rule` warning is reported at every path where a value
//! decodes as `null` and along which no pair of types repeats: exactly the
//! paths the walk would visit if it re-walked every pair it met and cut each
//! path only where it meets a pair already on it. A recursive type has
//! endless paths; this keeps those that do not come round to a pair again.
//! The set does not depend on the order fields, arguments or methods are
//! declared or visited in.
//!
//! The walk does not re-walk: a pair proven once keeps the warnings its walk
//! recorded, each with the pairs along its path, and re-reports them under
//! each further path it is met at, leaving out those whose path passes
//! through a pair on the current one. A proof that met a pair still on the
//! stack (a cycle through an enclosing pair) is cut there, so it is kept
//! only while that pair's walk is in progress; once it finishes, the next
//! meeting walks the pair again. A test holds the result to the re-walking
//! walk on every hand case and every case of the random campaign.
//!
//! # Bounds
//!
//! The walk is recursive, so its depth is bounded ([`MAX_CHECK_DEPTH`] pairs
//! on one path), and so is its work ([`MAX_CHECK_STEPS`] steps), each per
//! method. A step is one unit of work done, so the bound bounds the walk's
//! time: a pair visited, however it is answered (expanded, from a proof, at
//! a pair in progress, or from a failed pair, which also costs one step per
//! segment of the failure path it copies); a field, arm, method or value
//! examined, or passed over because the other side lacks it; a proven
//! pair's warning re-examined, one step per pair below the proven one on its path, at least
//! one; a warning recorded or re-reported, one step per segment of the
//! path it copies; and a failure recorded for a pair, one step per segment
//! of its path, so the failures a method keeps are bounded by its steps too.
//! Fields and arms are paired in one pass over the two
//! id-ordered lists, and methods in one pass by id and name, so no step
//! hides a scan. Reaching either bound fails the method closed with
//! `resource_limit_exceeded`; the compiler's own limits keep every Contract
//! it accepts well below both for interfaces of ordinary shape. The warnings
//! a method reports are bounded too ([`MAX_CHECK_WARNINGS`]), but that bound
//! does not touch the verdict: the decision is complete, so past it the
//! method keeps its verdict, reports the first warnings, and ends with a
//! `resource_limit_exceeded` *warning* saying the rest were not reported.
//!
//! Per-method bounds alone would let the written service multiply them: a
//! compiler-accepted source can declare thousands of methods that each walk
//! the same shared type. So the check as a whole is bounded too, by three
//! aggregates: [`MAX_CHECK_TOTAL_STEPS`] steps across all methods, each
//! method also costing one for being decided, so the bound caps the method
//! count too (`check_total_steps`); [`MAX_CHECK_TOTAL_WARNINGS`]
//! `special_opt_rule` warnings reported (`check_total_warnings`); and
//! [`MAX_CHECK_OUTPUT_BYTES`] bytes of diagnostic text — the `method`, `path`
//! and `message` strings of every diagnostic a method reports, as the JSON
//! response writes them, escapes included (`check_output_bytes`). Methods
//! are checked in name order. The step bound is checked as the work is spent;
//! the other two when a method is decided, against what it would report.
//! That text is measured string by string (a name, a message, one segment
//! of a path), and only up to the first string that passes the byte bound;
//! a string longer than what is left of it is counted at its raw length,
//! which JSON never shortens, without being measured. The step bounds do
//! not count this measuring, so it is bounded by the bytes instead: at most
//! the byte bound plus one string's bytes over the whole check, however
//! long the names, paths and lists of diagnostics are. The method at which
//! an aggregate bound is reached, and every method after it, fail closed:
//! each gets exactly one diagnostic, an *error* `resource_limit_exceeded`
//! naming the aggregate resource with its limit and the value observed when
//! it was reached (for the bytes, the count at the string that passed the
//! bound), and nothing else — not its `mode_changed`, nor a
//! `method_missing`. Below these bounds a
//! method's diagnostics do not depend on the other methods; once one is
//! reached, which methods are reported depends on the methods before them.
//!
//! So a response holds at most [`MAX_CHECK_OUTPUT_BYTES`] of reported text,
//! at most [`MAX_CHECK_TOTAL_WARNINGS`] warnings, and per written method at
//! most three other diagnostics (`mode_changed`, one error, and the
//! warning-bound notice) or one fail-closed diagnostic, each of fixed shape
//! apart from its strings: the fail-closed ones grow with the written
//! method names, that is linearly with the input, 397 bytes each
//! pretty-printed for a six-character name. Paths borrow their names from
//! the two compiled sides, so the memory a walk holds does not grow with the
//! length of a name either. Each method starts from fresh maps rather than
//! cleared ones, whose cost would be the capacity an earlier method grew.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ops::ControlFlow;

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
/// 1 MiB of stack unoptimized, and between 128 and 256 KiB optimized. The
/// wasm artifact reaches it under Node without exhausting its stack, which
/// the package's tests assert.
pub const MAX_CHECK_DEPTH: usize = 384;

/// The most work the check of one method may do, in steps: pairs visited,
/// fields, arms, methods and values examined or passed over, and path
/// segments warnings and failures copy or warnings re-examine (see the
/// module's Bounds).
pub const MAX_CHECK_STEPS: usize = 1_000_000;

/// The most `special_opt_rule` warnings one method reports. Re-reporting a
/// proven pair's warnings under every path it is met at can multiply them
/// along a shared, non-recursive type graph (a record of two fields of a
/// record of two fields, and so on); past this bound the rest are dropped
/// and one `resource_limit_exceeded` warning says so. The verdict is not
/// affected.
pub const MAX_CHECK_WARNINGS: usize = 1_000;

/// The most work the whole check may do, across every written method, plus
/// one step per method decided: ten methods at [`MAX_CHECK_STEPS`]. Past it
/// the method being checked and every method after it fail closed
/// (`check_total_steps`).
pub const MAX_CHECK_TOTAL_STEPS: usize = 10_000_000;

/// The most `special_opt_rule` warnings the whole check reports: ten
/// methods at [`MAX_CHECK_WARNINGS`]. A method whose warnings would take
/// the total past it, and every method after it, fail closed
/// (`check_total_warnings`).
pub const MAX_CHECK_TOTAL_WARNINGS: usize = 10_000;

/// The most diagnostic text the whole check reports, in bytes: the
/// `method`, `path` and `message` strings of every diagnostic a method
/// reports, the fail-closed ones aside, as the JSON response writes them
/// (escapes included, quotes not). 4 MiB, the compiler's default
/// `max_input_bytes`, its bound on one document it parses. A method whose
/// diagnostics would take the total past it, and every method after it,
/// fail closed (`check_output_bytes`). The text is measured only up to the
/// first string that passes the bound (see the module's Bounds).
pub const MAX_CHECK_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// How a check runs. [`CheckOptions::default`] is what `checkCompatible`
/// uses; the others exist for this crate's tests.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct CheckOptions {
    /// The work bound of one method.
    pub step_limit: usize,
    /// The work bound of the whole check.
    pub total_step_limit: usize,
    /// The bound on the warnings the whole check reports.
    pub total_warning_limit: usize,
    /// The bound on the diagnostic text the whole check reports.
    pub output_byte_limit: usize,
    /// Whether a proven pair's warnings are re-reported instead of the pair
    /// being walked again. Off, every meeting walks it again, which is
    /// exponential in general: the tests' reference for the warnings.
    pub memo: bool,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            step_limit: MAX_CHECK_STEPS,
            total_step_limit: MAX_CHECK_TOTAL_STEPS,
            total_warning_limit: MAX_CHECK_TOTAL_WARNINGS,
            output_byte_limit: MAX_CHECK_OUTPUT_BYTES,
            memo: true,
        }
    }
}

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
/// travels up, reversed once when it is rendered. Names are borrowed from
/// the Contracts, so a path costs the same whatever their length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Segment<'a> {
    /// A record field or variant arm: its source name, or its numeric id.
    Field(Option<&'a str>, u32),
    /// The element type of a `vec`.
    Element,
    /// The content type of an `opt`.
    Opt,
    /// An argument of a func type (the method's own at the root).
    Argument(usize),
    /// A result of a func type (the method's own at the root).
    Result(usize),
    /// A method of a service type.
    Method(&'a str),
}

/// Why a pair does not hold, relative to that pair.
#[derive(Debug, Clone)]
struct Failure<'a> {
    /// Innermost first.
    segments: Vec<Segment<'a>>,
    message: String,
}

/// The walk could not finish within its bounds.
#[derive(Debug, Clone)]
struct Exhausted {
    resource: &'static str,
    limit: usize,
    observed: usize,
}

enum Stop<'a> {
    Fails(Failure<'a>),
    Exhausted(Exhausted),
}

type Outcome<'a> = Result<(), Stop<'a>>;

fn fails<'a>(message: impl Into<String>) -> Outcome<'a> {
    Err(Stop::Fails(Failure {
        segments: Vec::new(),
        message: message.into(),
    }))
}

/// Prepend a segment to a failure travelling up out of a child pair.
fn within<'a>(segment: Segment<'a>, outcome: Outcome<'a>) -> Outcome<'a> {
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
struct Warning<'a> {
    /// Outermost first, relative to the method.
    path: Vec<Segment<'a>>,
    /// The pair each segment of `path` leads to.
    trail: Vec<Pair>,
    message: String,
}

/// `(sub side, sub reference, super reference)`: the super side is always
/// the other Contract.
type Pair = (Side, u32, u32);

/// A set of stack depths, below [`MAX_CHECK_DEPTH`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Depths([u64; MAX_CHECK_DEPTH.div_ceil(64)]);

impl Depths {
    fn insert(&mut self, depth: usize) {
        self.0[depth / 64] |= 1 << (depth % 64);
    }

    /// The depths below `limit`.
    fn below(self, limit: usize) -> Self {
        let mut kept = self;
        for (index, word) in kept.0.iter_mut().enumerate() {
            let first = index * 64;
            if first >= limit {
                *word = 0;
            } else if limit - first < 64 {
                *word &= (1 << (limit - first)) - 1;
            }
        }
        kept
    }

    fn union(&mut self, other: Self) {
        for (word, more) in self.0.iter_mut().zip(other.0) {
            *word |= more;
        }
    }

    fn deepest(self) -> Option<usize> {
        self.0
            .iter()
            .enumerate()
            .rev()
            .find(|(_, word)| **word != 0)
            .map(|(index, word)| index * 64 + 63 - word.leading_zeros() as usize)
    }
}

/// The warnings a proven pair's walk recorded: `warnings[start..end]`, every
/// one of whose paths begins with the pair's own path of `prefix` segments.
#[derive(Debug, Clone, Copy)]
struct Proven {
    start: usize,
    end: usize,
    prefix: usize,
    /// The stack depths of the enclosing pairs its walk was cut at. The
    /// proof is kept only while they are all on the stack, that is while
    /// the deepest of them is.
    cuts: Depths,
}

/// A pair whose walk is in progress.
#[derive(Debug, Default)]
struct Frame {
    /// The stack depths of the enclosing pairs this walk was cut at so far,
    /// all below its own.
    cuts: Depths,
    /// The proven pairs whose proof depends on this walk still being in
    /// progress; forgotten when it finishes.
    dependents: Vec<Pair>,
}

struct Checker<'a> {
    live: Graph<'a>,
    written: Graph<'a>,
    options: CheckOptions,
    /// The pairs whose walk is in progress, with their depth in `frames`.
    active: HashMap<Pair, usize>,
    frames: Vec<Frame>,
    /// The pairs whose walk finished, with the warnings it recorded.
    proven: HashMap<Pair, Proven>,
    /// The pairs proven, in order, so a failed probe can forget exactly its
    /// own.
    log: Vec<Pair>,
    /// Pairs known not to hold, within the current method.
    failed: BTreeMap<Pair, Failure<'a>>,
    warnings: Vec<Warning<'a>>,
    /// Whether a warning was dropped at [`MAX_CHECK_WARNINGS`].
    truncated: bool,
    /// The path from the method root to the pair under test, outermost
    /// first, and the pair each segment leads to; warnings copy both.
    path: Vec<Segment<'a>>,
    trail: Vec<Pair>,
    /// The work spent on the current method.
    steps: usize,
    /// The work spent on the whole check, never reset.
    total_steps: usize,
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
    fn field_name(&self, sub: (Side, u32), sup: (Side, u32), id: u32) -> Option<&'a str> {
        let (written, live) = if sub.0 == Side::Written {
            (sub.1, sup.1)
        } else {
            (sup.1, sub.1)
        };
        self.written
            .names
            .get(&(written, id))
            .or_else(|| self.live.names.get(&(live, id)))
            .copied()
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
    fn sub(&mut self, side: Side, sub: u32, sup: u32) -> Outcome<'a> {
        let pair = (side, sub, sup);
        // Every visit is a step, however it is answered: from the failed
        // pairs, at a pair in progress, from a proof, or by expanding it.
        self.spend(1)?;
        if let Some(length) = self.failed.get(&pair).map(|failure| failure.segments.len()) {
            // Answering from a failed pair copies its path.
            self.spend(length)?;
            return Err(Stop::Fails(self.failed[&pair].clone()));
        }
        if let Some(&depth) = self.active.get(&pair) {
            self.cut_at(depth);
            return Ok(());
        }
        if self.options.memo && self.proven.contains_key(&pair) {
            return self.repeat(pair);
        }
        if self.frames.len() >= MAX_CHECK_DEPTH {
            return Err(Stop::Exhausted(Exhausted {
                resource: "check_depth",
                limit: MAX_CHECK_DEPTH,
                observed: self.frames.len() + 1,
            }));
        }
        self.active.insert(pair, self.frames.len());
        self.frames.push(Frame::default());
        let start = self.warnings.len();
        let outcome = self.expand(side, sub, sup);
        self.finish(pair, start, outcome)
    }

    /// Close a pair's walk: forget the proofs that depended on it being in
    /// progress, and record its own proof or failure. Recording a failure
    /// copies its path, one step per segment.
    #[inline(never)]
    fn finish(&mut self, pair: Pair, start: usize, outcome: Outcome<'a>) -> Outcome<'a> {
        let frame = self.frames.pop().expect("the walk pushed its frame");
        self.active.remove(&pair);
        for dependent in &frame.dependents {
            self.proven.remove(dependent);
        }
        match &outcome {
            Ok(()) => {
                let proven = Proven {
                    start,
                    end: self.warnings.len(),
                    prefix: self.path.len(),
                    cuts: frame.cuts,
                };
                self.proven.insert(pair, proven);
                self.log.push(pair);
                if let Some(deepest) = frame.cuts.deepest() {
                    self.frames[deepest].dependents.push(pair);
                    self.depend(frame.cuts);
                }
            }
            Err(Stop::Fails(failure)) => {
                self.spend(failure.segments.len())?;
                self.failed.insert(pair, failure.clone());
            }
            Err(Stop::Exhausted(_)) => {}
        }
        outcome
    }

    /// The walk in progress was cut at the pairs at these stack depths: its
    /// proof holds only while their walks are in progress. A cut at the
    /// walk's own pair costs nothing.
    fn depend(&mut self, cuts: Depths) {
        if let Some(top) = self.frames.len().checked_sub(1) {
            self.frames[top].cuts.union(cuts.below(top));
        }
    }

    /// The walk in progress was cut at the pair at stack depth `depth`.
    #[inline(never)]
    fn cut_at(&mut self, depth: usize) {
        let mut cut = Depths::default();
        cut.insert(depth);
        self.depend(cut);
    }

    /// Spend `amount` of the method's work bound and of the whole check's.
    /// Reaching the whole check's is reported first: it stops the rest.
    fn spend(&mut self, amount: usize) -> Outcome<'a> {
        self.steps += amount;
        self.total_steps += amount;
        if self.total_steps > self.options.total_step_limit {
            return Err(Stop::Exhausted(Exhausted {
                resource: "check_total_steps",
                limit: self.options.total_step_limit,
                observed: self.total_steps,
            }));
        }
        if self.steps > self.options.step_limit {
            return Err(Stop::Exhausted(Exhausted {
                resource: "check_steps",
                limit: self.options.step_limit,
                observed: self.steps,
            }));
        }
        Ok(())
    }

    /// Report a proven pair's warnings again under the current path, except
    /// those whose path passes through a pair on the current one: a fresh
    /// walk would be cut there, so the result depends on that pair's walk
    /// still being in progress. Its range stays valid: a failed probe
    /// truncates only warnings recorded after it began, and forgets every
    /// pair proven in that time.
    ///
    /// Re-examining a warning costs one step per pair its path holds below
    /// the proven pair, at least one; re-reporting it, one per segment of the
    /// path it is reported at.
    #[inline(never)]
    fn repeat(&mut self, pair: Pair) -> Outcome<'a> {
        let proven = self.proven[&pair];
        self.depend(proven.cuts);
        for index in proven.start..proven.end {
            let below = self.warnings[index].trail.len() - proven.prefix;
            self.spend(below.max(1))?;
            let cut = self.warnings[index].trail[proven.prefix..]
                .iter()
                .filter_map(|pair| self.active.get(pair).copied())
                .min();
            if let Some(depth) = cut {
                self.cut_at(depth);
                continue;
            }
            if self.warnings.len() >= MAX_CHECK_WARNINGS {
                self.truncated = true;
                break;
            }
            self.spend(self.path.len() + below)?;
            let warning = &self.warnings[index];
            let mut path = self.path.clone();
            path.extend_from_slice(&warning.path[proven.prefix..]);
            let mut trail = self.trail.clone();
            trail.extend_from_slice(&warning.trail[proven.prefix..]);
            let message = warning.message.clone();
            self.warnings.push(Warning {
                path,
                trail,
                message,
            });
        }
        Ok(())
    }

    /// A child pair one segment below the current one.
    #[inline(never)]
    fn child(&mut self, segment: Segment<'a>, side: Side, sub: u32, sup: u32) -> Outcome<'a> {
        self.path.push(segment);
        self.trail.push((side, sub, sup));
        let outcome = self.sub(side, sub, sup);
        self.trail.pop();
        self.path.pop();
        within(segment, outcome)
    }

    /// Run `sub <: sup` as a transaction: on failure, forget every pair it
    /// proved and every warning it recorded. Only a bound being reached
    /// escapes it.
    #[inline(never)]
    fn probe(
        &mut self,
        segment: Segment<'a>,
        side: Side,
        sub: u32,
        sup: u32,
    ) -> Result<bool, Stop<'a>> {
        let proven = self.log.len();
        let warnings = self.warnings.len();
        let truncated = self.truncated;
        let outcome = self.child(segment, side, sub, sup);
        match outcome {
            Ok(()) => Ok(true),
            Err(Stop::Fails(_)) => {
                for pair in self.log.drain(proven..) {
                    self.proven.remove(&pair);
                }
                self.warnings.truncate(warnings);
                self.truncated = truncated;
                Ok(false)
            }
            Err(exhausted) => Err(exhausted),
        }
    }

    /// Record a special-opt-rule warning at the current path, one step per
    /// segment it copies, or drop it past [`MAX_CHECK_WARNINGS`] without
    /// building its message.
    fn warn(&mut self, left: &TypeNode) -> Outcome<'a> {
        if self.warnings.len() >= MAX_CHECK_WARNINGS {
            self.truncated = true;
            return Ok(());
        }
        self.spend(self.path.len())?;
        let message = special_opt_rule(left);
        let path = self.path.clone();
        let trail = self.trail.clone();
        self.warnings.push(Warning {
            path,
            trail,
            message,
        });
        Ok(())
    }

    /// Dispatch one pair to its rule. The recursion runs through here, so
    /// every rule is its own small function and every message is built out
    /// of line: what stays on the stack per level is a few words, which is
    /// what lets [`MAX_CHECK_DEPTH`] levels fit a browser's stack.
    fn expand(&mut self, side: Side, sub: u32, sup: u32) -> Outcome<'a> {
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
    fn opt_rule(&mut self, side: Side, sub: u32, left: &'a TypeNode, inner: u32) -> Outcome<'a> {
        let kept = match left {
            TypeNode::Opt { inner: content } => self.probe(Segment::Opt, side, *content, inner)?,
            _ if self.opt_like(side.other(), inner) => false,
            _ => self.probe(Segment::Opt, side, sub, inner)?,
        };
        if kept {
            Ok(())
        } else {
            self.warn(left)
        }
    }

    /// Every field of the super record present in the sub record as a
    /// subtype, or opt-like where the sub record has none. Both lists are in
    /// id order, the Contract's canonical field order, so one pass over them
    /// pairs the fields: a step per field of the super record, and one per
    /// field of the sub record passed over.
    #[inline(never)]
    fn record_rule(
        &mut self,
        side: Side,
        sub: u32,
        sup: u32,
        have: &'a [Field],
        want: &'a [Field],
    ) -> Outcome<'a> {
        debug_assert!(in_id_order(have) && in_id_order(want));
        let mut next = 0;
        for field in want {
            self.spend(1)?;
            let found = self.same_id(have, &mut next, field.id)?;
            let name = self.field_name((side, sub), (side.other(), sup), field.id);
            match found {
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
    ) -> Outcome<'a> {
        debug_assert!(in_id_order(have) && in_id_order(want));
        let mut next = 0;
        for arm in have {
            self.spend(1)?;
            let found = self.same_id(want, &mut next, arm.id)?;
            let name = self.field_name((side, sub), (side.other(), sup), arm.id);
            match found {
                Some(found) => self.child(Segment::Field(name, arm.id), side, arm.ty, found.ty)?,
                None => return within(Segment::Field(name, arm.id), arm_unknown()),
            }
        }
        Ok(())
    }

    /// The field of `fields`, from `*next` on, whose id is `id`: one step per
    /// field passed over. `fields` is in strictly increasing id order and the
    /// ids asked for increase, so `*next` only moves forward.
    fn same_id(
        &mut self,
        fields: &'a [Field],
        next: &mut usize,
        id: u32,
    ) -> Result<Option<&'a Field>, Stop<'a>> {
        while fields.get(*next).is_some_and(|field| field.id < id) {
            self.spend(1)?;
            *next += 1;
        }
        match fields.get(*next) {
            Some(field) if field.id == id => {
                *next += 1;
                Ok(Some(field))
            }
            _ => Ok(None),
        }
    }

    /// Equal modes, arguments contravariant, results covariant.
    #[inline(never)]
    fn func_rule(&mut self, side: Side, left: &'a TypeNode, right: &'a TypeNode) -> Outcome<'a> {
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
    /// subtype. Both lists are in the Contract's canonical method order, by
    /// id and then by name, so one pass pairs them, a step per method of the
    /// super service and one per method of the sub service passed over.
    #[inline(never)]
    fn service_rule(
        &mut self,
        side: Side,
        have: &'a [ServiceMethod],
        want: &'a [ServiceMethod],
    ) -> Outcome<'a> {
        let key = |method: &'a ServiceMethod| (method.id, method.name.as_str());
        debug_assert!(have.windows(2).all(|pair| key(&pair[0]) < key(&pair[1])));
        let mut next = 0;
        for method in want {
            self.spend(1)?;
            while have.get(next).is_some_and(|found| key(found) < key(method)) {
                self.spend(1)?;
                next += 1;
            }
            let segment = Segment::Method(method.name.as_str());
            match have.get(next).filter(|found| found.name == method.name) {
                Some(found) => {
                    next += 1;
                    self.child(segment, side, found.function, method.function)?
                }
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
        segment: fn(usize) -> Segment<'a>,
    ) -> Outcome<'a> {
        for (index, want) in sup.iter().enumerate() {
            self.spend(1)?;
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

/// Whether fields are in strictly increasing id order, as a Contract keeps
/// them.
fn in_id_order(fields: &[Field]) -> bool {
    fields.windows(2).all(|pair| pair[0].id < pair[1].id)
}

#[cold]
#[inline(never)]
fn mismatch<'a>(left: &TypeNode, right: &TypeNode) -> Outcome<'a> {
    fails(format!(
        "{} is not a subtype of {}",
        describe(left),
        describe(right)
    ))
}

#[cold]
#[inline(never)]
fn field_missing<'a>(missing: &TypeNode) -> Outcome<'a> {
    fails(format!(
        "the record has no such field, and its type {} is not null, reserved or an opt",
        describe(missing),
    ))
}

#[cold]
#[inline(never)]
fn value_missing<'a>(missing: &TypeNode) -> Outcome<'a> {
    fails(format!(
        "there is no value at this position, and its type {} is not null, reserved or an opt",
        describe(missing),
    ))
}

#[cold]
#[inline(never)]
fn arm_unknown<'a>() -> Outcome<'a> {
    fails("the other variant has no such arm")
}

#[cold]
#[inline(never)]
fn method_absent<'a>() -> Outcome<'a> {
    fails("the service has no such method")
}

#[cold]
#[inline(never)]
fn mode_differs<'a>(sub: MethodMode, sup: MethodMode) -> Outcome<'a> {
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

/// Whether a name is written as `.name` in a path, rather than quoted.
fn is_identifier(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '$'
        })
}

/// A name as a path segment: `.name` when identifier-shaped, else quoted.
fn name_segment(name: &str, prefix: &str) -> String {
    if is_identifier(name) {
        format!("{prefix}{name}")
    } else {
        let quoted = serde_json::to_string(name).expect("strings serialize");
        format!("{}[{quoted}]", prefix.trim_end_matches('.'))
    }
}

/// The bytes [`name_segment`]'s text takes inside a JSON string, quotes
/// aside, without building it: at least `prefix.len() + name.len()`, and
/// computed in time linear in that.
fn name_segment_len(name: &str, prefix: &str) -> usize {
    if is_identifier(name) {
        // Neither the prefix nor an identifier needs an escape.
        prefix.len() + name.len()
    } else {
        // `[`, the name as `serde_json` quotes it, and `]`, then escaped
        // again as a JSON string: each quote and backslash doubles, so the
        // two quotes take four bytes, and so does a quote or a backslash in
        // the name (`\"` written as `\\\"`).
        let quoted: usize = name
            .bytes()
            .map(|byte| match byte {
                b'"' | b'\\' => 4,
                b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 3,
                0x00..=0x1f => 7,
                _ => 1,
            })
            .sum();
        prefix.trim_end_matches('.').len() + 2 + 4 + quoted
    }
}

/// One segment of a rendered path: a name, rendered by [`name_segment`]
/// with its prefix, or text of its own of a few bytes.
enum Part<'a> {
    Name(&'a str, &'static str),
    Text(Cow<'static, str>),
}

/// Render a path, outermost first: `$args[0]`, `$results[1]` at the root,
/// then `.name` or `["name"]` (or `[id]` for an unnamed label) per field or
/// arm, `[*]` per vec element, `?` per opt content, and `::args[i]`,
/// `::results[i]` or `::name` into a func or service type.
fn render(path: &[Segment<'_>]) -> String {
    let mut text = String::new();
    render_parts(path, |part| {
        match part {
            Part::Name(name, prefix) => text.push_str(&name_segment(name, prefix)),
            Part::Text(part) => text.push_str(&part),
        }
        ControlFlow::Continue(())
    });
    text
}

/// The length in bytes of [`render`]'s text as a JSON string writes it,
/// quotes aside.
#[cfg(test)]
fn rendered_len(path: &[Segment<'_>]) -> usize {
    let mut meter = Meter {
        counted: 0,
        limit: usize::MAX,
    };
    meter.path(path);
    meter.counted
}

/// The bytes `text` takes inside a JSON string, quotes aside: what
/// `serde_json` writes for it, where `"`, `\` and the control characters
/// are escaped (`\n` and the like in two bytes, the rest as `\u00XX`).
fn json_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

/// Hand [`render`]'s text to `each`, one segment at a time, until it
/// breaks.
fn render_parts<'a>(path: &[Segment<'a>], mut each: impl FnMut(Part<'a>) -> ControlFlow<()>) {
    if each(Part::Text(Cow::Borrowed("$"))).is_break() {
        return;
    }
    for (index, segment) in path.iter().enumerate() {
        let nested = if index == 0 { "" } else { "::" };
        let part = match *segment {
            Segment::Field(Some(name), _) => Part::Name(name, "."),
            Segment::Field(None, id) => Part::Text(Cow::Owned(format!("[{id}]"))),
            Segment::Element => Part::Text(Cow::Borrowed("[*]")),
            Segment::Opt => Part::Text(Cow::Borrowed("?")),
            Segment::Argument(position) => {
                Part::Text(Cow::Owned(format!("{nested}args[{position}]")))
            }
            Segment::Result(position) => {
                Part::Text(Cow::Owned(format!("{nested}results[{position}]")))
            }
            Segment::Method(name) => Part::Name(name, "::"),
        };
        if each(part).is_break() {
            return;
        }
    }
}

/// Counts diagnostic text as the JSON response writes it, against the
/// output bound, and stops counting once past it.
///
/// A string is measured exactly only while its raw bytes fit in what is
/// left of the bound. One that does not is counted at its raw length,
/// which is never more than what JSON writes for it, and that passes the
/// bound. So measuring a string costs time linear in what it adds to the
/// count, and nothing is measured past the bound: however long the names
/// are, and however many reports and segments a method has, measuring
/// costs at most the bound's bytes plus one string's.
struct Meter {
    counted: usize,
    limit: usize,
}

impl Meter {
    fn over(&self) -> bool {
        self.counted > self.limit
    }

    /// Count a string of `raw` bytes whose written length, at least `raw`,
    /// `exact` computes in time linear in `raw`.
    fn count(&mut self, raw: usize, exact: impl FnOnce() -> usize) {
        if self.over() {
            return;
        }
        let length = if raw > self.limit - self.counted {
            raw
        } else {
            exact()
        };
        self.counted = self.counted.saturating_add(length);
    }

    fn text(&mut self, text: &str) {
        self.count(text.len(), || json_len(text));
    }

    fn path(&mut self, path: &[Segment<'_>]) {
        render_parts(path, |part| {
            match part {
                Part::Name(name, prefix) => {
                    self.count(prefix.len() + name.len(), || name_segment_len(name, prefix))
                }
                Part::Text(part) => self.text(&part),
            }
            if self.over() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
    }
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

/// One diagnostic a method reports, before it is rendered: its text is
/// measured against [`MAX_CHECK_OUTPUT_BYTES`] first.
struct Report<'a> {
    code: &'static str,
    severity: &'static str,
    /// Outermost first.
    path: Option<Vec<Segment<'a>>>,
    message: String,
    resource_limit: Option<Exhausted>,
}

impl Report<'_> {
    fn error(code: &'static str, message: String) -> Self {
        Self {
            code,
            severity: "error",
            path: None,
            message,
            resource_limit: None,
        }
    }

    /// Count the bytes of its `method`, `message` and `path` strings, as the
    /// JSON response writes them, until the count passes the bound.
    fn measure(&self, method: &str, meter: &mut Meter) {
        meter.text(method);
        meter.text(&self.message);
        if let Some(path) = &self.path {
            meter.path(path);
        }
    }

    fn render(&self, method: &str) -> Value {
        let mut item = json!({
            "code": self.code,
            "severity": self.severity,
            "method": method,
        });
        if let Some(path) = &self.path {
            item["path"] = json!(render(path));
        }
        item["message"] = json!(self.message);
        if let Some(exhausted) = &self.resource_limit {
            item["resource_limit"] = resource_limit(exhausted);
        }
        item
    }
}

fn resource_limit(exhausted: &Exhausted) -> Value {
    json!({
        "resource": exhausted.resource,
        "limit": exhausted.limit,
        "observed": exhausted.observed,
    })
}

/// The resource of the whole check's work bound.
const TOTAL_STEPS: &str = "check_total_steps";

impl<'a> Checker<'a> {
    /// Decide one written method from empty state: its reports, or the whole
    /// check's work bound reached while deciding it.
    fn method(
        &mut self,
        name: &str,
        written_function: u32,
        live_methods: &BTreeMap<&str, u32>,
    ) -> Result<Vec<Report<'a>>, Exhausted> {
        // Deciding a method is one step of the whole check's bound, whatever
        // it costs on its own: so the bound caps the method count too.
        self.total_steps += 1;
        if self.total_steps > self.options.total_step_limit {
            return Err(Exhausted {
                resource: TOTAL_STEPS,
                limit: self.options.total_step_limit,
                observed: self.total_steps,
            });
        }
        let Some(live_function) = live_methods.get(name) else {
            return Ok(vec![Report::error(
                "method_missing",
                format!("the live service has no method {name:?}"),
            )]);
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
            self.written.node(written_function),
            self.live.node(*live_function),
        )
        else {
            unreachable!("a service method's type is a func in a valid Contract");
        };
        let mut reports = Vec::new();
        if written_mode != live_mode {
            reports.push(Report::error(
                "mode_changed",
                format!(
                    "the written method is {} and the live one is {}",
                    mode_name(*written_mode),
                    mode_name(*live_mode),
                ),
            ));
        }
        // Fresh maps rather than cleared ones: clearing a hash map costs its
        // capacity, which an earlier method may have grown far past what
        // this one uses. Dropping them costs only what this method spent.
        self.active = HashMap::new();
        self.frames.clear();
        self.proven = HashMap::new();
        self.log.clear();
        self.failed = BTreeMap::new();
        self.warnings.clear();
        self.truncated = false;
        self.path.clear();
        self.trail.clear();
        self.steps = 0;
        let outcome = self
            .tuple(Side::Written, written_args, live_args, Segment::Argument)
            .and_then(|()| self.tuple(Side::Live, live_results, written_results, Segment::Result));
        match outcome {
            Ok(()) => {}
            Err(Stop::Fails(failure)) => {
                let mut path = failure.segments;
                path.reverse();
                reports.push(Report {
                    path: Some(path),
                    ..Report::error("method_incompatible", failure.message)
                });
            }
            Err(Stop::Exhausted(exhausted)) if exhausted.resource == TOTAL_STEPS => {
                return Err(exhausted);
            }
            Err(Stop::Exhausted(exhausted)) => {
                reports.push(Report {
                    resource_limit: Some(exhausted.clone()),
                    ..Report::error(
                        "resource_limit_exceeded",
                        format!(
                            "the check of this method stopped at its {} bound of {}",
                            exhausted.resource, exhausted.limit,
                        ),
                    )
                });
                // Nothing a bounded walk recorded is complete; report the
                // bound alone for this method.
                self.warnings.clear();
                self.truncated = false;
            }
        }
        for warning in self.warnings.drain(..) {
            reports.push(Report {
                code: "special_opt_rule",
                severity: "warning",
                path: Some(warning.path),
                message: warning.message,
                resource_limit: None,
            });
        }
        if self.truncated {
            reports.push(Report {
                code: "resource_limit_exceeded",
                severity: "warning",
                path: None,
                message: format!(
                    "this method has more special_opt_rule warnings than its check_warnings bound of {MAX_CHECK_WARNINGS}; the rest are not reported, and the verdict stands",
                ),
                resource_limit: Some(Exhausted {
                    resource: "check_warnings",
                    limit: MAX_CHECK_WARNINGS,
                    observed: MAX_CHECK_WARNINGS + 1,
                }),
            });
        }
        Ok(reports)
    }
}

/// The diagnostics of `live <: written`, in written-method name order (code
/// point), each method's errors before its warnings. The check passes
/// exactly when no diagnostic has severity `error`. Both Contracts must have
/// an actor; the caller refuses one that does not.
///
/// Each method is decided on its own, until the check as a whole reaches one
/// of its aggregate bounds: from the method at which it does, every method
/// gets one `resource_limit_exceeded` error naming it, and nothing else.
pub fn check(written: &Input<'_>, live: &Input<'_>, options: CheckOptions) -> Vec<Value> {
    let written_methods = actor_methods(written.contract).expect("the caller checked the actor");
    let live_methods = actor_methods(live.contract).expect("the caller checked the actor");
    let live_methods: BTreeMap<&str, u32> = live_methods
        .iter()
        .map(|(name, function)| (name.as_str(), *function))
        .collect();
    let mut checker = Checker {
        live: Graph::new(live.contract, live.source_info),
        written: Graph::new(written.contract, written.source_info),
        options,
        active: HashMap::new(),
        frames: Vec::new(),
        proven: HashMap::new(),
        log: Vec::new(),
        failed: BTreeMap::new(),
        warnings: Vec::new(),
        truncated: false,
        path: Vec::new(),
        trail: Vec::new(),
        steps: 0,
        total_steps: 0,
    };
    let mut ordered = written_methods;
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    let mut diagnostics = Vec::new();
    // The aggregate bound reached, once one is.
    let mut stopped: Option<Exhausted> = None;
    let mut reported_warnings = 0usize;
    let mut reported_bytes = 0usize;
    for (name, written_function) in ordered {
        if stopped.is_none() {
            match checker.method(&name, written_function, &live_methods) {
                Ok(reports) => {
                    let warnings = reported_warnings
                        + reports
                            .iter()
                            .filter(|report| report.code == "special_opt_rule")
                            .count();
                    // Measured up to the first string that passes the bound,
                    // and no further.
                    let mut meter = Meter {
                        counted: reported_bytes,
                        limit: options.output_byte_limit,
                    };
                    for report in &reports {
                        report.measure(&name, &mut meter);
                        if meter.over() {
                            break;
                        }
                    }
                    let bytes = meter.counted;
                    if warnings > options.total_warning_limit {
                        stopped = Some(Exhausted {
                            resource: "check_total_warnings",
                            limit: options.total_warning_limit,
                            observed: warnings,
                        });
                    } else if meter.over() {
                        stopped = Some(Exhausted {
                            resource: "check_output_bytes",
                            limit: options.output_byte_limit,
                            observed: bytes,
                        });
                    } else {
                        reported_warnings = warnings;
                        reported_bytes = bytes;
                        diagnostics.extend(reports.iter().map(|report| report.render(&name)));
                        continue;
                    }
                }
                Err(exhausted) => stopped = Some(exhausted),
            }
        }
        let exhausted = stopped.as_ref().expect("an aggregate bound was reached");
        diagnostics.push(json!({
            "code": "resource_limit_exceeded",
            "severity": "error",
            "method": name,
            "message": format!(
                "the check of the whole service reached its {} bound of {}; this method fails closed, and nothing else is reported for it",
                exhausted.resource, exhausted.limit,
            ),
            "resource_limit": resource_limit(exhausted),
        }));
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
            Segment::Field(Some("Ok"), 1),
            Segment::Opt,
            Segment::Element,
            Segment::Field(None, 3),
            Segment::Field(Some("two words"), 9),
            Segment::Argument(1),
            Segment::Method("get"),
            Segment::Method("not ident"),
            Segment::Result(2),
        ];
        assert_eq!(
            render(&path),
            r#"$results[0].Ok?[*][3]["two words"]::args[1]::get::["not ident"]::results[2]"#
        );
        assert_eq!(render(&[Segment::Argument(0)]), "$args[0]");
        assert_eq!(render(&[]), "$");
        // The output bound measures exactly the rendered text, as JSON
        // writes it.
        let written =
            |path: &[Segment<'_>]| serde_json::to_string(&render(path)).unwrap().len() - 2;
        for prefix in 0..=path.len() {
            assert_eq!(rendered_len(&path[..prefix]), written(&path[..prefix]));
        }
        let escaped = [
            Segment::Result(0),
            Segment::Field(
                Some("tab\there \"quoted\" back\\slash \u{1} \u{7f} \u{e9}"),
                1,
            ),
            Segment::Method("line\nbreak"),
        ];
        assert_eq!(rendered_len(&escaped), written(&escaped));
        let every_byte: String = (0..=0x7f_u8)
            .map(char::from)
            .chain(['\u{e9}', '\u{1F600}'])
            .collect();
        assert_eq!(
            json_len(&every_byte),
            serde_json::to_string(&every_byte).unwrap().len() - 2
        );
        // A name of every byte, as a field and as a method, measured
        // without being built.
        let named = [
            Segment::Result(0),
            Segment::Field(Some(&every_byte), 1),
            Segment::Method(&every_byte),
        ];
        assert_eq!(rendered_len(&named), written(&named));
    }

    /// The meter stops at the first string that passes the bound, and a
    /// string longer than what is left is counted at its raw length, not
    /// measured: so measuring costs at most the bound plus one string.
    #[test]
    fn the_meter_stops_at_the_bound() {
        let quotes = "\"".repeat(1_000);
        let path = [
            Segment::Result(0),
            Segment::Field(Some(&quotes), 1),
            Segment::Opt,
        ];
        // `$results[0]`, then `["\"…"]` escaped again (4 bytes a quote),
        // then `?`.
        let exact = 11 + 2 + 4 + 4 * quotes.len() + 1;
        assert_eq!(rendered_len(&path), exact);
        let meter = |limit: usize| {
            let mut meter = Meter { counted: 0, limit };
            meter.path(&path);
            meter.counted
        };
        assert_eq!(meter(exact), exact);
        // Past the bound at the name: its raw 1,001 bytes, and nothing
        // after it.
        assert_eq!(meter(100), 11 + 1 + quotes.len());
        // Past the bound before the name: it is not counted at all.
        assert_eq!(meter(5), 11);
        // A text longer than what is left is not measured either.
        let mut meter = Meter {
            counted: 0,
            limit: 10,
        };
        meter.text(&quotes);
        assert_eq!(meter.counted, quotes.len());
        meter.text("more");
        assert_eq!(meter.counted, quotes.len());
    }
}
