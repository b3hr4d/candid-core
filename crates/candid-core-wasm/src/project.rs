//! The interface projection behind `projectDid`: a `.did` holding the named
//! methods of a service and every declaration they reach, and nothing else.
//!
//! The projection works on the Candid syntax, not on the canonical Contract,
//! so the output keeps what a generator reads from the source and a Contract
//! does not hold: declaration names as written, doc comments, and argument
//! names. The caller has already compiled the input, so every source here is
//! known to parse and type-check; the bundle is merged exactly as the
//! compiler merges it (`candid_parser`'s merged program: every declaration,
//! the entry's first, then each other source's; the entry's actor; and the
//! service of each source a service import reaches). The caller then
//! compiles the output again, which is what proves it a valid `.did` with
//! exactly the requested methods.
//!
//! The text comes from a small printer here rather than from
//! `candid_parser`'s: that one quotes every name it treats as a keyword,
//! `reserved` and `nat` included, which the lexer reads as plain identifiers,
//! and upstream's parser drops the doc comment of a quoted name — so a
//! reprinted method named `reserved` would lose its docs. This printer
//! quotes only what the lexer requires and writes each doc line so that it
//! parses back to the same doc text. Its layout is fixed: one member per
//! line inside every non-empty record, variant and service, two spaces per
//! level, and signatures on one line. A tuple sits on one line too, unless
//! one of its elements is documented: then it takes one element per line,
//! like a record, so each element's docs sit above it. (A signature's
//! arguments carry no docs in Candid, so one line loses nothing there.)
//!
//! What the output drops: every method not requested, every declaration no
//! requested method reaches, a service class's init arguments (the output is
//! `service : { … }`, the view a client calls), and the import structure — a
//! bundle projects to one self-contained file.
//!
//! Determinism: methods come out in name order (code point), whatever order
//! the sources declare them in, a service import's among the entry's, and
//! whatever order the caller names them in; record fields and variant arms
//! in label-id order, as the parser gives them; and declarations in the
//! merged program's order (the entry's in source order, then each imported
//! source's, by source ID). So the same input and the same set of names give
//! the same bytes.
//!
//! # Cost
//!
//! Every lookup by name goes through an [`Index`]: names sorted once, then
//! found by binary search, `O(log n)` comparisons of names among `n`. Each
//! distinct requested name is one lookup among the service's methods, each
//! step from the actor through declaration names to its service is one
//! among the declarations, and so is each declaration name reachability
//! meets for the first time. `candid_parser`'s merged program would scan
//! every declaration at each of those steps (`IDLMergedProg::lookup`, and
//! `resolve_actor`'s walk to the service), which is why the bundle is merged
//! here. Reachability visits each type of the requested methods, and of the
//! declarations they reach, once.
//!
//! The text is bounded rather than linear in the source: its layout puts
//! two spaces a level before every line, a doc line's included, up to the
//! compiler's nesting bound of 256 levels, so a deep, documented type can
//! print to many times the length of its source. It is written up to
//! [`ProjectionOptions::text_byte_limit`], by default the compiler's bound
//! on one source, which it must stay within to be compiled again, and no
//! further: the first piece that would take it past the bound stops the
//! printing ([`Refusal::TooLong`]). So printing costs time linear in at
//! most the bound plus one piece: a quoted name, a doc line, or one line's
//! indentation.
//!
//! The rest is paid once per projection: parsing each source again (the
//! compiler parsed it first), merging, sorting the declarations' and the
//! methods' names for their indexes and the actor's methods for the output,
//! in time linear in the sources plus `O(n log n)` comparisons of names for
//! `n` declarations or methods. Compiling the input before, and the text
//! after, is the compiler's cost, within its own limits.

use std::borrow::Borrow;
use std::collections::BTreeSet;

use candid_core::{Limits, SourceImportKind, SourceInfo};
use candid_parser::candid::types::{FuncMode, Label};
use candid_parser::syntax::{
    Binding, FuncType, IDLActorType, IDLArgType, IDLType, PrimType, TypeField,
};
use candid_parser::IDLProg;

