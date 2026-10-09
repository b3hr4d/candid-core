//! TypeScript code generation over the `candid-core` Contract graph.
//!
//! This crate is the separate-crate half of the issue #38 decision: code
//! generation consumes the published Contract model and never influences it.
//! Depending on `candid-core` with `default-features = false` is the boundary
//! made structural — a generator needs no Candid parser, no filesystem
//! capability, and no host-value ABI.
//!
//! # Field names are inputs, not graph data
//!
//! The semantic Contract deliberately stores only the authoritative Candid
//! label ID on record and variant fields — names are provenance, held outside
//! the identity domain in the `SourceInfo` sidecar, which is `compiler`-feature
//! surface. A base-surface generator therefore takes names as caller-supplied
//! data: [`TsNames`] maps `(container, id)` to label text, and a field with no
//! entry renders by the ecosystem's `_id_` convention. With this crate's
//! `compiler` feature on, [`TsNames::from_source_info`] fills the table from a
//! compilation's provenance sidecar.
//!
//! # Scope
//!
//! All eighteen primitives, `opt`, `vec`, `record` (tuple-shaped records
//! become TypeScript tuples), `variant`, named declaration references with
//! recursion — and, since issue #104, the reference types: a `func` value is
//! the inert `{ principal, method }` reference (its signature lives in the
//! `c.func` builder), a `service` value is the principal of a running
//! service, and a contract with an actor exports `actor` (the
//! service schema) plus the type `Actor` (the typed call interface a call
//! layer built on the codec takes explicitly). A `class` declaration or
//! actor denotes its running service; init args are install-time metadata,
//! noted per declaration and not exposed. A `class` *nested* inside a value
//! type — which no Candid source can produce — fails closed with
//! [`TsGenError::UnsupportedConstruct`].
//!
//! # A clean domain model, not the wire shape
//!
//! By owner decision on issue #38, the output is the clean modern reading of
//! a Contract rather than the shapes the agent-js runtime produces: `opt T`
//! is `T | null`, variants are discriminated `{ tag, value }` unions, and
//! every `vec nat8` — `blob` — is `Uint8Array`. Two consequences are
//! deliberate.
//! An `opt` whose inner type can itself be `null` — another `opt`, `null`,
//! `reserved` — is *boxed*: `{ some: T } | null`, because `T | null` cannot
//! carry `None` versus `Some(None)` there. Only those opts box; `opt opt nat`
//! is `{ some: bigint | null } | null` while `opt nat` stays
//! `bigint | null`. The test is on the inner *node*, so an opt reached
//! through a declared alias — or through recursion, as in an opt node that is
//! its own inner — boxes exactly as the schema runtime's walkers and its
//! `OptDomain` type do. (That graph is `type L = opt L`; since issue #234 the
//! compiler refuses the source, so only a loaded or model-built Contract
//! holds it.)
//! And consuming these types against a live agent needs a boundary
//! conversion, which is future work recorded on the issue — the types
//! describe the domain, not the transport.
//!
//! # A declared primitive names only itself
//!
//! The Contract arena de-duplicates structurally identical nodes, so every use
//! of `nat64` in an interface is one node whichever declaration spelled it.
//! The generator renders a *composite* node by its first declaration's name —
//! a naming choice — but never a *primitive* one: since issue #191 every use
//! of a primitive renders structurally (`bigint`, `$.c.nat64`, `$.Principal`
//! for `principal`), and a declaration of it is emitted as itself.
//!
//! ```ts
//! // type Memo = nat64; type R = record { a : nat64; b : Memo };
//! type $Memo = bigint;
//! type $R = { a: bigint; b: bigint }; // not { a: Memo; b: Memo }
//! ```
//!
//! Before, one declaration renamed every use of its primitive across the whole
//! interface: `type Tokens = nat; type BlockIndex = nat` rendered a `Tokens`
//! field as `BlockIndex`, and `type Byte = nat8` turned every `blob` into
//! `Array<Byte>` — a value-domain change (`number[]` for `Uint8Array`) caused
//! by an unrelated declaration. A `blob`, and any `vec` of a `nat8` however
//! named, is always `Uint8Array` / `$.c.blob()`. The cost is the source
//! spelling: a field written `amount : Tokens` reads `amount: bigint`, the
//! same type. Composite nodes keep first-name rendering, as before; two
//! structurally equal *records* still collapse to the first name.
//!
//! # `.did` docs become JSDoc
//!
//! `TsNames::from_source_info` carries the sidecar's doc comments and
//! argument names to the generator (issue #191), which writes `/** … */`
//! above each exported type and const, on each record property and each
//! variant arm's `tag`, and on each method of the `Actor` type, with `@param`
//! tags for the argument names the `.did` wrote. The `Actor` method's own
//! parameters take those names; an unnamed argument, a reserved word, a name
//! that is not identifier-shaped, or a collision falls back to `arg{n}`, and
//! earns no `@param`.
//!
//! Candid's doc comment is the line comment: a `///` run (or plain `//` lines)
//! directly above a declaration, field, arm, method or the service. Block
//! comments are not docs. A node the arena has de-duplicated can carry several
//! occurrences' docs, and one rule picks: **the docs written inside the
//! declaration (or the actor) whose structure is being emitted** — the first
//! declaration for a node rendered by name, the containing declaration for an
//! anonymous one, the method's own occurrence in a signature. Occurrences
//! inside one origin that disagree are dropped, never merged. A
//! documented record or union spans several lines (Prettier's layout); one
//! with no docs keeps its one-line form.
//!
//! Doc text is neutralised, not interpreted: `*/` is written `*\/`, an `@`
//! that could start a tag or an inline link is written `\@`, and a run of three
//! backticks (a code fence the reader would never close) is written with each
//! backtick escaped — measured against the TypeScript compiler's own JSDoc
//! reader, so a `.did` comment can neither end its comment, nor forge a
//! `@param` or `@deprecated`, nor swallow the generator's own `@param` tags.
//! Tuple elements have no property to carry a doc and get none, and the
//! compiler-free [`TsNames::from_pairs`] surface carries no docs at all.
//!
//! # Each `Actor` method carries its mode
//!
//! Since issue #244 each method of `Actor` is its call signature intersected
//! with `$.WithMode<mode>` — `"query"`, `"composite_query"`, `"update"` or
//! `"oneway"`, the same literal the `c.func` builder in `actor` takes — so a
//! call layer tells a query from an update at compile time, from the `Actor`
//! type alone, and reads it back with the runtime's `ModeOf`:
//!
//! ```ts
//! type $Actor = {
//!   icrc1_balance_of: ((arg0: $Account) => Promise<bigint>) & $.WithMode<"query">;
//!   icrc1_transfer: ((arg0: $TransferArg) => Promise<$TransferResult>) & $.WithMode<"update">;
//! };
//! ```
//!
//! The mark is additive. `WithMode` is one optional property under a symbol
//! no value has, so the call signature is the one emitted before, `keyof
//! Actor` is unchanged, a plain async function still implements a method,
//! and the module's export names are unchanged — a separate mode map would
//! have taken a new export name from the declarations. An `Actor` written by
//! hand without the mark stays valid, and `ModeOf` reads its methods as the
//! whole `MethodMode` union: mode unknown.
//!
//! # Module layout: collision-free `$` bindings
//!
//! Since issue #188 every binding a generated module declares is a local
//! whose name starts with `$`, and each Candid name reaches consumers only
//! as an *export* name:
//!
//! ```ts
//! import * as $ from "@candid-core/schema";
//!
//! type $Account = { owner: $.Principal; subaccount: Uint8Array | null };
//! const $Account: $.Schema<$Account> = $.c.rec(() => $.c.record({ … }));
//! export { $Account as Account };
//!
//! const $actor: $.Schema<$.Principal> = $.c.rec(() => $.c.service({ … }));
//! type $Actor = {
//!   transfer: ((arg0: $TransferArg) => Promise<$TransferResult>) & $.WithMode<"update">;
//! };
//! export { $actor as actor, type $Actor as Actor };
//! ```
//!
//! The schema runtime is the namespace `$`; a declaration `X` is the local
//! `$X`, its alias and builder sharing that name, and `export { $X as X }`
//! exports both meanings. A declaration name is identifier-shaped
//! (`[A-Za-z_$][A-Za-z0-9_$]*`, a superset of Candid's identifier grammar),
//! so `$X` is always a valid binding and never a keyword, and prefixing is
//! injective — no declaration can shadow the runtime namespace, the ambient
//! types the lowerings use (`Array<T>`, `Record<string, never>`,
//! `Uint8Array`, `Promise<T>`), or another declaration. A declaration named
//! `c`, `Schema`, `Array`, `Promise`, or a TypeScript reserved word such as
//! `delete` or `string` therefore generates, and consumers import it under
//! its Candid spelling (`import { delete as del } from "./gen.ts"`). A
//! declaration named `default` becomes the module's default export.
//!
//! Only the module's own export names remain reserved: `actor` and `Actor`,
//! which the actor surface exports. A declaration by either name is omitted
//! ([`OmissionReason::ReservedExportName`]), unconditionally — with or
//! without an actor — so what a declaration generates never depends on
//! another part of the contract (the #116 locality rule).
//!
//! When [`TsOptions::principal_import`] names the schema runtime (the
//! default), the principal type is `$.Principal`; any other module is
//! imported as `import type { Principal } from "…"`, a local that no
//! `$`-prefixed declaration binding can collide with — so a declaration
//! named `Principal` generates either way.
//!
//! # Omission instead of refusal
//!
//! Since issue #189 a declaration the module cannot represent costs only
//! itself and what depends on it, not the whole interface.
//! [`generate_module`] returns a [`GeneratedModule`]: the module text and the
//! [`Omission`]s it made. Four causes make a declaration unrepresentable
//! ([`OmissionReason`]): a record field or variant arm name shaped like the
//! `_N_` id rendering, a variant arm whose payload is a declared `opt` of a
//! never-domain type, a name that is one of the module's export names, and —
//! from a Contract document only — a name that is not identifier-shaped. A
//! cause inside anonymous structure (`record { nested : variant { _1_ : nat
//! } }`) omits the declaration or actor method whose structure holds it.
//!
//! Omission then spreads over reverse type edges — every one, through nested
//! `func` and `service` types included — to the *containing declaration*: a
//! record holding `service { f : (Bad) -> () }` is omitted whole, because a
//! service value is encoded with its full method table and dropping `f` would
//! change the wire type a peer sees
//! ([`OmissionReason::ReferencesOmitted`], with [`Omission::via`] naming the
//! omitted declaration). Only the actor's own service drops individual
//! methods, from both the `actor` schema and the `Actor` type: calling a
//! method never encodes the actor's service type. The actor itself is never
//! omitted: when it is written `service : S` and the declaration `S` is
//! omitted, its surviving methods render inline; and a class actor's init
//! args, which are never rendered, omit nothing from it.
//!
//! ```ts
//! // type Bad = record { _0_ : nat }; type Holder = record { bad : Bad };
//! // service : { ok : () -> (); bad : (Bad) -> () }
//! // Generated by candid-core-ts from a candid-core Contract. Do not edit.
//! // Omitted: type Bad (reserved_field_name)
//! // Omitted: type Holder (references_omitted via Bad)
//! // Omitted: method bad (references_omitted via Bad)
//! ```
//!
//! The header lists every omission, declarations first, then methods, each
//! by name; a module that omits nothing is byte-identical to its pre-#189
//! text. Nothing emitted references an omitted declaration, and everything
//! emitted is byte-identical to what the module would hold had the omitted
//! declarations never been written (both pinned by the `omissions` golden).
//! `via` names the omitted declaration the entry's structure reaches first
//! on a shortest path to a cause (edge order breaks ties); the exact rule
//! lives in the `omissions` module and in its mirror in `schemaFromContract`,
//! which omits the same entries for the same reasons. [`TsGenError`] is left
//! for an invalid Contract graph — the refusals `schemaFromContract` also
//! makes whole-document (issue #129) — and for a cycle through no
//! declaration, which the loader accepts (see "Bounded generation" below).
//!
//! # Determinism
//!
//! Identical Contracts produce byte-identical TypeScript and an identical
//! omitted list: emission follows the Contract's canonical declaration and
//! field order, and nothing in the output depends on time, environment, or
//! map iteration order.
//!
//! # Bounded generation: constant stack, named cycles
//!
//! Generation uses constant call-stack depth in the Contract's nesting
//! (issue #218). The renderer keeps the composites it is in the middle of on
//! an explicit stack rather than the call stack, as ADR 0005 asks of every
//! graph walk, so a validated Contract of any depth generates on a small
//! thread: the 256-level types the compiler accepts, and the far deeper
//! chains `Contract::from_json` accepts at default or raised limits. The
//! tests run generation on a 64 KiB thread (512 KiB on Windows) in the dev
//! and release profiles. Depth costs heap, not stack; output size is a
//! separate matter, and this crate does not bound it.
//!
//! That walk ends only because of the cycle rule: **every cycle the module
//! renders passes through a declared node.** A declared node renders as its
//! declaration's name, which is how `type L = record { next : opt L }` is
//! written at all; an
//! undeclared node renders its structure in place, so a cycle through no
//! declaration would never end, and a TypeScript type alias has no spelling
//! for an anonymous cycle. Every Contract compiled from Candid source keeps
//! the rule, since a recursive Candid type needs a name. Contract validation
//! does not require it, so a Contract read from a document or built from a
//! draft can break it, and the generator refuses that Contract whole, before
//! rendering anything, with [`TsGenError::UndeclaredCycle`]. The rule covers
//! exactly what the renderer walks — the bodies of the declarations the
//! module emits, the actor's service and its surviving methods' signatures —
//! so a cycle the module never renders does not refuse: one in a class
//! actor's init args (not rendered, issue #104), or one reachable only from
//! an omitted declaration or method. Every Contract that generated before
//! issue #218 still generates, byte-identically. The reported node is the
//! first found that closes a cycle, by a depth-first search from those roots
//! in render order, children in edge order.
//! `schemaFromContract` accepts the same graph, because it builds every edge
//! lazily and needs no names; it is the one refusal the two do not share.
//!
//! # What is deliberately not claimed
//!
//! `@icp-sdk/bindgen` is a differential *oracle* for type-level agreement in a
//! later slice, never the specification; byte-identical output is a non-goal.
//! No performance claims are made. The Contract format is not stable v1, and
//! this crate tracks it as a pre-1.0 consumer pinned to an exact version.

