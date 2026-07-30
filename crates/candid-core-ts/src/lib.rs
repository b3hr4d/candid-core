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
//! # Scope of the first slice
//!
//! All eighteen primitives, `opt`, `vec`, `record` (tuple-shaped records
//! become TypeScript tuples), `variant`, and named declaration references,
//! recursion included. `func`, `service`, and `class` are deferred: a
//! top-level declaration of a deferred kind is skipped with a note in the
//! generated header, while a deferred construct *nested* inside a supported
//! type fails closed with [`TsGenError::UnsupportedConstruct`] rather than
//! silently emitting `unknown`.
//!
//! # A clean domain model, not the wire shape
//!
//! By owner decision on issue #38, the output is the clean modern reading of
//! a Contract rather than the shapes the agent-js runtime produces: `opt T`
//! is `T | null`, variants are discriminated `{ tag, value }` unions, and
//! anonymous `vec nat8` is `Uint8Array`. Two consequences are deliberate.
//! An `opt` whose inner type can itself be `null` — another `opt`, `null`,
//! `reserved` — fails closed with [`TsGenError::UnrepresentableOption`],
//! because `T | null` cannot carry `None` versus `Some(None)`. And consuming
//! these types against a live agent needs a boundary conversion, which is
//! future work recorded on the issue — the types describe the domain, not the
//! transport.
//!
//! # Determinism
//!
//! Identical Contracts produce byte-identical TypeScript: emission follows the
//! Contract's canonical declaration and field order, and nothing in the output
//! depends on time, environment, or map iteration order.
//!
//! # What is deliberately not claimed
//!
//! `@icp-sdk/bindgen` is a differential *oracle* for type-level agreement in a
//! later slice, never the specification; byte-identical output is a non-goal.
//! No performance claims are made. The Contract format is not stable v1, and
//! this crate tracks it as a pre-1.0 consumer pinned to an exact version.

use std::collections::BTreeMap;
use std::fmt;

use candid_core::{Contract, Field, PrimitiveType, TypeNode, TypeRef};

/// Caller-supplied field label text, keyed by `(container node, label id)`.
///
/// The semantic Contract stores only label IDs; see the crate docs for why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsNames {
    labels: BTreeMap<(TypeRef, u32), String>,
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
    /// and its provenance entry must not override the `_id_` rendering.
    #[cfg(feature = "compiler")]
    pub fn from_source_info(source_info: &candid_core::SourceInfo) -> Self {
        let mut names = Self::new();
        for provenance in source_info.field_labels() {
            if let candid_core::SourceLabel::Named { name } = &provenance.label {
                names.insert(provenance.container, provenance.id, name.clone());
            }
        }
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
    /// the `principal` primitive. The import is emitted only when used, and as
    /// `import type`, so it never implies a runtime dependency.
    pub principal_import: String,
}

impl Default for TsOptions {
    fn default() -> Self {
        Self {
            principal_import: "@icp-sdk/core/principal".to_string(),
        }
    }
}

/// A failure to generate. Fail-closed by design: no error path emits
/// placeholder TypeScript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TsGenError {
    /// A deferred construct (`func`, `service`, `class`) appears nested inside
    /// a supported type. Top-level declarations of deferred kinds are skipped
    /// with a header note instead.
    UnsupportedConstruct {
        declaration: String,
        kind: &'static str,
    },
    /// A declaration name cannot be a TypeScript type identifier.
    InvalidDeclarationName { name: String },
    /// A field or arm name shaped like the `_N_` id rendering (canonical
    /// decimal below 2^32). Erased to a schema key, such a name is
    /// indistinguishable from the rendering of numeric label id N, so the
    /// codec would derive the wrong wire id from it — the same reservation
    /// `schemaFromContract` enforces on its name table (issues #103, #115).
    ReservedFieldName { declaration: String, name: String },
    /// A declaration named after a binding the generated module itself
    /// references: an imported binding (`c`, `Schema`, `Principal`) or an
    /// ambient type its lowerings emit (`Array`, `Record`, `Uint8Array`).
    /// Emitting it would produce a module that cannot load or compile — a
    /// duplicate or shadowed module-scope binding — so generation refuses
    /// instead of emitting known-broken text. Unconditional by owner
    /// decision on issue #116: every name is refused even when the contract
    /// happens not to exercise the lowering that references it, so adding
    /// one field to a contract cannot start breaking a previously-working
    /// declaration name.
    ReservedDeclarationName { name: String },
    /// An `opt` whose inner type can itself be `null` in TypeScript — another
    /// `opt`, `null`, or `reserved` — cannot be carried by `T | null` without
    /// collapsing `None` into `Some(None)`. Fail closed rather than corrupt.
    UnrepresentableOption {
        declaration: String,
        inner: &'static str,
    },
    /// A type reference points outside the Contract arena. A validated
    /// Contract cannot contain one; this guards the unvalidated path.
    DanglingTypeRef { reference: TypeRef },
}

