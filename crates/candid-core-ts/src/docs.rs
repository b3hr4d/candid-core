//! Doc comments and argument names: the provenance the generator turns into
//! JSDoc (issue #191).
//!
//! The Contract holds neither. They live in the `SourceInfo` sidecar, one
//! entry per *source occurrence*, and canonicalization de-duplicates
//! structurally identical nodes — so two spellings that share a node can carry
//! different docs. This module resolves that with one rule, and the generator
//! applies it at every lookup: **docs come from the occurrence written inside
//! the declaration (or the actor) whose structure the generator is emitting**.
//! A node the generator renders by a declaration's name is emitted once, under
//! that first declaration, so its docs are that declaration's; an anonymous
//! node rendered inline belongs to the declaration that contains it, and one in
//! a method signature to the method's own occurrence. Occurrences inside one
//! origin that disagree are dropped rather than merged: a doc is never shown
//! where the source did not write it.

use std::collections::BTreeMap;

use candid_core::TypeRef;

/// Where a piece of provenance was written: a named declaration, or the
/// service actor. Declaration names are unique across a bundle (a duplicate is
/// a Candid type-check error), so the source file is not part of the key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Origin {
    Declaration(String),
    Actor,
}

impl Origin {
    /// The origin of a generator context string: `"actor"` for the actor
    /// surface — a name no declaration can have, since it is refused as a
    /// reserved export name — and a declaration name otherwise.
    pub(crate) fn of(declaration: &str) -> Self {
        if declaration == "actor" {
            Self::Actor
        } else {
            Self::Declaration(declaration.to_string())
        }
    }
}

/// Doc lines for one slot; `None` marks occurrences that disagreed.
type Docs = Option<Vec<String>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MethodEntry {
    /// The AST path of the method's occurrence — the prefix its argument
    /// names hang from. `None` when two occurrences of one service node under
    /// one origin both declare this method name, so no path is authoritative.
    path: Option<String>,
    docs: Docs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArgumentEntry {
    function: TypeRef,
    position: u32,
    name: String,
}

/// Docs and argument names, keyed by origin. Built only from a compilation's
/// `SourceInfo`, so without the `compiler` feature it is always empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Provenance {
    declarations: BTreeMap<String, Docs>,
    actor: Option<Docs>,
    fields: BTreeMap<(Origin, TypeRef, u32), Docs>,
    methods: BTreeMap<(Origin, TypeRef, String), MethodEntry>,
    arguments: BTreeMap<(Origin, String), ArgumentEntry>,
}

impl Provenance {
    pub(crate) fn declaration_docs(&self, name: &str) -> &[String] {
        slice(self.declarations.get(name))
    }

    pub(crate) fn actor_docs(&self) -> &[String] {
        slice(self.actor.as_ref())
    }

    /// The docs of field `id` of `container`, from the first of `origins`
    /// that has an occurrence of it — even an undocumented one, which is the
    /// source's own answer that the field has no docs there.
    pub(crate) fn field_docs(&self, origins: &[Origin], container: TypeRef, id: u32) -> &[String] {
        origins
            .iter()
            .find_map(|origin| self.fields.get(&(origin.clone(), container, id)))
            .map_or(&[], |docs| slice(Some(docs)))
    }

    /// The docs and the argument-name path prefix of a method of `service`
    /// under `origin`.
    pub(crate) fn method(
        &self,
        origin: &Origin,
        service: TypeRef,
        name: &str,
    ) -> Option<(&[String], Option<&str>)> {
        self.methods
            .get(&(origin.clone(), service, name.to_string()))
            .map(|entry| (slice(Some(&entry.docs)), entry.path.as_deref()))
    }

    /// The name of argument `position` of a function occurrence at
    /// `function_path` under `origin`, if the source wrote one and the entry
    /// belongs to `function`.
    pub(crate) fn argument_name(
        &self,
        origin: &Origin,
        function_path: &str,
        function: TypeRef,
        position: usize,
    ) -> Option<&str> {
        let position = u32::try_from(position).ok()?;
        self.arguments
            .get(&(origin.clone(), format!("{function_path}.args[{position}]")))
            .filter(|entry| entry.function == function && entry.position == position)
            .map(|entry| entry.name.as_str())
    }
}

fn slice(docs: Option<&Docs>) -> &[String] {
    docs.and_then(|docs| docs.as_deref()).unwrap_or(&[])
}