use std::collections::BTreeMap;
use std::fmt;

use candid_core::{Contract, Field, PrimitiveType, ServiceMethod, TypeNode, TypeRef};

mod cycles;
mod docs;
mod omissions;

use docs::{doc_block, resolve_parameters, Origin, Parameter, Provenance};

/// Caller-supplied provenance: field label text keyed by `(container node,
/// label id)`, and — when built from a compilation — the `.did` doc comments
/// and argument names the generator renders as JSDoc (issue #191).
///
/// The semantic Contract stores only label IDs; see the crate docs for why.
/// Docs and argument names are provenance of the same kind and travel the
/// same way, so one table carries all three: [`TsNames::new`] and
/// [`TsNames::from_pairs`] build the compiler-free base surface, which names
/// fields and documents nothing, and `TsNames::from_source_info` fills
/// everything the sidecar records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsNames {
    labels: BTreeMap<(TypeRef, u32), String>,
    provenance: Provenance,
}

impl TsNames {
    /// An empty table: every field renders by the `_id_` convention.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the label text for field `id` of the container node `container`.
    pub fn insert(&mut self, container: TypeRef, id: u32, label: impl Into<String>) {
        self.labels.insert((container, id), label.into());
    }

    /// Build a table from `(container, id, label)` triples.
    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (TypeRef, u32, S)>,
        S: Into<String>,
    {
        let mut names = Self::new();
        for (container, id, label) in pairs {
            names.insert(container, id, label);
        }
        names
    }

    /// Fill the table from a compilation's provenance sidecar.
    ///
    /// Only named labels are recorded: a numeric Candid label carries no name,
    /// and its provenance entry must not override the `_id_` rendering. The
    /// sidecar's doc comments and argument names are recorded too, and become
    /// JSDoc; see the crate docs for the occurrence rule that picks which
    /// occurrence documents a node the arena has de-duplicated.
    #[cfg(feature = "compiler")]
    pub fn from_source_info(source_info: &candid_core::SourceInfo) -> Self {
        let mut names = Self::new();
        for provenance in source_info.field_labels() {
            if let candid_core::SourceLabel::Named { name } = &provenance.label {
                names.insert(provenance.container, provenance.id, name.clone());
            }
        }
        names.provenance = Provenance::from_source_info(source_info);
        names
    }