impl fmt::Display for TsGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedConstruct { declaration, kind } => write!(
                f,
                "declaration `{declaration}` nests a `{kind}` type, which the \
                 first generator slice defers (issue #38)"
            ),
            Self::InvalidDeclarationName { name } => write!(
                f,
                "declaration name `{name}` is not a valid TypeScript type identifier"
            ),
            Self::ReservedFieldName { declaration, name } => write!(
                f,
                "declaration `{declaration}` names a field `{name}`, which is \
                 shaped like the `_N_` numeric-id rendering: erased to a schema \
                 key it would make the codec derive wire id N instead of the \
                 name's hash, so generation refuses (issues #103, #115)"
            ),
            Self::ReservedDeclarationName { name } => write!(
                f,
                "declaration name `{name}` collides with a binding the \
                 generated module itself references (its imports and the \
                 ambient types its lowerings use), so the module could \
                 never compile; generation refuses (issue #116)"
            ),
            Self::UnrepresentableOption { declaration, inner } => write!(
                f,
                "declaration `{declaration}` wraps `{inner}` in `opt`: `T | null` \
                 cannot distinguish `None` from `Some(None)` there, so generation \
                 refuses rather than collapsing the two"
            ),
            Self::DanglingTypeRef { reference } => {
                write!(
                    f,
                    "type reference {reference} is outside the Contract arena"
                )
            }
        }
    }
}

impl std::error::Error for TsGenError {}

/// Generate a TypeScript module from a Contract.
///
/// See the crate docs for scope, determinism, and the role of [`TsNames`].
pub fn generate_module(
    contract: &Contract,
    names: &TsNames,
    options: &TsOptions,
) -> Result<String, TsGenError> {
    Generator {
        contract,
        names,
        declared: first_names(contract),
        uses_principal: false,
    }
    .module(options)
}

/// The first declaration name for each node, in declaration order. Later
/// aliases of the same node render as references to the first name.
fn first_names(contract: &Contract) -> BTreeMap<TypeRef, String> {
    let mut map = BTreeMap::new();
    for declaration in contract.declarations() {
        map.entry(declaration.ty)
            .or_insert_with(|| declaration.name.clone());
    }
    map
}

/// One traversal, two syntaxes. `Alias` renders the static type expression;
/// `Builder` renders the runtime schema expression the alias annotates. Every
/// guard — deferred constructs, collapsing options, dangling refs — runs
/// before this dispatch, so the two outputs can never disagree about what is
/// representable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Alias,
    Builder,
}

struct Generator<'a> {
    contract: &'a Contract,
    names: &'a TsNames,
    declared: BTreeMap<TypeRef, String>,
    uses_principal: bool,
}