/// The most text the unknown-method diagnostics of one projection report, in
/// bytes: their messages, as the JSON response writes them (escapes
/// included, quotes not). 4 MiB, the compiler's default `max_input_bytes`
/// and the check's bound on its reported text. The unknown name whose
/// message would take the text past it, and every unknown name after it,
/// are not reported; one `resource_limit_exceeded` diagnostic
/// (`projection_diagnostic_bytes`) says how many were left. The text is
/// measured only up to the first message that passes the bound.
pub const MAX_PROJECTION_DIAGNOSTIC_BYTES: usize = 4 * 1024 * 1024;

/// How a projection runs. [`ProjectionOptions::default`] is what
/// `projectDid` uses; this crate's tests lower the bounds.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct ProjectionOptions {
    /// The bound on the projection's text, in bytes: by default the
    /// compiler's bound on one source (`Limits::max_source_bytes`, 1 MiB),
    /// since the text is compiled again.
    pub text_byte_limit: usize,
    /// The bound on the text of the unknown-method diagnostics.
    pub diagnostic_byte_limit: usize,
}

impl Default for ProjectionOptions {
    fn default() -> Self {
        Self {
            text_byte_limit: Limits::default().max_source_bytes(),
            diagnostic_byte_limit: MAX_PROJECTION_DIAGNOSTIC_BYTES,
        }
    }
}

/// What a projection's lookups by name cost: this crate's tests pin it, so
/// that a lookup that scans, or a name looked up by anything but an
/// [`Index`], fails them.
#[doc(hidden)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionWork {
    /// Names looked up: each distinct requested name among the service's
    /// methods, each step from the actor to its service, and each
    /// declaration name reachability meets for the first time.
    pub lookups: usize,
    /// The names those lookups compared.
    pub compared: usize,
}

/// Why a projection was not built from sources the compiler accepted.
pub enum Refusal {
    /// The text would take more than `limit` bytes. Printing stopped at the
    /// first piece that would have passed the bound; `observed` counts the
    /// text written so far and that piece.
    TooLong { limit: usize, observed: usize },
    /// Never expected; the caller reports it as an internal failure.
    Internal(String),
}

fn internal(message: impl Into<String>) -> Refusal {
    Refusal::Internal(message.into())
}

/// Names sorted once, each with a value, and found by binary search: a
/// lookup compares `O(log n)` names for `n` entries, where a scan compares
/// up to `n`. The first entry given for a name is the one kept, as
/// `IDLMergedProg::lookup` finds the first declaration of a name (the
/// compiler refuses a name declared twice).
pub struct Index<'a, T> {
    entries: Vec<(&'a str, T)>,
}