    fn get(&self, container: TypeRef, id: u32) -> Option<&str> {
        self.labels.get(&(container, id)).map(String::as_str)
    }
}

/// Generation options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsOptions {
    /// The module the `Principal` type is imported from when a Contract uses
    /// a principal (the primitive, a func reference, a service reference).
    /// Since issue #187 that type is the schema runtime's own `Principal` —
    /// canonical principal text as a branded string, the value the codec
    /// decodes and the only one it encodes — not the SDK class, which the
    /// runtime never constructs or accepts. At the default — the schema
    /// runtime itself — the type is referenced through the module's `$`
    /// namespace (`$.Principal`). Any other module is imported only when
    /// used, as `import type { Principal }`, so it never implies a runtime
    /// dependency; it must re-export the schema runtime's `Principal`
    /// itself, because the brand makes that type nominal and the invariant
    /// `$.Schema<…>` annotations compile against no other type.
    pub principal_import: String,
}

/// The module specifier every generated module imports the schema runtime
/// from, as the namespace `$`.
const SCHEMA_MODULE: &str = "@candid-core/schema";

impl Default for TsOptions {
    fn default() -> Self {
        Self {
            principal_import: SCHEMA_MODULE.to_string(),
        }
    }
}

/// A refusal of the whole Contract. Since issue #189 only a Contract graph
/// the emitter cannot walk refuses: a declaration the module cannot
/// represent is omitted instead (see [`Omission`]), and no error path emits
/// placeholder TypeScript.
///
/// Every public [`Contract`] is validated on construction, so
/// [`UnsupportedConstruct`](Self::UnsupportedConstruct) and
/// [`DanglingTypeRef`](Self::DanglingTypeRef) are not reachable through this
/// crate's API today; they guard the graph invariants the emitter relies on,
/// and refuse exactly the documents `schemaFromContract` refuses (class
/// placement, issue #129; dangling references).
/// [`UndeclaredCycle`](Self::UndeclaredCycle) *is* reachable — from a
/// validated Contract no Candid source produced (issue #218) — and is the one
/// refusal `schemaFromContract` does not share. Refusals are checked in that
/// order: a declared class and dangling references first, then the cycle
/// rule, then a class nested in a value type as rendering meets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TsGenError {
    /// A `class` type is named by a declaration or nested inside a value
    /// type — candid-core's `class_not_actor_root` and
    /// `class_not_first_class_type`, shapes no Candid source can produce
    /// (classes exist only at actor position). `func` and `service` generate
    /// since issue #104.
    UnsupportedConstruct {
        declaration: String,
        kind: &'static str,
    },
    /// A type reference points outside the Contract arena. A validated
    /// Contract cannot contain one; this guards the unvalidated path.
    DanglingTypeRef { reference: TypeRef },
    /// A cycle the module would render passes through no declared node;
    /// `reference` is a node on it (see "Bounded generation" in the crate
    /// docs for which, and for the cycles the module never renders). A
    /// generated type alias has no spelling for an anonymous cycle, and the
    /// renderer, which stops only at declared nodes, would expand it without
    /// end. No Candid source produces one — a recursive Candid type needs a
    /// name — but Contract validation accepts any cycle, so a Contract read
    /// with `Contract::from_json` or built from a `ContractDraft` can hold
    /// one. `schemaFromContract` accepts the same graph: the loader needs no
    /// name for a node.
    UndeclaredCycle { reference: TypeRef },
}

impl fmt::Display for TsGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedConstruct { declaration, kind } => write!(
                f,
                "declaration `{declaration}` nests a `{kind}` type, which \
                 exists only at declaration or actor position (issue #104)"
            ),
            Self::DanglingTypeRef { reference } => {
                write!(
                    f,
                    "type reference {reference} is outside the Contract arena"
                )
            }
            Self::UndeclaredCycle { reference } => write!(
                f,
                "type node {reference} lies on a cycle that passes through no \
                 declaration; a TypeScript type alias cannot spell an anonymous \
                 cycle (issue #218)"
            ),
        }
    }
}

impl std::error::Error for TsGenError {}

/// A generated module and what it had to leave out (issue #189).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedModule {
    /// The complete TypeScript module text. With nothing omitted it is
    /// byte-identical to what the generator emitted before issue #189; with
    /// omissions, its header lists them, one `// Omitted:` line each.
    pub module: String,
    /// Every declaration and actor method left out, declarations first,
    /// then methods, each group sorted by name (byte order).
    pub omitted: Vec<Omission>,
}

/// One declaration or actor method the module leaves out, and why.
///
/// A declaration the module cannot represent is omitted together with
/// everything that references it — through any type edge, nested `func` and
/// `service` types included, up to the *containing declaration* — because
/// dropping a method from a service type in value position would change the
/// wire type a peer sees. Only the actor's own service drops individual
/// methods: calling a method never encodes the actor's service type. The
/// serialized form (`@candid-core/cli`'s `ModuleSuccess.omitted`) is
/// `{ "kind", "name", "reason", "via"? }` with the snake_case codes below.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Omission {
    /// A declaration, or a method of the actor's service.
    pub kind: OmissionKind,
    /// The declaration or method name, as the Contract spells it.
    pub name: String,
    /// Why it was left out.
    pub reason: OmissionReason,
    /// For [`OmissionReason::ReferencesOmitted`], the omitted declaration
    /// the entry references (the first one found, see the crate docs); for
    /// every other reason, `None`.
    pub via: Option<String>,
}

/// What an [`Omission`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OmissionKind {
    /// A named declaration: its alias, builder and export are not emitted.
    Declaration,
    /// A method of the actor's service: absent from both the `actor` schema
    /// and the `Actor` type.
    Method,
}

impl OmissionKind {
    /// The stable serialized code: `declaration` or `method`.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Method => "method",
        }
    }
}

/// Why an [`Omission`] was left out. A closed set: adding a reason is an API
/// change. The serialized codes are snake_case, like every other issue code
/// the repository emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OmissionReason {
    /// `reserved_field_name`: a record field or variant arm name shaped like
    /// the `_N_` id rendering (canonical decimal below 2^32). Erased to a
    /// schema key it is indistinguishable from the rendering of numeric label
    /// id N, so the codec would derive the wrong wire id from it — the same
    /// reservation `schemaFromContract` enforces (issues #103, #115).
    ReservedFieldName,
    /// `ambiguous_variant_arm`: a variant arm whose payload is a *declared*
    /// `opt` of a never-domain type (`opt empty`, `opt` of an empty variant).
    /// The arm renders as a bare reference whose static type is
    /// `Schema<null>` — indistinguishable from a declared alias of `null`,
    /// which `VariantInfer` must classify as a bare tag — while the emitter
    /// and runtime classify the arm by its node and demand a `value`. No
    /// type-level rule can satisfy both (issue #127). The anonymous form
    /// (`variant { a : opt empty }`) generates: its `OptSchema<never>` type
    /// carries the classification.
    AmbiguousVariantArm,
    /// `reserved_export_name`: a declaration named after one of the module's
    /// own export names, `actor` or `Actor`, which the actor surface exports
    /// (issue #188). Omitted with or without an actor, by the #116 locality
    /// rule: adding an actor must not change what an unrelated declaration
    /// generates.
    ReservedExportName,
    /// `invalid_declaration_name`: a declaration name that is not
    /// identifier-shaped (`[A-Za-z_$][A-Za-z0-9_$]*`), so `$` plus the name
    /// is not a binding. Candid source cannot produce one; a Contract
    /// document, which admits any non-empty name, can.
    InvalidDeclarationName,
    /// `references_omitted`: the entry's structure references an omitted
    /// declaration, named by [`Omission::via`], whose name the module would
    /// otherwise use without defining it.
    ReferencesOmitted,
}

impl OmissionReason {
    /// The stable serialized code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::ReservedFieldName => "reserved_field_name",
            Self::AmbiguousVariantArm => "ambiguous_variant_arm",
            Self::ReservedExportName => "reserved_export_name",
            Self::InvalidDeclarationName => "invalid_declaration_name",
            Self::ReferencesOmitted => "references_omitted",
        }
    }
}

