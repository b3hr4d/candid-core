//! The interface projection behind `projectDid`: a `.did` holding the named
//! methods of a service and every declaration they reach, and nothing else.
//!
//! The projection works on the Candid syntax, not on the canonical Contract,
//! so the output keeps what a generator reads from the source and a Contract
//! does not hold: declaration names as written, doc comments, and argument
//! names. The caller has already compiled the input, so every source here is
//! known to parse and type-check; the bundle is merged exactly as the
//! compiler merges it (`candid_parser`'s merged program, the entry first).
//! The caller then compiles the output again, which is what proves it a
//! valid `.did` with exactly the requested methods.
//!
//! The text comes from a small printer here rather than from
//! `candid_parser`'s: that one quotes every name it treats as a keyword,
//! `reserved` and `nat` included, which the lexer reads as plain identifiers,
//! and upstream's parser drops the doc comment of a quoted name — so a
//! reprinted method named `reserved` would lose its docs. This printer
//! quotes only what the lexer requires and writes each doc line so that it
//! parses back to the same doc text. Its layout is fixed: one member per
//! line inside every non-empty record, variant and service, two spaces per
//! level, tuples and signatures on one line.
//!
//! What the output drops: every method not requested, every declaration no
//! requested method reaches, a service class's init arguments (the output is
//! `service : { … }`, the view a client calls), and the import structure — a
//! bundle projects to one self-contained file.
//!
//! Determinism: methods come out in name order (code point), the order
//! `candid_parser` gives a service's methods, whatever order the source
//! declares them or the caller names them in; record fields and variant arms
//! in label-id order, as the parser gives them; and declarations in the
//! merged program's order (the entry's in source order, then each imported
//! source's, by source ID). So the same input and the same set of names give
//! the same bytes.

use std::collections::BTreeSet;

use candid_core::{SourceImportKind, SourceInfo};
use candid_parser::candid::types::{FuncMode, Label};
use candid_parser::syntax::{
    Binding, Dec, FuncType, IDLActorType, IDLArgType, IDLMergedProg, IDLType, PrimType, TypeField,
};
use candid_parser::IDLProg;

/// Why a projection could not be built from sources the compiler accepted.
/// Never expected; the caller reports it as an internal failure.
pub struct Internal(pub String);

/// The text of a source after one leading UTF-8 byte order mark, which the
/// compiler skips the same way.
fn candid_text(source: &str) -> &str {
    source.strip_prefix('\u{FEFF}').unwrap_or(source)
}

fn parse(name: &str, source: &str) -> Result<IDLProg, Internal> {
    candid_text(source)
        .parse::<IDLProg>()
        .map_err(|error| Internal(format!("{name} no longer parses: {error}")))
}

/// The bundle's merged program, entry first, as the compiler builds it.
fn merged_program(source_info: &SourceInfo, entry: &str) -> Result<IDLMergedProg, Internal> {
    let entry_source = source_info
        .sources()
        .iter()
        .find(|source| source.name == entry)
        .ok_or_else(|| Internal(format!("the entry {entry} is not among the sources")))?;
    let mut merged = IDLMergedProg::new(parse(entry, &entry_source.source)?);
    // A source joins the actor when any edge reaching it is a service import.
    let services: BTreeSet<&str> = source_info
        .imports()
        .iter()
        .filter(|import| import.kind == SourceImportKind::Service)
        .map(|import| import.to.as_str())
        .collect();
    for source in source_info.sources() {
        if source.name == entry {
            continue;
        }
        let program = parse(&source.name, &source.source)?;
        merged
            .merge(
                services.contains(source.name.as_str()),
                source.name.clone(),
                program,
            )
            .map_err(|error| Internal(format!("merging {}: {error}", source.name)))?;
    }
    Ok(merged)
}