impl<'a, T> Index<'a, T> {
    pub fn new(entries: impl IntoIterator<Item = (&'a str, T)>) -> Self {
        let mut entries: Vec<(&'a str, T)> = entries.into_iter().collect();
        // A stable sort keeps the first entry of a name ahead of any later
        // one, and `dedup_by` keeps the first of each run.
        entries.sort_by(|left, right| left.0.cmp(right.0));
        entries.dedup_by(|later, earlier| later.0 == earlier.0);
        Self { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The value of `name`, counting the lookup and every name it compares.
    pub fn get(&self, name: &str, work: &mut ProjectionWork) -> Option<&T> {
        work.lookups += 1;
        self.entries
            .binary_search_by(|(probe, _)| {
                work.compared += 1;
                (*probe).cmp(name)
            })
            .ok()
            .map(|position| &self.entries[position].1)
    }

    /// The names, sorted (code point).
    pub fn names(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.entries.iter().map(|(name, _)| *name)
    }
}

/// The text of a source after one leading UTF-8 byte order mark, which the
/// compiler skips the same way.
fn candid_text(source: &str) -> &str {
    source.strip_prefix('\u{FEFF}').unwrap_or(source)
}

fn parse(name: &str, source: &str) -> Result<IDLProg, Refusal> {
    candid_text(source)
        .parse::<IDLProg>()
        .map_err(|error| internal(format!("{name} no longer parses: {error}")))
}

/// The bundle merged as the compiler merges it, with `IDLMergedProg::merge`:
/// every declaration, the entry's first, then each other source's in the
/// order the source info lists the sources; the entry's actor; and the
/// actor of each source a service import reaches, in the same order. Built
/// here rather than with `IDLMergedProg`, whose lookups by name scan every
/// declaration, so that every lookup goes through an [`Index`].
struct Bundle {
    declarations: Vec<Binding>,
    actor: Option<IDLActorType>,
    imported: Vec<IDLActorType>,
}

impl Bundle {
    fn new(source_info: &SourceInfo, entry: &str) -> Result<Self, Refusal> {
        let entry_source = source_info
            .sources()
            .iter()
            .find(|source| source.name == entry)
            .ok_or_else(|| internal(format!("the entry {entry} is not among the sources")))?;
        let program = parse(entry, &entry_source.source)?;
        let mut declarations: Vec<Binding> = IDLProg::typ_decs(program.decs).collect();
        // A source joins the actor when any edge reaching it is a service
        // import.
        let services: BTreeSet<&str> = source_info
            .imports()
            .iter()
            .filter(|import| import.kind == SourceImportKind::Service)
            .map(|import| import.to.as_str())
            .collect();
        let mut imported = Vec::new();
        for source in source_info.sources() {
            if source.name == entry {
                continue;
            }
            let program = parse(&source.name, &source.source)?;
            declarations.extend(IDLProg::typ_decs(program.decs));
            if services.contains(source.name.as_str()) {
                imported.push(program.actor.ok_or_else(|| {
                    internal(format!(
                        "{} is imported as a service and declares none",
                        source.name
                    ))
                })?);
            }
        }
        Ok(Self {
            declarations,
            actor: program.actor,
            imported,
        })
    }

    /// The actor's methods, in name order, and the actor's docs: the
    /// methods `IDLMergedProg::resolve_actor` gives the compiler, the entry
    /// service's (through declaration names and a class's constructor) and
    /// then each service-imported source's, with every name looked up in
    /// `declarations`.
    fn service<'b>(
        &'b self,
        declarations: &Index<'b, &'b Binding>,
        work: &mut ProjectionWork,
    ) -> Result<(Vec<&'b Binding>, &'b [String]), Refusal> {
        if self.actor.is_none() && self.imported.is_empty() {
            return Err(internal("the bundle has no actor"));
        }
        let mut methods = Vec::new();
        for actor in self.actor.iter().chain(&self.imported) {
            methods.extend(chase(&actor.typ, declarations, work)?);
        }
        // Each service's methods come sorted from the parser; a service
        // import's follow the entry's, so the whole list is sorted again.
        methods.sort_by(|left, right| left.id.cmp(&right.id));
        let docs = self
            .actor
            .as_ref()
            .map_or(&[][..], |actor| actor.docs.as_slice());
        Ok((methods, docs))
    }
}

/// The methods of the service a type names, following declaration names and
/// a class's constructor: at most one class step and one step per
/// declaration (each name followed once; a name met twice would be a
/// cycle), then the service itself — declarations + 2 iterations.
fn chase<'b>(
    typ: &'b IDLType,
    declarations: &Index<'b, &'b Binding>,
    work: &mut ProjectionWork,
) -> Result<&'b [Binding], Refusal> {
    let mut current = typ;
    for _ in 0..declarations.len() + 2 {
        current = match current {
            IDLType::ServT(methods) => return Ok(methods),
            IDLType::ClassT(_, service) => service.as_ref(),
            IDLType::VarT(name) => {
                let binding: &'b Binding = declarations
                    .get(name, work)
                    .copied()
                    .ok_or_else(|| internal(format!("unbound service name {name}")))?;
                &binding.typ
            }
            _ => return Err(internal("the actor is not a service")),
        };
    }
    Err(internal("the actor's service names form a cycle"))
}

/// Every declaration name the types mention, directly or through other
/// declarations: each type visited once, each name looked up once.
fn reach<'b>(
    roots: impl IntoIterator<Item = &'b IDLType>,
    declarations: &Index<'b, &'b Binding>,
    work: &mut ProjectionWork,
) -> BTreeSet<&'b str> {
    let mut seen = BTreeSet::new();
    let mut pending: Vec<&'b IDLType> = roots.into_iter().collect();
    while let Some(ty) = pending.pop() {
        match ty {
            IDLType::VarT(name) => {
                if seen.insert(name.as_str()) {
                    if let Some(binding) = declarations.get(name, work).copied() {
                        pending.push(&binding.typ);
                    }
                }
            }
            IDLType::PrimT(_) | IDLType::PrincipalT => {}
            IDLType::OptT(inner) | IDLType::VecT(inner) => pending.push(inner),
            IDLType::RecordT(fields) | IDLType::VariantT(fields) => {
                pending.extend(fields.iter().map(|field| &field.typ));
            }
            IDLType::FuncT(func) => {
                pending.extend(func.args.iter().map(|arg| &arg.typ));
                pending.extend(func.rets.iter().map(|arg| &arg.typ));
            }
            IDLType::ServT(methods) => pending.extend(methods.iter().map(|method| &method.typ)),
            IDLType::ClassT(args, service) => {
                pending.extend(args.iter().map(|arg| &arg.typ));
                pending.push(service);
            }
        }
    }
    seen
}