impl fmt::Display for Omission {
    /// The entry as the module header lists it, without the comment marker:
    /// `type Holder (references_omitted via Bad)`. Names that are not
    /// identifier-shaped are quoted, and nothing in a name can end the line
    /// comment it is written into.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            OmissionKind::Declaration => "type",
            OmissionKind::Method => "method",
        };
        write!(
            f,
            "{kind} {} ({}",
            comment_name(&self.name),
            self.reason.code()
        )?;
        if let Some(via) = &self.via {
            write!(f, " via {}", comment_name(via))?;
        }
        f.write_str(")")
    }
}

/// Generate a TypeScript module from a Contract.
///
/// See the crate docs for scope, determinism, omissions, and the role of
/// [`TsNames`].
pub fn generate_module(
    contract: &Contract,
    names: &TsNames,
    options: &TsOptions,
) -> Result<GeneratedModule, TsGenError> {
    let principal = if options.principal_import == SCHEMA_MODULE {
        "$.Principal"
    } else {
        "Principal"
    };
    let declared = first_names(contract);
    let analysis = omissions::analyze(contract, names, &declared)?;
    cycles::check(contract, &declared, &analysis)?;
    let module = Generator {
        contract,
        names,
        declared,
        principal,
        uses_principal: false,
        origins: Vec::new(),
        indent: 0,
    }
    .module(options, &analysis)?;
    Ok(GeneratedModule {
        module,
        omitted: analysis.omitted,
    })
}

/// The actor's service node, and whether the actor is a service class, whose
/// init args the module does not render (issue #104). `None` without an
/// actor. Shared by the emitter and the cycle rule, which must agree on what
/// the actor surface walks.
fn actor_service(contract: &Contract) -> Result<Option<(TypeRef, bool)>, TsGenError> {
    let Some(actor) = contract.actor() else {
        return Ok(None);
    };
    Ok(Some(match actor {
        candid_core::Actor::Service { service } => (*service, false),
        candid_core::Actor::Class { class } => match omissions::node(contract, *class)? {
            TypeNode::Class { service, .. } => (*service, true),
            _ => (*class, false),
        },
    }))
}

/// The module-local binding of a declaration: `$` plus its name. Candid
/// names are identifier-shaped, so the local is a valid binding, never a
/// keyword, and never equal to the runtime namespace `$`, an ambient type,
/// or another declaration's local (see "Module layout" in the crate docs).
fn local(name: &str) -> String {
    format!("${name}")
}

/// The first declaration name for each *composite* node, in declaration
/// order. Later aliases of the same node render as references to the first
/// name.
///
/// A primitive node is never in the map. The Contract arena de-duplicates
/// structurally identical nodes, so every `nat64` in an interface is one node
/// whichever declaration spells it — and a name recorded for it would be
/// rendered at *every* use of that primitive, including the ones that never
/// wrote the name (issue #191): `type Memo = nat64` would turn an unrelated
/// `nat64` field into `Memo`, and `type Byte = nat8` would turn every `blob`
/// into `Array<Byte>`. A primitive declaration is still emitted, structurally
/// (`type $Memo = bigint`); it just names nothing but itself.
fn first_names(contract: &Contract) -> BTreeMap<TypeRef, String> {
    let mut map = BTreeMap::new();
    for declaration in contract.declarations() {
        if matches!(
            contract.types().get(declaration.ty as usize),
            Some(TypeNode::Primitive { .. })
        ) {
            continue;
        }
        map.entry(declaration.ty)
            .or_insert_with(|| declaration.name.clone());
    }
    map
}

/// One traversal, two syntaxes. `Alias` renders the static type expression;
/// `Builder` renders the runtime schema expression the alias annotates. Every
/// guard — deferred constructs, dangling refs — and every shape decision —
/// boxed options included — runs before this dispatch, so the two outputs
/// can never disagree about what is representable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Alias,
    Builder,
}

struct Generator<'a> {
    contract: &'a Contract,
    names: &'a TsNames,
    declared: BTreeMap<TypeRef, String>,
    /// The principal type expression: `$.Principal`, or the imported
    /// `Principal` for a non-default [`TsOptions::principal_import`].
    principal: &'static str,
    uses_principal: bool,
    /// The origins the doc lookups of the structure being emitted are scoped
    /// to, in priority order (see `docs`): the declaration being emitted, or
    /// for an `Actor` method the method's own occurrence and then the
    /// declaration of its function type.
    origins: Vec<Origin>,
    /// The nesting level, in two-space units, of the line a rendered alias
    /// expression starts on. Only an alias containing docs spans lines, and
    /// its members indent from here.
    indent: usize,
}

/// One member of an object type in an alias: its docs and its text.
struct Member {
    docs: Vec<String>,
    text: String,
}

/// What entering a node yields: its whole text, or the frame that collects
/// its children's.
enum Entered {
    Text(String),
    Frame(Frame),
}

/// One composite on the renderer's explicit stack (issue #218): the node's
/// own data, the pieces rendered so far, and what the recursive renderer kept
/// in locals — the alias indent level to restore. The next child is the one
/// after the pieces collected so far, in edge order.
enum Frame {
    Opt {
        inner: TypeRef,
        boxed: bool,
        rendered: Option<String>,
    },
    Vec {
        inner: TypeRef,
        rendered: Option<String>,
    },
    Tuple {
        fields: Vec<Field>,
        elements: Vec<String>,
    },
    Record {
        reference: TypeRef,
        fields: Vec<Field>,
        level: usize,
        members: Vec<Member>,
    },
    Variant {
        reference: TypeRef,
        fields: Vec<Field>,
        level: usize,
        arms: Vec<Vec<Member>>,
    },
    Func {
        arguments: usize,
        children: Vec<TypeRef>,
        rendered: Vec<String>,
        mode: candid_core::MethodMode,
    },
    Service {
        methods: Vec<ServiceMethod>,
        members: Vec<String>,
    },
}