#[cfg(feature = "compiler")]
impl Provenance {
    pub(crate) fn from_source_info(source_info: &candid_core::SourceInfo) -> Self {
        use candid_core::{SourceFunctionArgumentDirection, SourceOrigin};

        let origin_of = |origin: &SourceOrigin| match origin {
            SourceOrigin::Declaration { name, .. } => Origin::Declaration(name.clone()),
            SourceOrigin::Actor { .. } => Origin::Actor,
        };
        let mut provenance = Self::default();
        for declaration in source_info.declarations() {
            merge(
                provenance.declarations.entry(declaration.name.clone()),
                normalize_docs(&declaration.docs),
            );
        }
        for actor in source_info.actors() {
            let docs = normalize_docs(&actor.docs);
            match &mut provenance.actor {
                None => provenance.actor = Some(Some(docs)),
                Some(slot) => disagree(slot, docs),
            }
        }
        for field in source_info.field_labels() {
            merge(
                provenance
                    .fields
                    .entry((origin_of(&field.origin), field.container, field.id)),
                normalize_docs(&field.docs),
            );
        }
        for method in source_info.methods() {
            let docs = normalize_docs(&method.docs);
            let key = (
                origin_of(&method.origin),
                method.service,
                method.name.clone(),
            );
            match provenance.methods.entry(key) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(MethodEntry {
                        path: Some(method.path.clone()),
                        docs: Some(docs),
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    let entry = slot.get_mut();
                    if entry.path.as_deref() != Some(method.path.as_str()) {
                        entry.path = None;
                    }
                    disagree(&mut entry.docs, docs);
                }
            }
        }
        for argument in source_info.function_arguments() {
            if argument.direction != SourceFunctionArgumentDirection::Argument {
                continue;
            }
            provenance.arguments.insert(
                (origin_of(&argument.origin), argument.path.clone()),
                ArgumentEntry {
                    function: argument.function,
                    position: argument.position,
                    name: argument.name.clone(),
                },
            );
        }
        provenance
    }
}

/// Record `docs` under an entry, marking the slot conflicted if a different
/// value is already there.
#[cfg(feature = "compiler")]
fn merge<K: Ord>(entry: std::collections::btree_map::Entry<'_, K, Docs>, docs: Vec<String>) {
    match entry {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(Some(docs));
        }
        std::collections::btree_map::Entry::Occupied(mut slot) => disagree(slot.get_mut(), docs),
    }
}

#[cfg(feature = "compiler")]
fn disagree(slot: &mut Docs, docs: Vec<String>) {
    if slot.as_ref() != Some(&docs) {
        *slot = None;
    }
}

/// The doc lines the parser attached, cleaned for use as JSDoc text.
///
/// `candid_parser` 0.4.0 strips a doc comment's `//` marker with
/// `trim_start_matches`, which for the `///` doc form removes one pair and
/// leaves the third slash: `/// text` arrives as `/ text`. A leading slash
/// followed by whitespace (or nothing) is that residue and is removed; a
/// slash that starts a word (`/etc/hosts`) is text and stays. Control
/// characters and line separators become spaces so one entry is always one
/// physical line, and blank lines at either end are dropped.
#[cfg(feature = "compiler")]
fn normalize_docs(raw: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = raw
        .iter()
        .map(|line| {
            let cleaned: String = line
                .chars()
                .map(|c| {
                    if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                        ' '
                    } else {
                        c
                    }
                })
                .collect();
            let cleaned = cleaned.trim();
            let text = match cleaned.strip_prefix('/') {
                Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => rest,
                _ => cleaned,
            };
            text.trim().to_string()
        })
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let leading = lines.iter().take_while(|line| line.is_empty()).count();
    lines.drain(..leading);
    lines
}