/// The projection's text and the names of the methods it holds, in name
/// order. `methods` are names the caller has checked against the service.
/// The text is at most `limit` bytes; a projection that would take more is
/// refused with [`Refusal::TooLong`] as soon as printing reaches the bound.
pub fn project(
    source_info: &SourceInfo,
    entry: &str,
    methods: &BTreeSet<String>,
    limit: usize,
    work: &mut ProjectionWork,
) -> Result<(String, Vec<String>), Refusal> {
    let bundle = Bundle::new(source_info, entry)?;
    let declarations = Index::new(
        bundle
            .declarations
            .iter()
            .map(|binding| (binding.id.as_str(), binding)),
    );
    let (service, docs) = bundle.service(&declarations, work)?;
    let selected: Vec<&Binding> = service
        .into_iter()
        .filter(|method| methods.contains(&method.id))
        .collect();
    let reached = reach(
        selected.iter().copied().map(|method| &method.typ),
        &declarations,
        work,
    );
    let mut out = Out {
        text: String::new(),
        limit,
    };
    print(
        &mut out,
        bundle
            .declarations
            .iter()
            .filter(|binding| reached.contains(binding.id.as_str())),
        docs,
        &selected,
    )
    .map_err(|Overflow { observed }| Refusal::TooLong { limit, observed })?;
    let names = selected.iter().map(|method| method.id.clone()).collect();
    Ok((out.text, names))
}

/// The projection's text, written up to `limit` bytes: a piece that would
/// take it past the bound is not written, and stops the printing.
struct Out {
    text: String,
    limit: usize,
}

/// The text would pass its bound: `observed` counts what was written and
/// the piece that did not fit.
struct Overflow {
    observed: usize,
}

impl Out {
    fn push_str(&mut self, piece: &str) -> Result<(), Overflow> {
        if piece.len() > self.limit.saturating_sub(self.text.len()) {
            return Err(Overflow {
                observed: self.text.len().saturating_add(piece.len()),
            });
        }
        self.text.push_str(piece);
        Ok(())
    }

    fn push(&mut self, character: char) -> Result<(), Overflow> {
        self.push_str(character.encode_utf8(&mut [0; 4]))
    }
}

/// The tokens the Candid lexer reserves; any other identifier-shaped name
/// is read as an identifier and needs no quotes (`reserved`, `nat` and
/// `text` among them). `true` and `false` lex as booleans.
const KEYWORDS: &[&str] = &[
    "null",
    "vec",
    "record",
    "variant",
    "func",
    "service",
    "oneway",
    "query",
    "composite_query",
    "blob",
    "type",
    "import",
    "opt",
    "principal",
    "true",
    "false",
];