impl Generator<'_> {
    fn module(mut self, options: &TsOptions) -> Result<String, TsGenError> {
        let mut aliases = Vec::new();
        let mut deferred = Vec::new();

        for declaration in self.contract.declarations() {
            if !is_ts_type_identifier(&declaration.name) {
                return Err(TsGenError::InvalidDeclarationName {
                    name: declaration.name.clone(),
                });
            }
            if RESERVED_MODULE_BINDINGS.contains(&declaration.name.as_str()) {
                return Err(TsGenError::ReservedDeclarationName {
                    name: declaration.name.clone(),
                });
            }
            match self.node(declaration.ty)? {
                TypeNode::Func { .. } => deferred.push((declaration.name.clone(), "func")),
                TypeNode::Service { .. } => deferred.push((declaration.name.clone(), "service")),
                TypeNode::Class { .. } => deferred.push((declaration.name.clone(), "class")),
                _ => {
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
                    let alias =
                        self.declaration_body(declaration.ty, &declaration.name, Target::Alias)?;
                    let builder =
                        self.declaration_body(declaration.ty, &declaration.name, Target::Builder)?;
                    aliases.push(format!(
                        "export type {name} = {alias};\nexport const {name}: Schema<{name}> = c.rec(() => {builder});\n",
                        name = declaration.name,
                    ));
                }
            }
        }

        let mut out = String::from(
            "// Generated by candid-core-ts from a candid-core Contract. Do not edit.\n",
        );
        for (name, kind) in &deferred {
            out.push_str(&format!(
                "// Deferred: `{name}` is a `{kind}` declaration; \
                 func/service/class arrive in a later slice (issue #38).\n"
            ));
        }
        if self.contract.actor().is_some() {
            out.push_str(
                "// Deferred: the actor interface; func/service/class arrive in a \
                 later slice (issue #38).\n",
            );
        }
        if !aliases.is_empty() {
            out.push_str("import { c, type Schema } from \"@candid-core/schema\";\n");
        }
        if self.uses_principal {
            out.push_str(&format!(
                "import type {{ Principal }} from {};\n",
                quote_string(&options.principal_import)
            ));
        }
        if !aliases.is_empty() {
            out.push('\n');
            out.push_str(&aliases.join("\n"));
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
            Some(first) if first != own_name => Ok(first.clone()),
            _ => self.render_structure(ty, own_name, target),
        }
    }

    fn node(&self, reference: TypeRef) -> Result<&TypeNode, TsGenError> {
        self.contract
            .types()
            .get(reference as usize)
            .ok_or(TsGenError::DanglingTypeRef { reference })
    }

    /// Render a type expression. Declared nodes render as their first name —
    /// which is what terminates recursion, since every Candid cycle passes
    /// through a declaration.
    fn render(
        &mut self,
        reference: TypeRef,
        declaration: &str,
        target: Target,
    ) -> Result<String, TsGenError> {
        if let Some(name) = self.declared.get(&reference) {
            // The named shortcut is only sound if the alias it references was
            // actually emitted. A declared func/service/class was *skipped*
            // with a header note, so a reference to its name would be an
            // undefined type in the output — exactly the silent hole the
            // fail-closed rule exists to prevent.
            let kind = match self.node(reference)? {
                TypeNode::Func { .. } => Some("func"),
                TypeNode::Service { .. } => Some("service"),
                TypeNode::Class { .. } => Some("class"),
                _ => None,
            };
            if let Some(kind) = kind {
                return Err(TsGenError::UnsupportedConstruct {
                    declaration: declaration.to_string(),
                    kind,
                });
            }
            return Ok(name.clone());
        }
        self.render_structure(reference, declaration, target)
    }

    fn render_structure(
        &mut self,
        reference: TypeRef,
        declaration: &str,
        target: Target,
    ) -> Result<String, TsGenError> {
        match self.node(reference)?.clone() {
            TypeNode::Primitive { primitive } => Ok(self.primitive(primitive, target).to_string()),
            TypeNode::Opt { inner } => {
                // `T | null` reads as an absent value should — but it can only
                // carry Candid's optionality when `T` itself can never be
                // `null`. Three inner shapes break that: another `opt` (the
                // classic `None` vs `Some(None)`), `null` itself, and
                // `reserved`, whose `unknown` absorbs `null` entirely. The
                // check is on the inner *node*, not its spelling, so an alias
                // of an opt collapses just as surely and is refused just the
                // same.
                let collapsing = match self.node(inner)? {
                    TypeNode::Opt { .. } => Some("opt"),
                    TypeNode::Primitive {
                        primitive: PrimitiveType::Null,
                    } => Some("null"),
                    TypeNode::Primitive {
                        primitive: PrimitiveType::Reserved,
                    } => Some("reserved"),
                    _ => None,
                };
                if let Some(inner_kind) = collapsing {
                    return Err(TsGenError::UnrepresentableOption {
                        declaration: declaration.to_string(),
                        inner: inner_kind,
                    });
                }
                let inner = self.render(inner, declaration, target)?;
                Ok(match target {
                    Target::Alias => format!("{inner} | null"),
                    Target::Builder => format!("c.opt({inner})"),
                })
            }
            TypeNode::Vec { inner } => {
                // `vec nat8` is binary data, and `Uint8Array` is its modern
                // type. Only the *anonymous* nat8 node gets it: an element
                // type the Contract declares by name (`type Byte = nat8`) is a
                // deliberate abstraction and keeps its name.
                if !self.declared.contains_key(&inner) {
                    if let TypeNode::Primitive {
                        primitive: PrimitiveType::Nat8,
                    } = self.node(inner)?
                    {
                        return Ok(match target {
                            Target::Alias => "Uint8Array".to_string(),
                            Target::Builder => "c.blob()".to_string(),
                        });
                    }
                }
                let inner = self.render(inner, declaration, target)?;
                Ok(match target {
                    Target::Alias => format!("Array<{inner}>"),
                    Target::Builder => format!("c.vec({inner})"),
                })
            }
            TypeNode::Record { fields } => {
                if fields.is_empty() {
                    // `{}` means "anything non-nullish" in TypeScript; an empty
                    // Candid record is a unit value, and this is its honest type.
                    return Ok(match target {
                        Target::Alias => "Record<string, never>".to_string(),
                        Target::Builder => "c.unit()".to_string(),
                    });
                }
                if is_tuple_shaped(&fields) {
                    let mut elements = Vec::with_capacity(fields.len());
                    for field in &fields {
                        elements.push(self.render(field.ty, declaration, target)?);
                    }
                    return Ok(match target {
                        Target::Alias => format!("[{}]", elements.join(", ")),
                        Target::Builder => format!("c.tuple([{}])", elements.join(", ")),
                    });
                }
                let mut members = Vec::with_capacity(fields.len());
                for field in &fields {
                    self.check_reserved_name(reference, field.id, declaration)?;
                    let key = self.field_key(reference, field.id, target);
                    let value = self.render(field.ty, declaration, target)?;
                    members.push(format!("{key}: {value}"));
                }
                Ok(match target {
                    Target::Alias => format!("{{ {} }}", members.join("; ")),
                    Target::Builder => format!("c.record({{ {} }})", members.join(", ")),
                })
            }
            TypeNode::Variant { fields } => {
                if fields.is_empty() {
                    // A variant with no tags is uninhabited. The builder's
                    // `VariantInfer<{}>` distributes over no arms and infers
                    // `never`, matching the alias by construction.
                    return Ok(match target {
                        Target::Alias => "never".to_string(),
                        Target::Builder => "c.variant({})".to_string(),
                    });
                }
                // A discriminated union: `tag` is the label as a string
                // literal type, `value` carries the payload and is omitted for
                // a `null` payload, because Candid's bare `ok` and `ok : null`
                // are the same variant arm and an ever-present `value: null`
                // would be noise on every tag-only arm. The builder emits the
                // arm's payload schema either way — `VariantInfer` performs the
                // same null-payload omission at the type level, which is what
                // keeps the two representations provably aligned.
                let mut arms = Vec::with_capacity(fields.len());
                for field in &fields {
                    self.check_reserved_name(reference, field.id, declaration)?;
                    match target {
                        Target::Alias => {
                            let tag = self.tag_literal(reference, field.id);
                            let payload_is_null = matches!(
                                self.node(field.ty)?,
                                TypeNode::Primitive {
                                    primitive: PrimitiveType::Null
                                }
                            );
                            if payload_is_null {
                                arms.push(format!("{{ tag: {tag} }}"));
                            } else {
                                let value = self.render(field.ty, declaration, target)?;
                                arms.push(format!("{{ tag: {tag}; value: {value} }}"));
                            }
                        }
                        Target::Builder => {
                            let key = self.field_key(reference, field.id, target);
                            let value = self.render(field.ty, declaration, target)?;
                            arms.push(format!("{key}: {value}"));
                        }
                    }
                }
                Ok(match target {
                    Target::Alias => arms.join(" | "),
                    Target::Builder => format!("c.variant({{ {} }})", arms.join(", ")),
                })
            }
            TypeNode::Func { .. } => Err(TsGenError::UnsupportedConstruct {
                declaration: declaration.to_string(),
                kind: "func",
            }),
            TypeNode::Service { .. } => Err(TsGenError::UnsupportedConstruct {
                declaration: declaration.to_string(),
                kind: "service",
            }),
            TypeNode::Class { .. } => Err(TsGenError::UnsupportedConstruct {
                declaration: declaration.to_string(),
                kind: "class",
            }),
        }
    }

    fn primitive(&mut self, primitive: PrimitiveType, target: Target) -> &'static str {
        if target == Target::Builder {
            if primitive == PrimitiveType::Principal {
                self.uses_principal = true;
            }
            return match primitive {
                PrimitiveType::Null => "c.null",
                PrimitiveType::Bool => "c.bool",
                PrimitiveType::Nat => "c.nat",
                PrimitiveType::Int => "c.int",
                PrimitiveType::Nat8 => "c.nat8",
                PrimitiveType::Nat16 => "c.nat16",
                PrimitiveType::Nat32 => "c.nat32",
                PrimitiveType::Nat64 => "c.nat64",
                PrimitiveType::Int8 => "c.int8",
                PrimitiveType::Int16 => "c.int16",
                PrimitiveType::Int32 => "c.int32",
                PrimitiveType::Int64 => "c.int64",
                PrimitiveType::Float32 => "c.float32",
                PrimitiveType::Float64 => "c.float64",
                PrimitiveType::Text => "c.text",
                PrimitiveType::Reserved => "c.reserved",
                PrimitiveType::Empty => "c.empty",
                PrimitiveType::Principal => "c.principal",
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
                "Principal"
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

    /// Refuse a supplied name shaped like the `_N_` id rendering wherever a
    /// key or tag would render it. Tuple-shaped records never render keys and
    /// are exempt by construction — and unreachable besides: a `_N_`-shaped
    /// name hashes far outside the `0..n-1` id range tuples require.
    fn check_reserved_name(
        &self,
        container: TypeRef,
        id: u32,
        declaration: &str,
    ) -> Result<(), TsGenError> {
        match self.names.get(container, id) {
            Some(name) if is_reserved_numeric_name(name) => Err(TsGenError::ReservedFieldName {
                declaration: declaration.to_string(),
                name: name.to_string(),
            }),
            _ => Ok(()),
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
/// Names the generated module itself references: the imported bindings
/// (`import { c, type Schema }` always, `import type { Principal }` when the
/// principal primitive is used) and the ambient types its lowerings emit
/// (`Array<T>` for vecs, `Record<string, never>` for the empty record,
/// `Uint8Array` for anonymous `vec nat8`). A declaration by any of these
/// names shadows the referenced binding for the whole module.
const RESERVED_MODULE_BINDINGS: &[&str] =
    &["c", "Schema", "Principal", "Array", "Record", "Uint8Array"];

/// Property names may be any identifier-shaped text, keywords included —
/// `{ delete: T }` is legal TypeScript — so only the character shape matters.
fn is_ts_property_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Type alias names additionally must not be a reserved or predefined-type
/// word: `export type delete = …` and `export type string = …` do not parse.
fn is_ts_type_identifier(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "any",
        "as",
        "await",
        "bigint",
        "boolean",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "declare",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "function",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "never",
        "new",
        "null",
        "number",
        "object",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "static",
        "string",
        "super",
        "switch",
        "symbol",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "unknown",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ];
    is_ts_property_identifier(name) && !RESERVED.contains(&name)
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
    fn type_identifiers_reject_reserved_words() {
        assert!(is_ts_type_identifier("Item"));
        assert!(!is_ts_type_identifier("delete"));
        assert!(!is_ts_type_identifier("string"));
        assert!(!is_ts_type_identifier("undefined"));
        // Reserved at ES-module top level even though it is not a keyword
        // everywhere: `export type await = ...` does not parse.
        assert!(!is_ts_type_identifier("await"));
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