impl Generator<'_> {
    fn module(
        mut self,
        options: &TsOptions,
        analysis: &omissions::Analysis,
    ) -> Result<String, TsGenError> {
        let mut aliases = Vec::new();

        for declaration in self.contract.declarations() {
            // An omitted declaration emits nothing, and — by the closure the
            // analysis computed — nothing emitted below references it.
            if analysis.omits_declaration(&declaration.name) {
                continue;
            }
            let target_ty = declaration.ty;
            // The alias is the reviewed type; the builder is the
            // runtime schema annotated with it. `Schema<T>` is
            // invariant, so the annotation makes `tsc` prove the
            // builder infers exactly the alias — the golden type-check
            // is the equality gate, not a convention.
            //
            // Every declaration is wrapped in `c.rec`: emission order
            // is canonical (name-sorted), not dependency-sorted, and
            // the lazy thunk is what makes a forward reference safe at
            // module initialization.
            //
            // Both meanings bind the `$`-prefixed local and leave under the
            // Candid name through one export specifier (issue #188).
            self.origins = vec![Origin::of(&declaration.name)];
            self.indent = 0;
            let alias = self.declaration_body(target_ty, &declaration.name, Target::Alias)?;
            let builder = self.declaration_body(target_ty, &declaration.name, Target::Builder)?;
            // The declaration's docs sit above both meanings of the local:
            // a hover on either the type or the value reads them, and the
            // `export` specifier carries whichever the consumer uses.
            let docs = doc_block(
                0,
                self.names.provenance.declaration_docs(&declaration.name),
                &[],
            );
            let local = local(&declaration.name);
            aliases.push(format!(
                "{docs}{}\n{docs}const {local}: $.Schema<{local}> = $.c.rec(() => {builder});\nexport {{ {local} as {name} }};\n",
                assign(&format!("type {local}"), &alias) + ";",
                name = declaration.name,
            ));
        }

        // The actor surface: the service schema plus the directly-emitted
        // call interface — one async method per service method, the reply
        // convention being zero results ⇒ void, one ⇒ the value, several ⇒ a
        // tuple. A call layer takes `Actor` explicitly, because `c.rec` erases
        // method structure from a schema's type.
        let mut actor_out = String::new();
        if let Some((service_ty, is_class)) = actor_service(self.contract)? {
            let mut methods = match self.node(service_ty)? {
                TypeNode::Service { methods } => methods.clone(),
                _ => Vec::new(),
            };
            // A method whose signature references an omitted declaration (or
            // holds an unrepresentable anonymous type) leaves both surfaces
            // below together: it is dropped here, before either renders.
            methods.retain(|method| !analysis.omits_method(&method.name));
            let builder = if analysis.is_tainted(service_ty) {
                // Something under the actor's service is omitted, so the
                // service cannot be referenced by a declaration's name — its
                // first declaration, if it has one, is omitted too. Render the
                // surviving methods inline, exactly as `render_structure`
                // renders a service builder. Sound only for the actor itself:
                // calling a method never encodes the actor's service type.
                let mut members = Vec::with_capacity(methods.len());
                for method in &methods {
                    let value = self.render(method.function, "actor", Target::Builder)?;
                    members.push(format!("{}: {value}", method_key(&method.name)));
                }
                format!("$.c.service({{ {} }})", members.join(", "))
            } else {
                self.declaration_body(service_ty, "actor", Target::Builder)?
            };
            self.uses_principal = true;
            // A method's docs and argument names come from the service's
            // occurrence in the actor position, else from the first
            // declaration the generator emits for the service node (an actor
            // written `service : S`), by the same occurrence rule as fields.
            let mut method_origins = vec![Origin::Actor];
            if let Some(first) = self.declared.get(&service_ty) {
                method_origins.push(Origin::Declaration(first.clone()));
            }
            let mut signatures = Vec::with_capacity(methods.len());
            for method in &methods {
                let (func, mode) = match self.node(method.function)? {
                    TypeNode::Func {
                        args,
                        results,
                        mode,
                    } => ((args.clone(), results.clone()), Some(*mode)),
                    _ => ((Vec::new(), Vec::new()), None),
                };
                let (docs, parameters, origins) =
                    self.method_provenance(&method_origins, service_ty, method, func.0.len());
                // Anonymous types in the signature are the method's own
                // occurrence; method members sit one level in.
                self.origins = origins;
                self.indent = 1;
                let mut params = Vec::with_capacity(func.0.len());
                for (arg, parameter) in func.0.iter().zip(&parameters) {
                    params.push(format!(
                        "{}: {}",
                        parameter.name,
                        self.render(*arg, "actor", Target::Alias)?
                    ));
                }
                let reply = match func.1.len() {
                    0 => "void".to_string(),
                    1 => self.render(func.1[0], "actor", Target::Alias)?,
                    _ => {
                        let mut parts = Vec::with_capacity(func.1.len());
                        for result in &func.1 {
                            parts.push(self.render(*result, "actor", Target::Alias)?);
                        }
                        format!("[{}]", parts.join(", "))
                    }
                };
                let tags: Vec<String> = parameters
                    .iter()
                    .filter(|parameter| parameter.declared)
                    .map(|parameter| parameter.name.clone())
                    .collect();
                // The method's mode rides on its type as a phantom
                // intersection: the call signature is the one a call layer
                // has always read, and `$.ModeOf` reads the mode back. A
                // method whose node is not a func (no validated Contract
                // holds one) carries none, which reads as mode unknown.
                let signature =
                    format!("({params}) => Promise<{reply}>", params = params.join(", "));
                let ty = match mode {
                    Some(mode) => format!("({signature}) & $.WithMode<\"{}\">", mode_text(mode)),
                    None => signature,
                };
                signatures.push(format!(
                    "{}  {key}: {ty};",
                    doc_block(1, &docs, &tags),
                    key = method_key(&method.name),
                ));
            }
            let docs = doc_block(0, self.names.provenance.actor_docs(), &[]);
            actor_out.push_str(&format!(
                "{docs}const $actor: $.Schema<{principal}> = $.c.rec(() => {builder});\n",
                principal = self.principal,
            ));
            actor_out.push_str(&format!(
                "{docs}type $Actor = {{\n{}\n}};\n",
                signatures.join("\n")
            ));
            actor_out.push_str("export { $actor as actor, type $Actor as Actor };\n");
            if is_class {
                actor_out.push_str(
                    "// Note: the actor is a service class; init args are install-time \
                     metadata not exposed here (issue #104).\n",
                );
            }
        }

        let mut out = String::from(
            "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n",
        );
        // The omissions, in `GeneratedModule::omitted` order. A module that
        // omits nothing gains no line, so its text is unchanged by issue #189.
        for omission in &analysis.omitted {
            out.push_str(&format!("// Omitted: {omission}\n"));
        }
        if !aliases.is_empty() || !actor_out.is_empty() {
            out.push_str(&format!(
                "import * as $ from {};\n",
                quote_string(SCHEMA_MODULE)
            ));
        }
        if self.uses_principal && options.principal_import != SCHEMA_MODULE {
            out.push_str(&format!(
                "import type {{ Principal }} from {};\n",
                quote_string(&options.principal_import)
            ));
        }
        if !aliases.is_empty() {
            out.push('\n');
            out.push_str(&aliases.join("\n"));
        }
        if !actor_out.is_empty() {
            out.push('\n');
            out.push_str(&actor_out);
        }
        Ok(out)
    }

    /// The right-hand side of one alias. A declaration whose node is first
    /// declared under a *different* name renders as that name, so later
    /// aliases of one node stay aliases instead of duplicating structure.
    fn declaration_body(
        &mut self,
        ty: TypeRef,
        own_name: &str,
        target: Target,
    ) -> Result<String, TsGenError> {
        match self.declared.get(&ty) {
            Some(first) if first != own_name => Ok(local(first)),
            _ => self.render_structure(ty, own_name, target),
        }
    }

    /// The docs, parameter names and doc origins for one `Actor` method.
    ///
    /// The method's occurrence is the first of `origins` that wrote it. Its
    /// argument names are those written on its own inline function type; a
    /// method typed by a reference (`f : Handler`) writes none there, so they
    /// come from the first declaration the generator emits for the function
    /// node. The origins returned scope the doc lookups of the anonymous
    /// types in the signature in the same order: the method's own occurrence
    /// first, then that declaration.
    fn method_provenance(
        &self,
        origins: &[Origin],
        service: TypeRef,
        method: &ServiceMethod,
        arguments: usize,
    ) -> (Vec<String>, Vec<Parameter>, Vec<Origin>) {
        let provenance = &self.names.provenance;
        let occurrence = origins.iter().find_map(|origin| {
            provenance
                .method(origin, service, &method.name)
                .map(|(docs, path)| (origin, docs, path))
        });
        let mut sources: Vec<(Origin, String)> = Vec::new();
        let mut scope: Vec<Origin> = Vec::new();
        if let Some((origin, _, path)) = &occurrence {
            scope.push((*origin).clone());
            if let Some(path) = path {
                sources.push(((*origin).clone(), format!("{path}.function")));
            }
        }
        if let Some(first) = self.declared.get(&method.function) {
            scope.push(Origin::Declaration(first.clone()));
            sources.push((Origin::Declaration(first.clone()), format!("type:{first}")));
        }
        let declared: Vec<Option<String>> = (0..arguments)
            .map(|position| {
                sources.iter().find_map(|(origin, path)| {
                    provenance
                        .argument_name(origin, path, method.function, position)
                        .map(str::to_string)
                })
            })
            .collect();
        let docs = occurrence
            .map(|(_, docs, _)| docs.to_vec())
            .unwrap_or_default();
        (docs, resolve_parameters(&declared), scope)
    }

    /// An object type from its members. Single-line when nothing in it spans
    /// lines — the form every doc-free alias has always had — and one member
    /// per line, each preceded by its docs, otherwise. `level` is the
    /// nesting level of the line the opening brace is on.
    fn object_type(members: &[Member], level: usize) -> String {
        if members
            .iter()
            .all(|member| member.docs.is_empty() && !member.text.contains('\n'))
        {
            let texts: Vec<&str> = members.iter().map(|member| member.text.as_str()).collect();
            return format!("{{ {} }}", texts.join("; "));
        }
        let mut out = String::from("{\n");
        for member in members {
            out.push_str(&doc_block(level + 1, &member.docs, &[]));
            out.push_str(&"  ".repeat(level + 1));
            out.push_str(&member.text);
            out.push_str(";\n");
        }
        out.push_str(&"  ".repeat(level));
        out.push('}');
        out
    }

    fn node(&self, reference: TypeRef) -> Result<&TypeNode, TsGenError> {
        self.contract
            .types()
            .get(reference as usize)
            .ok_or(TsGenError::DanglingTypeRef { reference })
    }

    /// Render a type expression. A declared node renders as its first name;
    /// anything else renders its structure. Since issue #218 two things that
    /// were assumed are enforced: every cycle the renderer walks passes
    /// through a declared node
    /// (`cycles::check`, run before any rendering, refuses the Contract
    /// otherwise), so expanding undeclared structure always reaches a leaf or
    /// a name; and the walk keeps its pending work on an explicit stack
    /// ([`Frame`]), so its call-stack depth is constant in the nesting.
    fn render(
        &mut self,
        reference: TypeRef,
        declaration: &str,
        target: Target,
    ) -> Result<String, TsGenError> {
        self.walk(reference, true, declaration, target)
    }

    /// Render a node's own structure, even when it is declared: the
    /// right-hand side of its declaration. Its children render as [`render`]
    /// does.
    ///
    /// [`render`]: Self::render
    fn render_structure(
        &mut self,
        reference: TypeRef,
        declaration: &str,
        target: Target,
    ) -> Result<String, TsGenError> {
        self.walk(reference, false, declaration, target)
    }

    /// The renderer: a loop over an explicit stack of [`Frame`]s, one per
    /// composite being rendered, each holding what was the continuation of a
    /// recursive call (issue #218, ADR 0005). Children are entered in edge
    /// order and every state change a recursive call made around a child —
    /// the alias indent — is made and undone at the same points, so the text,
    /// and the first error when there is one, are those of a depth-first
    /// recursive rendering.
    fn walk(
        &mut self,
        root: TypeRef,
        by_name: bool,
        declaration: &str,
        target: Target,
    ) -> Result<String, TsGenError> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut entering = Some((root, by_name));
        let mut finished: Option<String> = None;
        loop {
            if let Some((reference, by_name)) = entering.take() {
                match self.enter(reference, by_name, declaration, target)? {
                    Entered::Text(text) => finished = Some(text),
                    Entered::Frame(frame) => {
                        stack.push(frame);
                        // Every frame but the root's is an undeclared node,
                        // and with the cycle rule checked no path repeats
                        // one, so the stack never outgrows the arena. Should
                        // the rule ever go unchecked, this ends the walk
                        // instead of letting it consume memory forever.
                        assert!(
                            stack.len() <= self.contract.types().len(),
                            "the renderer met a cycle through no declaration, \
                             which `cycles::check` refuses before rendering"
                        );
                    }
                }
            }
            let Some(frame) = stack.last_mut() else {
                return Ok(finished.expect("an empty stack follows a finished expression"));
            };
            if let Some(text) = finished.take() {
                self.accept(frame, text, target);
            }
            match self.next_child(frame, target)? {
                Some(child) => entering = Some((child, true)),
                None => {
                    let frame = stack.pop().expect("the frame being closed is on the stack");
                    finished = Some(self.close(frame, target));
                }
            }
        }
    }

    /// Start rendering one node: its whole text when it has no children to
    /// render, else the frame that will collect them.
    fn enter(
        &mut self,
        reference: TypeRef,
        by_name: bool,
        declaration: &str,
        target: Target,
    ) -> Result<Entered, TsGenError> {
        if by_name {
            if let Some(name) = self.declared.get(&reference) {
                // The named shortcut is only sound if the alias it references
                // was actually emitted. A declared class is refused before
                // rendering, so a reference to its name would be an undefined
                // type in the output — exactly the silent hole the fail-closed
                // rule exists to prevent.
                if matches!(self.node(reference)?, TypeNode::Class { .. }) {
                    return Err(TsGenError::UnsupportedConstruct {
                        declaration: declaration.to_string(),
                        kind: "class",
                    });
                }
                return Ok(Entered::Text(local(name)));
            }
        }
        Ok(match self.node(reference)?.clone() {
            TypeNode::Primitive { primitive } => {
                Entered::Text(self.primitive(primitive, target).to_string())
            }
            TypeNode::Opt { inner } => {
                // `T | null` reads as an absent value should — but it can only
                // carry Candid's optionality when `T` itself can never be
                // `null`. Three inner shapes break that: another `opt` (the
                // classic `None` vs `Some(None)`), `null` itself, and
                // `reserved`, whose `unknown` absorbs `null` entirely. Those
                // box the present value as `{ some: T }`; nothing else does.
                // The test is on the inner *node*, not its spelling: the arena
                // shares one node per declaration, so an alias of an opt — or
                // an opt node that is its own inner (the graph of
                // `type L = opt L`, which the compiler refuses since issue
                // #234 but a loaded or model-built Contract can hold) —
                // boxes just the same, with no reference to chase.
                // The builder is `c.opt(…)` either way: the runtime's
                // `OptDomain` type and its walkers apply the identical rule,
                // and the invariant annotation makes `tsc` prove agreement.
                let boxed = admits_null(self.node(inner)?);
                Entered::Frame(Frame::Opt {
                    inner,
                    boxed,
                    rendered: None,
                })
            }
            TypeNode::Vec { inner } => {
                // `vec nat8` is binary data, and `Uint8Array` is its modern
                // type — always, whatever the element is called: in Candid
                // `blob` *is* `vec nat8`, so `vec Byte` with `type Byte =
                // nat8` is the same type, and one unrelated declaration must
                // not change the value domain of every blob in the
                // interface (issue #191, amending #38).
                if let TypeNode::Primitive {
                    primitive: PrimitiveType::Nat8,
                } = self.node(inner)?
                {
                    return Ok(Entered::Text(
                        match target {
                            Target::Alias => "Uint8Array",
                            Target::Builder => "$.c.blob()",
                        }
                        .to_string(),
                    ));
                }
                Entered::Frame(Frame::Vec {
                    inner,
                    rendered: None,
                })
            }
            TypeNode::Record { fields } => {
                if fields.is_empty() {
                    // `{}` means "anything non-nullish" in TypeScript; an empty
                    // Candid record is a unit value, and this is its honest type.
                    return Ok(Entered::Text(
                        match target {
                            Target::Alias => "Record<string, never>",
                            Target::Builder => "$.c.unit()",
                        }
                        .to_string(),
                    ));
                }
                if is_tuple_shaped(&fields) {
                    let elements = Vec::with_capacity(fields.len());
                    return Ok(Entered::Frame(Frame::Tuple { fields, elements }));
                }
                // Members of a multi-line object sit one level in; a member's
                // own nested object indents from there. `close` restores it.
                let level = self.indent;
                if target == Target::Alias {
                    self.indent = level + 1;
                }
                let members = Vec::with_capacity(fields.len());
                Entered::Frame(Frame::Record {
                    reference,
                    fields,
                    level,
                    members,
                })
            }
            TypeNode::Variant { fields } => {
                if fields.is_empty() {
                    // A variant with no tags is uninhabited. The builder's
                    // `VariantInfer<{}>` distributes over no arms and infers
                    // `never`, matching the alias by construction.
                    return Ok(Entered::Text(
                        match target {
                            Target::Alias => "never",
                            Target::Builder => "$.c.variant({})",
                        }
                        .to_string(),
                    ));
                }
                let level = self.indent;
                let arms = Vec::with_capacity(fields.len());
                Entered::Frame(Frame::Variant {
                    reference,
                    fields,
                    level,
                    arms,
                })
            }
            TypeNode::Func {
                args,
                results,
                mode,
            } => {
                // A func *value* is inert reference data — `{ principal,
                // method }` — while the signature lives in the builder
                // (issue #104).
                match target {
                    Target::Alias => {
                        self.uses_principal = true;
                        Entered::Text(format!(
                            "{{ principal: {}; method: string }}",
                            self.principal
                        ))
                    }
                    Target::Builder => {
                        let arguments = args.len();
                        let mut children = args;
                        children.extend(results);
                        let rendered = Vec::with_capacity(children.len());
                        Entered::Frame(Frame::Func {
                            arguments,
                            children,
                            rendered,
                            mode,
                        })
                    }
                }
            }
            TypeNode::Service { methods } => {
                // A service *value* is the principal of a running service.
                match target {
                    Target::Alias => {
                        self.uses_principal = true;
                        Entered::Text(self.principal.to_string())
                    }
                    Target::Builder => {
                        let members = Vec::with_capacity(methods.len());
                        Entered::Frame(Frame::Service { methods, members })
                    }
                }
            }
            TypeNode::Class { .. } => {
                return Err(TsGenError::UnsupportedConstruct {
                    declaration: declaration.to_string(),
                    kind: "class",
                })
            }
        })
    }

    /// The next child a frame renders, after making the state change the
    /// recursive renderer made before that call; `None` when the frame has
    /// every piece it needs.
    fn next_child(
        &mut self,
        frame: &mut Frame,
        target: Target,
    ) -> Result<Option<TypeRef>, TsGenError> {
        Ok(match frame {
            Frame::Opt {
                inner, rendered, ..
            }
            | Frame::Vec { inner, rendered } => rendered.is_none().then_some(*inner),
            Frame::Tuple { fields, elements } => fields.get(elements.len()).map(|field| field.ty),
            Frame::Record {
                fields, members, ..
            } => fields.get(members.len()).map(|field| field.ty),
            Frame::Variant {
                reference,
                fields,
                level,
                arms,
            } => {
                // A discriminated union: `tag` is the label as a string
                // literal type, `value` carries the payload and is omitted for
                // a `null` payload, because Candid's bare `ok` and `ok : null`
                // are the same variant arm and an ever-present `value: null`
                // would be noise on every tag-only arm. `VariantInfer` omits
                // the same arms for the schema shapes this generator emits —
                // never-domain payloads keep `value`, `opt` payloads keep
                // `value`, and only a null-domain non-opt reference is a bare
                // tag (issue #127) — with one shape the type level cannot
                // classify: a declared alias of `opt empty` renders as a
                // reference whose static type equals a null alias's, so the
                // omission analysis leaves out every declaration and method
                // that would render one (`ambiguous_variant_arm`) instead of
                // emitting text the equality gate would reject.
                while let Some(field) = fields.get(arms.len()) {
                    if target == Target::Builder {
                        return Ok(Some(field.ty));
                    }
                    let payload_is_null = matches!(
                        self.node(field.ty)?,
                        TypeNode::Primitive {
                            primitive: PrimitiveType::Null
                        }
                    );
                    if !payload_is_null {
                        // An expanded arm's members are three levels in:
                        // `| ` on the union line, then the object. `accept`
                        // restores the union's level.
                        self.indent = *level + 3;
                        return Ok(Some(field.ty));
                    }
                    let tag = self.tag_member(*reference, field.id);
                    arms.push(vec![tag]);
                }
                None
            }
            Frame::Func {
                children, rendered, ..
            } => children.get(rendered.len()).copied(),
            Frame::Service { methods, members } => {
                methods.get(members.len()).map(|method| method.function)
            }
        })
    }

    /// Hand a frame the text of the child it last asked for, undoing the
    /// state change made before that child.
    fn accept(&mut self, frame: &mut Frame, text: String, target: Target) {
        match frame {
            Frame::Opt { rendered, .. } | Frame::Vec { rendered, .. } => *rendered = Some(text),
            Frame::Tuple { elements, .. } => elements.push(text),
            Frame::Record {
                reference,
                fields,
                members,
                ..
            } => {
                let field = &fields[members.len()];
                let key = self.field_key(*reference, field.id, target);
                let docs = self
                    .names
                    .provenance
                    .field_docs(&self.origins, *reference, field.id)
                    .to_vec();
                members.push(Member {
                    docs,
                    text: property(&key, &text),
                });
            }
            Frame::Variant {
                reference,
                fields,
                level,
                arms,
            } => {
                let field = &fields[arms.len()];
                match target {
                    Target::Alias => {
                        self.indent = *level;
                        let tag = self.tag_member(*reference, field.id);
                        arms.push(vec![
                            tag,
                            Member {
                                docs: Vec::new(),
                                text: property("value", &text),
                            },
                        ]);
                    }
                    Target::Builder => {
                        let key = self.field_key(*reference, field.id, target);
                        arms.push(vec![Member {
                            docs: Vec::new(),
                            text: property(&key, &text),
                        }]);
                    }
                }
            }
            Frame::Func { rendered, .. } => rendered.push(text),
            Frame::Service { methods, members } => {
                let key = method_key(&methods[members.len()].name);
                members.push(format!("{key}: {text}"));
            }
        }
    }

    /// A finished frame's text.
    fn close(&mut self, frame: Frame, target: Target) -> String {
        match frame {
            Frame::Opt {
                boxed, rendered, ..
            } => {
                let inner = rendered.expect("an opt closes after its inner type");
                match target {
                    Target::Alias if boxed => format!("{{ some: {inner} }} | null"),
                    Target::Alias => format!("{inner} | null"),
                    Target::Builder => format!("$.c.opt({inner})"),
                }
            }
            Frame::Vec { rendered, .. } => {
                let inner = rendered.expect("a vec closes after its element type");
                match target {
                    Target::Alias => format!("Array<{inner}>"),
                    Target::Builder => format!("$.c.vec({inner})"),
                }
            }
            Frame::Tuple { elements, .. } => match target {
                Target::Alias => format!("[{}]", elements.join(", ")),
                Target::Builder => format!("$.c.tuple([{}])", elements.join(", ")),
            },
            Frame::Record { level, members, .. } => {
                self.indent = level;
                match target {
                    Target::Alias => Self::object_type(&members, level),
                    Target::Builder => {
                        let texts: Vec<&str> =
                            members.iter().map(|member| member.text.as_str()).collect();
                        format!("$.c.record({{ {} }})", texts.join(", "))
                    }
                }
            }
            Frame::Variant { level, arms, .. } => match target {
                Target::Alias => {
                    let plain = |members: &[Member]| {
                        members
                            .iter()
                            .all(|member| member.docs.is_empty() && !member.text.contains('\n'))
                    };
                    if arms.iter().all(|members| plain(members)) {
                        let arms: Vec<String> = arms
                            .iter()
                            .map(|members| Self::object_type(members, level))
                            .collect();
                        arms.join(" | ")
                    } else {
                        // Documented arms: one arm per line, the union
                        // opened on its own line as Prettier lays it out.
                        let mut out = String::new();
                        for members in &arms {
                            out.push('\n');
                            out.push_str(&"  ".repeat(level + 1));
                            out.push_str("| ");
                            out.push_str(&Self::object_type(members, level + 2));
                        }
                        out
                    }
                }
                Target::Builder => {
                    let arms: Vec<&str> = arms
                        .iter()
                        .map(|members| members[0].text.as_str())
                        .collect();
                    format!("$.c.variant({{ {} }})", arms.join(", "))
                }
            },
            Frame::Func {
                arguments,
                rendered,
                mode,
                ..
            } => {
                let (args, results) = rendered.split_at(arguments);
                format!(
                    "$.c.func([{}], [{}], \"{}\")",
                    args.join(", "),
                    results.join(", "),
                    mode_text(mode),
                )
            }
            Frame::Service { members, .. } => {
                format!("$.c.service({{ {} }})", members.join(", "))
            }
        }
    }

    /// A variant arm's `tag` member in an alias, with the arm's docs.
    fn tag_member(&self, container: TypeRef, id: u32) -> Member {
        Member {
            docs: self
                .names
                .provenance
                .field_docs(&self.origins, container, id)
                .to_vec(),
            text: format!("tag: {}", self.tag_literal(container, id)),
        }
    }

    fn primitive(&mut self, primitive: PrimitiveType, target: Target) -> &'static str {
        if target == Target::Builder {
            if primitive == PrimitiveType::Principal {
                self.uses_principal = true;
            }
            return match primitive {
                PrimitiveType::Null => "$.c.null",
                PrimitiveType::Bool => "$.c.bool",
                PrimitiveType::Nat => "$.c.nat",
                PrimitiveType::Int => "$.c.int",
                PrimitiveType::Nat8 => "$.c.nat8",
                PrimitiveType::Nat16 => "$.c.nat16",
                PrimitiveType::Nat32 => "$.c.nat32",
                PrimitiveType::Nat64 => "$.c.nat64",
                PrimitiveType::Int8 => "$.c.int8",
                PrimitiveType::Int16 => "$.c.int16",
                PrimitiveType::Int32 => "$.c.int32",
                PrimitiveType::Int64 => "$.c.int64",
                PrimitiveType::Float32 => "$.c.float32",
                PrimitiveType::Float64 => "$.c.float64",
                PrimitiveType::Text => "$.c.text",
                PrimitiveType::Reserved => "$.c.reserved",
                PrimitiveType::Empty => "$.c.empty",
                PrimitiveType::Principal => "$.c.principal",
            };
        }
        match primitive {
            PrimitiveType::Null => "null",
            PrimitiveType::Bool => "boolean",
            // Unbounded (or 64-bit) on the wire; `number` silently corrupts
            // integers past 2^53.
            PrimitiveType::Nat
            | PrimitiveType::Int
            | PrimitiveType::Nat64
            | PrimitiveType::Int64 => "bigint",
            PrimitiveType::Nat8
            | PrimitiveType::Nat16
            | PrimitiveType::Nat32
            | PrimitiveType::Int8
            | PrimitiveType::Int16
            | PrimitiveType::Int32
            | PrimitiveType::Float32
            | PrimitiveType::Float64 => "number",
            PrimitiveType::Text => "string",
            // Accepts anything, asserts nothing — exactly `unknown`.
            PrimitiveType::Reserved => "unknown",
            // Uninhabited on both sides.
            PrimitiveType::Empty => "never",
            PrimitiveType::Principal => {
                self.uses_principal = true;
                self.principal
            }
        }
    }

    /// A variant tag as a TypeScript string literal type: the supplied label
    /// text when one exists, else the `_id_` convention. Always quoted — the
    /// literal is a type, not a property name.
    fn tag_literal(&self, container: TypeRef, id: u32) -> String {
        match self.names.get(container, id) {
            Some(name) => quote_string(name),
            None => quote_string(&format!("_{id}_")),
        }
    }

    /// A property key: the supplied name when one exists (quoted when it is
    /// not shaped like an identifier), else the ecosystem's `_id_` convention.
    ///
    /// The exact name `__proto__` renders as a computed key in builder object
    /// literals: a non-computed `__proto__` property definition — bare or
    /// quoted — sets the object's prototype instead of defining the field,
    /// and TypeScript does not model that runtime special case, so the tsc
    /// equality gate cannot catch the divergence. Type literals have no such
    /// case, so aliases keep the plain form.
    fn field_key(&self, container: TypeRef, id: u32, target: Target) -> String {
        match self.names.get(container, id) {
            Some(name) if name == "__proto__" && target == Target::Builder => {
                format!("[{}]", quote_string(name))
            }
            Some(name) if is_ts_property_identifier(name) => name.to_string(),
            Some(name) => quote_string(name),
            None => format!("_{id}_"),
        }
    }
}