/// A method name or field label as the lexer reads it back: bare when it
/// lexes as an identifier, else a Candid text literal. Quoting only where
/// the lexer requires it matters beyond looks: upstream's parser drops the
/// doc comment of a quoted name, so `reserved` printed as `"reserved"` would
/// lose its docs.
fn name(text: &str) -> String {
    let identifier = text
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if identifier && !KEYWORDS.contains(&text) {
        return text.to_string();
    }
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            control if control.is_control() => {
                quoted.push_str(&format!("\\u{{{:x}}}", u32::from(control)));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// Doc lines, each parsing back to the same doc text. The parser strips one
/// leading run of `//` from a line comment and trims what is left, so a text
/// starting with a single `/` (which is how it reads a `///` line) is written
/// `//` + text — `/// text` again — and any other text `// ` + text.
fn docs(out: &mut Out, indent: &str, lines: &[String]) -> Result<(), Overflow> {
    for line in lines {
        out.push_str(indent)?;
        if line.is_empty() {
            out.push_str("//")?;
        } else if line.starts_with('/') && !line.starts_with("//") {
            out.push_str("//")?;
            out.push_str(line)?;
        } else {
            out.push_str("// ")?;
            out.push_str(line)?;
        }
        out.push('\n')?;
    }
    Ok(())
}

fn label(label: &Label) -> String {
    match label {
        Label::Named(text) => name(text),
        Label::Id(id) | Label::Unnamed(id) => id.to_string(),
    }
}

/// The whole projection: the declarations, then the service.
fn print<'b>(
    out: &mut Out,
    declarations: impl Iterator<Item = &'b Binding>,
    actor_docs: &[String],
    methods: &[&Binding],
) -> Result<(), Overflow> {
    for binding in declarations {
        docs(out, "", &binding.docs)?;
        out.push_str("type ")?;
        out.push_str(&binding.id)?;
        out.push_str(" = ")?;
        ty(out, "", &binding.typ)?;
        out.push_str(";\n")?;
    }
    docs(out, "", actor_docs)?;
    out.push_str("service : ")?;
    service_body(out, "", methods)?;
    out.push('\n')
}

/// One type, starting mid-line at nesting `indent`: composite types open a
/// block whose members sit one level deeper, one per line.
fn ty(out: &mut Out, indent: &str, typ: &IDLType) -> Result<(), Overflow> {
    match typ {
        IDLType::PrimT(primitive) => out.push_str(primitive_name(primitive)),
        IDLType::PrincipalT => out.push_str("principal"),
        IDLType::VarT(id) => out.push_str(id),
        IDLType::OptT(inner) => {
            out.push_str("opt ")?;
            ty(out, indent, inner)
        }
        IDLType::VecT(inner) => {
            if matches!(**inner, IDLType::PrimT(PrimType::Nat8)) {
                out.push_str("blob")
            } else {
                out.push_str("vec ")?;
                ty(out, indent, inner)
            }
        }
        IDLType::RecordT(fields) => {
            out.push_str("record ")?;
            let tuple =
                !fields.is_empty() && fields.iter().all(|f| matches!(f.label, Label::Unnamed(_)));
            if tuple && fields.iter().all(|field| field.docs.is_empty()) {
                // Tuple syntax, on one line: no element has a doc to keep.
                out.push_str("{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push_str("; ")?;
                    }
                    ty(out, indent, &field.typ)?;
                }
                out.push_str(" }")
            } else if tuple {
                // A documented element needs a line of its own for its docs:
                // tuple syntax, one element per line, labels still implicit.
                members(out, indent, fields, Members::Tuple)
            } else {
                members(out, indent, fields, Members::Record)
            }
        }
        IDLType::VariantT(fields) => {
            out.push_str("variant ")?;
            members(out, indent, fields, Members::Variant)
        }
        IDLType::FuncT(func) => {
            out.push_str("func ")?;
            signature(out, indent, func)
        }
        IDLType::ServT(methods) => {
            out.push_str("service ")?;
            service_body(out, indent, methods)
        }
        IDLType::ClassT(args, service) => {
            arguments(out, indent, args)?;
            out.push_str(" -> ")?;
            ty(out, indent, service)
        }
    }
}