/// Neutralise one doc line for a JSDoc body.
///
/// Three things in doc text are not inert, measured against the TypeScript
/// compiler's own JSDoc reader (7.0.2) and pinned by `ts/tests/jsdoc.test.ts`:
///
/// - `*/` ends the comment, so it is written `*\/`;
/// - an `@` that is not glued to a preceding letter or digit starts a JSDoc
///   tag (or, after `{`, an inline `{@link`), letting a `.did` comment forge a
///   `@param` or `@deprecated` the source never declared — or, unclosed,
///   swallow the generator's own `@param` tags. Such an `@` is written `\@`;
///   `a@b.c` stays as it is;
/// - a run of three or more backticks opens a fenced code block the reader
///   never closes, swallowing everything after it, so each backtick of such a
///   run is written `` \` ``. One or two backticks are ordinary inline code.
///
/// Everything else — braces, angle brackets, asterisks, single backticks —
/// is inert, and one line's markup never reaches the next because the
/// generator prefixes every line itself.
fn escape_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut chars = line.chars().peekable();
    let mut previous: Option<char> = None;
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'/') => {
                chars.next();
                out.push_str("*\\/");
                previous = Some('/');
                continue;
            }
            '@' if !previous.is_some_and(|p| p.is_ascii_alphanumeric()) => out.push_str("\\@"),
            '`' => {
                let mut run = 1;
                while chars.peek() == Some(&'`') {
                    chars.next();
                    run += 1;
                }
                let fence = run >= 3;
                for _ in 0..run {
                    if fence {
                        out.push('\\');
                    }
                    out.push('`');
                }
                previous = Some('`');
                continue;
            }
            c => out.push(c),
        }
        previous = Some(c);
    }
    out
}

/// A JSDoc block at `indent` levels of two spaces, ending in a newline, or
/// the empty string when there is nothing to say. `params` are `@param` names
/// and must be identifier-shaped.
pub(crate) fn doc_block(indent: usize, lines: &[String], params: &[String]) -> String {
    if lines.is_empty() && params.is_empty() {
        return String::new();
    }
    let pad = "  ".repeat(indent);
    if let ([line], []) = (lines, params) {
        return format!("{pad}/** {} */\n", escape_line(line));
    }
    let mut out = format!("{pad}/**\n");
    for line in lines {
        let line = escape_line(line);
        if line.is_empty() {
            out.push_str(&format!("{pad} *\n"));
        } else {
            out.push_str(&format!("{pad} * {line}\n"));
        }
    }
    for param in params {
        out.push_str(&format!("{pad} * @param {param}\n"));
    }
    out.push_str(&format!("{pad} */\n"));
    out
}

/// One parameter of an `Actor` method: its emitted name, and whether the
/// name is the `.did`'s own (and so earns a `@param` tag) or a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parameter {
    pub(crate) name: String,
    pub(crate) declared: bool,
}

/// Choose the emitted parameter names for a method's arguments. A name the
/// `.did` wrote is used when it is a usable TypeScript parameter name and no
/// earlier argument already took it; every other argument — unnamed, reserved
/// word, not identifier-shaped, duplicate — becomes `arg{n}`. Declared names
/// are placed first, so a fallback that would collide with one (`(arg1 : nat,
/// text)`) takes a trailing `_` instead of stealing the declared name.
pub(crate) fn resolve_parameters(declared: &[Option<String>]) -> Vec<Parameter> {
    let mut taken = std::collections::BTreeSet::new();
    let mut resolved: Vec<Option<Parameter>> = declared
        .iter()
        .map(|name| {
            let name = name.as_deref()?;
            if is_usable_parameter_name(name) && taken.insert(name.to_string()) {
                Some(Parameter {
                    name: name.to_string(),
                    declared: true,
                })
            } else {
                None
            }
        })
        .collect();
    for (position, slot) in resolved.iter_mut().enumerate() {
        if slot.is_none() {
            let mut name = format!("arg{position}");
            while !taken.insert(name.clone()) {
                name.push('_');
            }
            *slot = Some(Parameter {
                name,
                declared: false,
            });
        }
    }
    resolved.into_iter().flatten().collect()
}