/// `lhs = rhs`, or `lhs =` and the right-hand side on the lines below when it
/// opens with a line break (a multi-line union), so no line ends in a space.
fn assign(lhs: &str, rhs: &str) -> String {
    if rhs.starts_with('\n') {
        format!("{lhs} ={rhs}")
    } else {
        format!("{lhs} = {rhs}")
    }
}

/// `key: value`, with the same line-break rule as [`assign`].
fn property(key: &str, value: &str) -> String {
    if value.starts_with('\n') {
        format!("{key}:{value}")
    } else {
        format!("{key}: {value}")
    }
}

/// Whether a node's TypeScript domain admits `null` as a value — the rule
/// that boxes an `opt` over it. Mirrors the schema runtime exactly (its
/// `isBoxedOpt`, its `OptDomain` type, and the walkers' node test): another
/// `opt` (its absence is `null`), the `null` primitive, and `reserved`
/// (`unknown`). A never-domain node (`empty`, an empty variant) does not, so
/// `opt empty` stays the plain `null`.
fn admits_null(node: &TypeNode) -> bool {
    matches!(
        node,
        TypeNode::Opt { .. }
            | TypeNode::Primitive {
                primitive: PrimitiveType::Null | PrimitiveType::Reserved,
            }
    )
}

/// Candid tuple lowering assigns the sequential numeric labels `0..n`, so a
/// record whose ids are exactly that sequence renders as a TypeScript tuple.
fn is_tuple_shaped(fields: &[Field]) -> bool {
    fields
        .iter()
        .enumerate()
        .all(|(index, field)| field.id as usize == index)
}