/// What a `{ … }` block of fields holds, which decides how a field prints.
#[derive(Clone, Copy, PartialEq)]
enum Members {
    /// `label : type`.
    Record,
    /// `label : type`, or a bare `label` for a `null` arm.
    Variant,
    /// A tuple's elements: a bare `type`, the parser numbering them again.
    Tuple,
}

/// `{ … }` of a record, variant or documented tuple, one field per line,
/// docs above each.
fn members(
    out: &mut Out,
    indent: &str,
    fields: &[TypeField],
    kind: Members,
) -> Result<(), Overflow> {
    if fields.is_empty() {
        return out.push_str("{}");
    }
    let inner = format!("{indent}  ");
    out.push_str("{\n")?;
    for field in fields {
        docs(out, &inner, &field.docs)?;
        out.push_str(&inner)?;
        if kind == Members::Tuple {
            ty(out, &inner, &field.typ)?;
        } else {
            out.push_str(&label(&field.label))?;
            if !(kind == Members::Variant && field.typ == IDLType::PrimT(PrimType::Null)) {
                out.push_str(" : ")?;
                ty(out, &inner, &field.typ)?;
            }
        }
        out.push_str(";\n")?;
    }
    out.push_str(indent)?;
    out.push('}')
}

/// `{ … }` of a service, one method per line, docs above each.
fn service_body<B: Borrow<Binding>>(
    out: &mut Out,
    indent: &str,
    methods: &[B],
) -> Result<(), Overflow> {
    if methods.is_empty() {
        return out.push_str("{}");
    }
    let inner = format!("{indent}  ");
    out.push_str("{\n")?;
    for method in methods {
        let method = method.borrow();
        docs(out, &inner, &method.docs)?;
        out.push_str(&inner)?;
        out.push_str(&name(&method.id))?;
        out.push_str(" : ")?;
        match &method.typ {
            IDLType::FuncT(func) => signature(out, &inner, func)?,
            other => ty(out, &inner, other)?,
        }
        out.push_str(";\n")?;
    }
    out.push_str(indent)?;
    out.push('}')
}

fn signature(out: &mut Out, indent: &str, func: &FuncType) -> Result<(), Overflow> {
    arguments(out, indent, &func.args)?;
    out.push_str(" -> ")?;
    arguments(out, indent, &func.rets)?;
    for mode in &func.modes {
        out.push_str(match mode {
            FuncMode::Oneway => " oneway",
            FuncMode::Query => " query",
            FuncMode::CompositeQuery => " composite_query",
        })?;
    }
    Ok(())
}

fn arguments(out: &mut Out, indent: &str, args: &[IDLArgType]) -> Result<(), Overflow> {
    out.push('(')?;
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            out.push_str(", ")?;
        }
        if let Some(arg_name) = &arg.name {
            out.push_str(&name(arg_name))?;
            out.push_str(" : ")?;
        }
        ty(out, indent, &arg.typ)?;
    }
    out.push(')')
}

fn primitive_name(primitive: &PrimType) -> &'static str {
    match primitive {
        PrimType::Nat => "nat",
        PrimType::Nat8 => "nat8",
        PrimType::Nat16 => "nat16",
        PrimType::Nat32 => "nat32",
        PrimType::Nat64 => "nat64",
        PrimType::Int => "int",
        PrimType::Int8 => "int8",
        PrimType::Int16 => "int16",
        PrimType::Int32 => "int32",
        PrimType::Int64 => "int64",
        PrimType::Float32 => "float32",
        PrimType::Float64 => "float64",
        PrimType::Bool => "bool",
        PrimType::Text => "text",
        PrimType::Null => "null",
        PrimType::Reserved => "reserved",
        PrimType::Empty => "empty",
    }
}