/// The methods of the actor's service type, following declaration names.
fn service_methods(merged: &IDLMergedProg, actor: &IDLType) -> Result<Vec<Binding>, Internal> {
    let mut current = actor.clone();
    // A chain is at most one class step, one step per declaration (each
    // name followed once; a name met twice would be a cycle), and the final
    // service: declarations + 2 iterations.
    for _ in 0..merged.bindings().count() + 2 {
        current = match current {
            IDLType::ServT(methods) => return Ok(methods),
            IDLType::ClassT(_, service) => *service,
            IDLType::VarT(name) => merged
                .lookup(&name)
                .map(|binding| binding.typ.clone())
                .ok_or_else(|| Internal(format!("unbound service name {name}")))?,
            other => return Err(Internal(format!("the actor is not a service: {other:?}"))),
        };
    }
    Err(Internal(
        "the actor's service names form a cycle".to_string(),
    ))
}

/// Every declaration name a type mentions, directly or through other
/// declarations.
fn reach(merged: &IDLMergedProg, roots: &[&IDLType]) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending: Vec<IDLType> = roots.iter().map(|root| (*root).clone()).collect();
    while let Some(ty) = pending.pop() {
        match ty {
            IDLType::VarT(name) => {
                if seen.insert(name.clone()) {
                    if let Some(binding) = merged.lookup(&name) {
                        pending.push(binding.typ.clone());
                    }
                }
            }
            IDLType::PrimT(_) | IDLType::PrincipalT => {}
            IDLType::OptT(inner) | IDLType::VecT(inner) => pending.push(*inner),
            IDLType::RecordT(fields) | IDLType::VariantT(fields) => {
                pending.extend(fields.into_iter().map(|TypeField { typ, .. }| typ));
            }
            IDLType::FuncT(func) => {
                pending.extend(func.args.into_iter().map(|arg| arg.typ));
                pending.extend(func.rets.into_iter().map(|arg| arg.typ));
            }
            IDLType::ServT(methods) => pending.extend(methods.into_iter().map(|m| m.typ)),
            IDLType::ClassT(args, service) => {
                pending.extend(args.into_iter().map(|arg| arg.typ));
                pending.push(*service);
            }
        }
    }
    seen
}

/// The projection's text and the names of the methods it holds, in output
/// order. `methods` are names the caller has checked against the service.
pub fn project(
    source_info: &SourceInfo,
    entry: &str,
    methods: &BTreeSet<String>,
) -> Result<(String, Vec<String>), Internal> {
    let merged = merged_program(source_info, entry)?;
    let actor = merged
        .resolve_actor()
        .map_err(|error| Internal(format!("resolving the actor: {error}")))?
        .ok_or_else(|| Internal("the bundle has no actor".to_string()))?;
    let selected: Vec<Binding> = service_methods(&merged, &actor.typ)?
        .into_iter()
        .filter(|method| methods.contains(&method.id))
        .collect();
    let roots: Vec<&IDLType> = selected.iter().map(|method| &method.typ).collect();
    let reached = reach(&merged, &roots);
    let decs: Vec<Dec> = merged
        .bindings()
        .filter(|binding| reached.contains(&binding.id))
        .map(|binding| Dec::TypD(binding.clone()))
        .collect();
    let names = selected.iter().map(|method| method.id.clone()).collect();
    let program = IDLProg {
        decs,
        actor: Some(IDLActorType {
            typ: IDLType::ServT(selected),
            docs: actor.docs,
        }),
    };
    Ok((print(&program), names))
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
fn docs(out: &mut String, indent: &str, lines: &[String]) {
    for line in lines {
        out.push_str(indent);
        if line.is_empty() {
            out.push_str("//");
        } else if line.starts_with('/') && !line.starts_with("//") {
            out.push_str("//");
            out.push_str(line);
        } else {
            out.push_str("// ");
            out.push_str(line);
        }
        out.push('\n');
    }
}

fn label(label: &Label) -> String {
    match label {
        Label::Named(text) => name(text),
        Label::Id(id) | Label::Unnamed(id) => id.to_string(),
    }
}

/// The whole projection: declarations, then the service.
fn print(program: &IDLProg) -> String {
    let mut out = String::new();
    for dec in &program.decs {
        if let Dec::TypD(binding) = dec {
            docs(&mut out, "", &binding.docs);
            out.push_str("type ");
            out.push_str(&binding.id);
            out.push_str(" = ");
            ty(&mut out, "", &binding.typ);
            out.push_str(";\n");
        }
    }
    if let Some(actor) = &program.actor {
        docs(&mut out, "", &actor.docs);
        out.push_str("service : ");
        if let IDLType::ServT(methods) = &actor.typ {
            service_body(&mut out, "", methods);
        }
        out.push('\n');
    }
    out
}

/// One type, starting mid-line at nesting `indent`: composite types open a
/// block whose members sit one level deeper, one per line.
fn ty(out: &mut String, indent: &str, typ: &IDLType) {
    match typ {
        IDLType::PrimT(primitive) => out.push_str(primitive_name(primitive)),
        IDLType::PrincipalT => out.push_str("principal"),
        IDLType::VarT(id) => out.push_str(id),
        IDLType::OptT(inner) => {
            out.push_str("opt ");
            ty(out, indent, inner);
        }
        IDLType::VecT(inner) => {
            if matches!(**inner, IDLType::PrimT(PrimType::Nat8)) {
                out.push_str("blob");
            } else {
                out.push_str("vec ");
                ty(out, indent, inner);
            }
        }
        IDLType::RecordT(fields) => {
            out.push_str("record ");
            if !fields.is_empty() && fields.iter().all(|f| matches!(f.label, Label::Unnamed(_))) {
                // Tuple syntax. Docs on an element have nowhere to attach.
                out.push_str("{ ");
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push_str("; ");
                    }
                    ty(out, indent, &field.typ);
                }
                out.push_str(" }");
            } else {
                members(out, indent, fields, false);
            }
        }
        IDLType::VariantT(fields) => {
            out.push_str("variant ");
            members(out, indent, fields, true);
        }
        IDLType::FuncT(func) => {
            out.push_str("func ");
            signature(out, indent, func);
        }
        IDLType::ServT(methods) => {
            out.push_str("service ");
            service_body(out, indent, methods);
        }
        IDLType::ClassT(args, service) => {
            arguments(out, indent, args);
            out.push_str(" -> ");
            ty(out, indent, service);
        }
    }
}