/// Whether a name is the canonical `_N_` id rendering: underscore, decimal
/// digits, underscore, no leading zero (unless the single digit 0), value
/// below 2^32. Mirrors `ts/labels.ts`'s `numericKeyId` exactly — the two
/// sides must reserve the same set, or one of them mis-derives a wire id.
fn is_reserved_numeric_name(name: &str) -> bool {
    let Some(digits) = name
        .strip_prefix('_')
        .and_then(|rest| rest.strip_suffix('_'))
    else {
        return false;
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return false;
    }
    digits.parse::<u64>().is_ok_and(|value| value < 1 << 32)
}
/// The Candid method mode as the builder literal the schema core takes.
fn mode_text(mode: candid_core::MethodMode) -> &'static str {
    match mode {
        candid_core::MethodMode::Update => "update",
        candid_core::MethodMode::Query => "query",
        candid_core::MethodMode::CompositeQuery => "composite_query",
        candid_core::MethodMode::Oneway => "oneway",
    }
}

/// A service-method property key: bare when identifier-shaped, quoted
/// otherwise, computed for exactly `__proto__` (the #114 rule — a method may
/// legally carry that name, and a non-computed literal would set the
/// prototype). Methods carry their name explicitly on the wire, so the
/// `_N_` id-rendering reservation does not apply here.
fn method_key(name: &str) -> String {
    if name == "__proto__" {
        format!("[{}]", quote_string(name))
    } else if is_ts_property_identifier(name) {
        name.to_string()
    } else {
        quote_string(name)
    }
}