/// Words a strict-mode TypeScript module cannot bind as a parameter name
/// (`this` in first position would even be read as a `this` parameter).
const UNUSABLE_PARAMETER_NAMES: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
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
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// An ASCII identifier that is not a reserved word. Non-ASCII names are legal
/// TypeScript but stay out: the fallback keeps the rule checkable by shape.
fn is_usable_parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !UNUSABLE_PARAMETER_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn a_terminator_cannot_end_the_comment() {
        for hostile in ["*/", "a */ b", "**/", "*/*/", "ends with *", "/", "* /"] {
            let block = doc_block(0, &lines(&[hostile]), &[]);
            assert_eq!(
                block.matches("*/").count(),
                1,
                "{hostile:?} must leave exactly the closing terminator: {block}"
            );
            assert!(block.trim_end().ends_with("*/"), "{block}");
            let block = doc_block(1, &lines(&["first", hostile, "last"]), &["x".to_string()]);
            assert_eq!(block.matches("*/").count(), 1, "{hostile:?}: {block}");
        }
    }

    #[test]
    fn tags_and_fences_cannot_be_forged_or_swallow_the_generated_tags() {
        let escaped = |line: &str| escape_line(line);
        // Tag starts, however they are spelled.
        assert_eq!(escaped("@param forged"), "\\@param forged");
        assert_eq!(escaped("text @param x"), "text \\@param x");
        assert_eq!(escaped("{@link X}"), "{\\@link X}");
        assert_eq!(escaped("(@x)"), "(\\@x)");
        assert_eq!(escaped("@@"), "\\@\\@");
        // An @ glued to a word is text.
        assert_eq!(escaped("mail a@b.c 1@2"), "mail a@b.c 1@2");
        // Fences: runs of three or more are escaped whole; one or two stay.
        assert_eq!(escaped("```ts"), "\\`\\`\\`ts");
        assert_eq!(escaped("a ```` b"), "a \\`\\`\\`\\` b");
        assert_eq!(escaped("`code` and ``x``"), "`code` and ``x``");
        // Terminators.
        assert_eq!(escaped("a */ b"), "a *\\/ b");
        assert_eq!(escaped("**/"), "**\\/");
        assert_eq!(escaped("*/*/"), "*\\/*\\/");
        // Plain text is untouched.
        assert_eq!(
            escaped("plain, with { braces } <b> * and /"),
            "plain, with { braces } <b> * and /"
        );
    }

    #[test]
    fn layouts() {
        assert_eq!(doc_block(0, &[], &[]), "");
        assert_eq!(doc_block(2, &lines(&["one"]), &[]), "    /** one */\n");
        assert_eq!(
            doc_block(1, &lines(&["one", "", "two"]), &["x".to_string()]),
            "  /**\n   * one\n   *\n   * two\n   * @param x\n   */\n"
        );
        assert_eq!(
            doc_block(0, &[], &["x".to_string()]),
            "/**\n * @param x\n */\n"
        );
    }

    #[test]
    fn parameter_names_fall_back_by_rule() {
        let names = |declared: &[Option<&str>]| -> Vec<(String, bool)> {
            let declared: Vec<Option<String>> = declared
                .iter()
                .map(|name| name.map(str::to_string))
                .collect();
            resolve_parameters(&declared)
                .into_iter()
                .map(|parameter| (parameter.name, parameter.declared))
                .collect()
        };
        let own = |name: &str| (name.to_string(), true);
        let fallback = |name: &str| (name.to_string(), false);
        assert_eq!(names(&[]), vec![]);
        assert_eq!(
            names(&[Some("amount"), None, Some("to")]),
            vec![own("amount"), fallback("arg1"), own("to")]
        );
        // Reserved words, a bare `this`, non-identifier and non-ASCII shapes.
        for bad in [
            "delete",
            "class",
            "this",
            "eval",
            "arguments",
            "yield",
            "await",
            "let",
            "static",
            "has space",
            "1x",
            "",
            "na\u{ef}ve",
            "a-b",
        ] {
            assert_eq!(
                names(&[Some(bad)]),
                vec![fallback("arg0")],
                "{bad:?} must fall back"
            );
        }
        // Contextual and ordinary words are fine.
        for good in ["type", "string", "of", "as", "$", "_", "a1", "Promise"] {
            assert_eq!(names(&[Some(good)]), vec![own(good)], "{good:?}");
        }
        // A duplicate falls back; a fallback never steals a declared name.
        assert_eq!(
            names(&[Some("x"), Some("x")]),
            vec![own("x"), fallback("arg1")]
        );
        assert_eq!(
            names(&[Some("arg1"), None]),
            vec![own("arg1"), fallback("arg1_")]
        );
        assert_eq!(
            names(&[None, Some("arg0")]),
            vec![fallback("arg0_"), own("arg0")]
        );
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn parser_residue_and_control_characters_are_normalised() {
        let normalised = |raw: &[&str]| normalize_docs(&lines(raw));
        assert_eq!(
            normalised(&["/ text", "/", "plain", "/etc/hosts", "//x"]),
            lines(&["text", "", "plain", "/etc/hosts", "//x"])
        );
        assert_eq!(
            normalised(&["/", "/ a\u{2028}b\tc\r", "/"]),
            lines(&["a b c"])
        );
        assert_eq!(normalised(&["/", "/"]), Vec::<String>::new());
    }
}