/// `{ … }` of a record or variant, one field per line, docs above each.
fn members(out: &mut String, indent: &str, fields: &[TypeField], variant: bool) {
    if fields.is_empty() {
        out.push_str("{}");
        return;
    }
    let inner = format!("{indent}  ");
    out.push_str("{\n");
    for field in fields {
        docs(out, &inner, &field.docs);
        out.push_str(&inner);
        out.push_str(&label(&field.label));
        if !(variant && field.typ == IDLType::PrimT(PrimType::Null)) {
            out.push_str(" : ");
            ty(out, &inner, &field.typ);
        }
        out.push_str(";\n");
    }
    out.push_str(indent);
    out.push('}');
}

/// `{ … }` of a service, one method per line, docs above each.
fn service_body(out: &mut String, indent: &str, methods: &[Binding]) {
    if methods.is_empty() {
        out.push_str("{}");
        return;
    }
    let inner = format!("{indent}  ");
    out.push_str("{\n");
    for method in methods {
        docs(out, &inner, &method.docs);
        out.push_str(&inner);
        out.push_str(&name(&method.id));
        out.push_str(" : ");
        match &method.typ {
            IDLType::FuncT(func) => signature(out, &inner, func),
            other => ty(out, &inner, other),
        }
        out.push_str(";\n");
    }
    out.push_str(indent);
    out.push('}');
}

fn signature(out: &mut String, indent: &str, func: &FuncType) {
    arguments(out, indent, &func.args);
    out.push_str(" -> ");
    arguments(out, indent, &func.rets);
    for mode in &func.modes {
        out.push_str(match mode {
            FuncMode::Oneway => " oneway",
            FuncMode::Query => " query",
            FuncMode::CompositeQuery => " composite_query",
        });
    }
}

fn arguments(out: &mut String, indent: &str, args: &[IDLArgType]) {
    out.push('(');
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        if let Some(arg_name) = &arg.name {
            out.push_str(&name(arg_name));
            out.push_str(" : ");
        }
        ty(out, indent, &arg.typ);
    }
    out.push(')');
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