/// The module's own export names: the actor surface exports the service
/// schema as `actor` and its call interface as `Actor`, so a declaration by
/// either name would be a duplicate export. Every other binding the module
/// declares is a `$`-prefixed local (issue #188), which no declaration name
/// can collide with; these two are reserved unconditionally, actor or not,
/// by the #116 locality rule, and a declaration by either name is omitted
/// (`reserved_export_name`, issue #189).
const RESERVED_EXPORT_NAMES: &[&str] = &["actor", "Actor"];

/// A declaration or method name as the module header writes it: bare when
/// identifier-shaped, else JSON-quoted with the two further line terminators
/// ECMAScript honours inside a `//` comment (U+2028, U+2029) escaped too, so
/// no name can end the comment line it is listed on.
fn comment_name(name: &str) -> String {
    if is_ts_property_identifier(name) {
        return name.to_string();
    }
    quote_string(name)
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Property names may be any identifier-shaped text, keywords included —
/// `{ delete: T }` is legal TypeScript — so only the character shape matters.
/// The same shape admits a declaration name: its `$`-prefixed local is then
/// a valid binding that no reserved word can equal, and its export name may
/// be any identifier name.
fn is_ts_property_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// JSON-compatible string quoting, which is valid TypeScript.
fn quote_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The scaffold's original assertion, kept: the base surface links and is
    // usable without any feature.
    #[test]
    fn base_surface_links_without_features() {
        let limits = candid_core::Limits::default();
        assert!(limits.max_input_bytes() > 0);
    }

    #[test]
    fn property_identifiers_accept_keywords_and_reject_shapes() {
        assert!(is_ts_property_identifier("delete"));
        assert!(is_ts_property_identifier("_1234_"));
        assert!(is_ts_property_identifier("$ok"));
        assert!(!is_ts_property_identifier("1abc"));
        assert!(!is_ts_property_identifier("has space"));
        assert!(!is_ts_property_identifier("naïve"));
        assert!(!is_ts_property_identifier(""));
    }

    #[test]
    fn reserved_numeric_name_shapes_mirror_the_ts_predicate() {
        // Must match ts/labels.ts numericKeyId exactly: canonical decimal
        // below 2^32, no leading zero unless the single digit 0.
        for reserved in ["_0_", "_5_", "_123_", "_4294967295_"] {
            assert!(is_reserved_numeric_name(reserved), "{reserved}");
        }
        for ordinary in [
            "_007_",
            "_4294967296_",
            "__",
            "_x1_",
            "_1",
            "1_",
            "x",
            "_1_2_",
        ] {
            assert!(!is_reserved_numeric_name(ordinary), "{ordinary}");
        }
    }

    #[test]
    fn quoting_escapes_json_style() {
        assert_eq!(quote_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(quote_string("naïve"), "\"naïve\"");
    }
}
