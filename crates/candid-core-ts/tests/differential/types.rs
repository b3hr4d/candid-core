//! Random Candid type environments, rendered as `.did` source so the
//! reference (`candid_parser`) and the model under test (candid-core's
//! compiler, then `schemaFromContract`) read the very same text.
//!
//! An environment is a list of declarations `T0 … Tn` grouped in families: a
//! random base type and up to three perturbations of it (a field added,
//! dropped or retyped, `nat` widened to `int`, an `opt` wrapped or
//! unwrapped, …). Decode cases write a value at one declaration and read it
//! at another of the same family most of the time, which is where the
//! coercion rules live.

use super::rng::Rng;

#[derive(Clone, Debug)]
pub enum Lab {
    Name(&'static str),
    Id(u32),
}

impl Lab {
    pub fn id(&self) -> u32 {
        match self {
            Lab::Name(name) => candid::idl_hash(name),
            Lab::Id(id) => *id,
        }
    }

    fn render(&self) -> String {
        match self {
            Lab::Name(name) if is_identifier(name) => (*name).to_string(),
            Lab::Name(name) => quoted(name),
            Lab::Id(id) => id.to_string(),
        }
    }

    /// Another spelling of the same field id: the number, a name that
    /// hashes to it, or a name's hash twin (see `TWINS`). The field-name
    /// table attaches names to canonical nodes, so a declaration that respells
    /// a sibling's field is where `env:label-collision` lives.
    fn respell(&self) -> Option<Lab> {
        match self {
            Lab::Name(name) => Some(
                TWINS
                    .iter()
                    .find(|(one, _)| one == name)
                    .map_or(Lab::Id(candid::idl_hash(name)), |(_, twin)| Lab::Name(twin)),
            ),
            Lab::Id(id) => NAMES
                .iter()
                .find(|name| candid::idl_hash(name) == *id)
                .map(|name| Lab::Name(name)),
        }
    }
}

/// Candid's reserved words: a field so named must be quoted.
const KEYWORDS: &[&str] = &[
    "blob",
    "bool",
    "composite_query",
    "empty",
    "float32",
    "float64",
    "func",
    "import",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "nat",
    "nat8",
    "nat16",
    "nat32",
    "nat64",
    "null",
    "oneway",
    "opt",
    "principal",
    "query",
    "record",
    "reserved",
    "service",
    "text",
    "type",
    "variant",
    "vec",
];

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&name)
}

/// A Candid text literal: `"` and `\` escaped, everything else as is.
fn quoted(name: &str) -> String {
    let mut text = String::from("\"");
    for c in name.chars() {
        if c == '"' || c == '\\' {
            text.push('\\');
        }
        text.push(c);
    }
    text.push('"');
    text
}

#[derive(Clone, Debug)]
pub enum Ty {
    Prim(&'static str),
    Opt(Box<Ty>),
    Vec(Box<Ty>),
    /// Named or numbered fields; an empty list is `record {}`.
    Record(Vec<(Lab, Ty)>),
    /// `record { T0; T1; … }`: positional fields, ids `0..n`.
    Tuple(Vec<Ty>),
    Variant(Vec<(Lab, Ty)>),
    Ref(usize),
    Func {
        args: Vec<Ty>,
        rets: Vec<Ty>,
        mode: &'static str,
    },
    Service(Vec<(&'static str, Ty)>),
}

const PRIMS: &[&str] = &[
    "null",
    "bool",
    "nat",
    "int",
    "nat8",
    "nat16",
    "nat32",
    "nat64",
    "int8",
    "int16",
    "int32",
    "int64",
    "float32",
    "float64",
    "text",
    "reserved",
    "principal",
    "blob",
    "empty",
];
const PRIM_WEIGHTS: &[u32] = &[4, 4, 8, 8, 4, 3, 3, 4, 3, 3, 3, 4, 3, 3, 8, 3, 4, 4, 1];

/// Field and arm names. Besides plain identifiers: names that must be quoted
/// (non-ASCII, a space, a quote, a keyword, the empty name), and pairs whose
/// label ids coincide with each other or with a number in `IDS` (`""` hashes
/// to 0, `d` to 100, and `aaazaa`/`cctakw` to 3807829753; see `TWINS`).
const NAMES: &[&str] = &[
    "a",
    "b",
    "c",
    "d",
    "ok",
    "err",
    "head",
    "tail",
    "value",
    "x",
    "y",
    "Nat",
    "Text",
    "foo",
    "名前",
    "with space",
    "",
    "say \"hi\"",
    "nat",
    "aaazaa",
    "cctakw",
];
/// Distinct names with one label id (`idl_hash`), each to its twin.
const TWINS: &[(&str, &str)] = &[("aaazaa", "cctakw"), ("cctakw", "aaazaa")];
// Not 4_294_967_295: `candid_parser` 0.4.0 computes the next positional id
// as `id + 1` while parsing a record, which overflows (a panic under debug
// assertions) for a field labelled `u32::MAX` — upstream, outside both sides
// of this differential.
const IDS: &[u32] = &[0, 1, 2, 3, 5, 7, 100, 4_294_967_294];
const METHODS: &[&str] = &["m", "get", "put", "run"];
const MODES: &[&str] = &["", "query", "composite_query", "oneway"];

fn random_prim(rng: &mut Rng) -> &'static str {
    PRIMS[rng.weighted(PRIM_WEIGHTS)]
}

fn random_label(rng: &mut Rng) -> Lab {
    if rng.chance(3, 4) {
        Lab::Name(NAMES[rng.below(NAMES.len())])
    } else {
        Lab::Id(*rng.pick(IDS))
    }
}

/// Fields with pairwise distinct ids.
fn distinct_fields(
    rng: &mut Rng,
    count: usize,
    mut ty: impl FnMut(&mut Rng) -> Ty,
) -> Vec<(Lab, Ty)> {
    let mut fields: Vec<(Lab, Ty)> = Vec::new();
    for _ in 0..count {
        let label = random_label(rng);
        if fields.iter().any(|(other, _)| other.id() == label.id()) {
            continue;
        }
        let field_ty = ty(rng);
        fields.push((label, field_ty));
    }
    fields
}

pub fn random_ty(rng: &mut Rng, depth: usize, decls: usize) -> Ty {
    if depth >= 3 {
        return if decls > 0 && rng.chance(1, 4) {
            Ty::Ref(rng.below(decls))
        } else {
            Ty::Prim(random_prim(rng))
        };
    }
    let next = depth + 1;
    match rng.weighted(&[30, 12, 9, 15, 5, 12, 8, 3, 2]) {
        0 => Ty::Prim(random_prim(rng)),
        1 => Ty::Opt(Box::new(random_ty(rng, next, decls))),
        2 => Ty::Vec(Box::new(random_ty(rng, next, decls))),
        3 => {
            let count = rng.below(4);
            Ty::Record(distinct_fields(rng, count, |rng| {
                random_ty(rng, next, decls)
            }))
        }
        4 => {
            let count = 1 + rng.below(3);
            Ty::Tuple((0..count).map(|_| random_ty(rng, next, decls)).collect())
        }
        5 => {
            let count = 1 + rng.below(3);
            Ty::Variant(distinct_fields(rng, count, |rng| {
                random_ty(rng, next, decls)
            }))
        }
        6 if decls > 0 => Ty::Ref(rng.below(decls)),
        6 => Ty::Prim(random_prim(rng)),
        7 => random_func(rng, next, decls),
        _ => {
            let mut methods: Vec<(&'static str, Ty)> = Vec::new();
            for _ in 0..1 + rng.below(2) {
                let name = *rng.pick(METHODS);
                if methods.iter().all(|(other, _)| *other != name) {
                    methods.push((name, random_func(rng, next, decls)));
                }
            }
            Ty::Service(methods)
        }
    }
}

fn random_func(rng: &mut Rng, depth: usize, decls: usize) -> Ty {
    let mode = *rng.pick(MODES);
    let args = (0..rng.below(3))
        .map(|_| random_ty(rng, depth + 1, decls))
        .collect();
    let rets = if mode == "oneway" {
        Vec::new()
    } else {
        (0..rng.below(3))
            .map(|_| random_ty(rng, depth + 1, decls))
            .collect()
    };
    Ty::Func { args, rets, mode }
}

/// Respell one field's label (see `Lab::respell`); the id stays the same, so
/// the fields stay distinct.
fn respell_one(rng: &mut Rng, fields: &mut [(Lab, Ty)]) {
    let index = rng.below(fields.len());
    if let Some(label) = fields[index].0.respell() {
        fields[index].0 = label;
    }
}

/// A neighbouring type: the edits the subtyping and coercion rules care
/// about, applied at one random position.
pub fn perturb(rng: &mut Rng, ty: &Ty, decls: usize) -> Ty {
    match ty {
        Ty::Prim(name) => {
            let related: &[&str] = match *name {
                "nat" => &["int", "nat64", "nat8", "reserved"],
                "int" => &["nat", "int64", "float64"],
                "nat8" => &["nat16", "nat", "int8", "blob"],
                "nat16" | "nat32" => &["nat", "nat64", "int32"],
                "nat64" => &["nat", "int64"],
                "int8" | "int16" | "int32" => &["int", "int64", "nat32"],
                "int64" => &["int", "nat64"],
                "float32" => &["float64"],
                "float64" => &["float32", "int"],
                "text" => &["principal", "blob", "reserved"],
                "principal" => &["text"],
                "blob" => &["text", "principal"],
                "null" => &["reserved", "bool"],
                "reserved" => &["null", "nat"],
                "bool" => &["null", "nat8"],
                _ => &["null", "reserved"],
            };
            match rng.weighted(&[6, 2, 1, 1]) {
                0 => Ty::Prim(related[rng.below(related.len())]),
                1 => Ty::Opt(Box::new(ty.clone())),
                2 => Ty::Prim("empty"),
                _ => random_ty(rng, 2, decls),
            }
        }
        Ty::Opt(inner) => match rng.weighted(&[3, 4, 1, 1]) {
            0 => (**inner).clone(),
            1 => Ty::Opt(Box::new(perturb(rng, inner, decls))),
            2 => Ty::Opt(Box::new(Ty::Opt(inner.clone()))),
            _ => Ty::Vec(inner.clone()),
        },
        Ty::Vec(inner) => match rng.weighted(&[5, 2, 1]) {
            0 => Ty::Vec(Box::new(perturb(rng, inner, decls))),
            1 => Ty::Opt(Box::new(ty.clone())),
            _ => (**inner).clone(),
        },
        Ty::Record(fields) => {
            let mut fields = fields.clone();
            match rng.weighted(&[3, 3, 4, 1, 1, 2]) {
                0 if !fields.is_empty() => {
                    let index = rng.below(fields.len());
                    fields.remove(index);
                }
                5 if !fields.is_empty() => respell_one(rng, &mut fields),
                1 => {
                    let label = random_label(rng);
                    if fields.iter().all(|(other, _)| other.id() != label.id()) {
                        let field_ty = if rng.chance(1, 2) {
                            Ty::Opt(Box::new(random_ty(rng, 2, decls)))
                        } else {
                            random_ty(rng, 2, decls)
                        };
                        fields.push((label, field_ty));
                    }
                }
                2 if !fields.is_empty() => {
                    let index = rng.below(fields.len());
                    fields[index].1 = perturb(rng, &fields[index].1, decls);
                }
                3 => return Ty::Opt(Box::new(Ty::Record(fields))),
                4 => {
                    return Ty::Tuple(fields.into_iter().map(|(_, ty)| ty).collect());
                }
                _ => {}
            }
            Ty::Record(fields)
        }
        Ty::Tuple(elements) => {
            let mut elements = elements.clone();
            match rng.weighted(&[3, 3, 4]) {
                0 if !elements.is_empty() => {
                    elements.pop();
                }
                1 => elements.push(if rng.chance(1, 2) {
                    Ty::Opt(Box::new(random_ty(rng, 2, decls)))
                } else {
                    random_ty(rng, 2, decls)
                }),
                _ if !elements.is_empty() => {
                    let index = rng.below(elements.len());
                    elements[index] = perturb(rng, &elements[index], decls);
                }
                _ => {}
            }
            if elements.is_empty() {
                Ty::Record(Vec::new())
            } else {
                Ty::Tuple(elements)
            }
        }
        Ty::Variant(arms) => {
            let mut arms = arms.clone();
            match rng.weighted(&[3, 3, 4, 1, 2]) {
                4 if !arms.is_empty() => respell_one(rng, &mut arms),
                0 if arms.len() > 1 => {
                    let index = rng.below(arms.len());
                    arms.remove(index);
                }
                1 => {
                    let label = random_label(rng);
                    if arms.iter().all(|(other, _)| other.id() != label.id()) {
                        arms.push((label, random_ty(rng, 2, decls)));
                    }
                }
                2 if !arms.is_empty() => {
                    let index = rng.below(arms.len());
                    arms[index].1 = perturb(rng, &arms[index].1, decls);
                }
                3 => return Ty::Opt(Box::new(Ty::Variant(arms))),
                _ => {}
            }
            Ty::Variant(arms)
        }
        Ty::Ref(index) => match rng.weighted(&[3, 2, 2]) {
            0 => Ty::Ref(rng.below(decls.max(1))),
            1 => Ty::Opt(Box::new(Ty::Ref(*index))),
            _ => random_ty(rng, 2, decls),
        },
        Ty::Func { args, rets, mode } => {
            let mut args = args.clone();
            let mut rets = rets.clone();
            let mut mode = *mode;
            match rng.weighted(&[2, 3, 3, 2]) {
                0 => mode = *rng.pick(MODES),
                1 if !args.is_empty() => {
                    let index = rng.below(args.len());
                    args[index] = perturb(rng, &args[index], decls);
                }
                2 if !rets.is_empty() => {
                    let index = rng.below(rets.len());
                    rets[index] = perturb(rng, &rets[index], decls);
                }
                _ => args.push(Ty::Opt(Box::new(random_ty(rng, 2, decls)))),
            }
            if mode == "oneway" {
                rets.clear();
            }
            Ty::Func { args, rets, mode }
        }
        Ty::Service(methods) => {
            let mut methods = methods.clone();
            match rng.weighted(&[1, 1, 2]) {
                0 if methods.len() > 1 => {
                    let index = rng.below(methods.len());
                    methods.remove(index);
                }
                1 => {
                    let name = *rng.pick(METHODS);
                    if methods.iter().all(|(other, _)| *other != name) {
                        methods.push((name, random_func(rng, 2, decls)));
                    }
                }
                _ if !methods.is_empty() => {
                    let index = rng.below(methods.len());
                    methods[index].1 = perturb(rng, &methods[index].1, decls);
                }
                _ => {}
            }
            Ty::Service(methods)
        }
    }
}

pub fn decl_name(index: usize) -> String {
    format!("T{index}")
}

fn render_list(items: &[Ty]) -> String {
    items.iter().map(render).collect::<Vec<_>>().join(", ")
}

fn render_signature(args: &[Ty], rets: &[Ty], mode: &str) -> String {
    let mut text = format!("({}) -> ({})", render_list(args), render_list(rets));
    if !mode.is_empty() {
        text.push(' ');
        text.push_str(mode);
    }
    text
}

pub fn render(ty: &Ty) -> String {
    match ty {
        Ty::Prim(name) => (*name).to_string(),
        Ty::Opt(inner) => format!("opt {}", render(inner)),
        Ty::Vec(inner) => format!("vec {}", render(inner)),
        Ty::Record(fields) => {
            let body: Vec<String> = fields
                .iter()
                .map(|(label, ty)| format!("{} : {}", label.render(), render(ty)))
                .collect();
            format!("record {{ {} }}", body.join("; "))
        }
        Ty::Tuple(elements) => {
            let body: Vec<String> = elements.iter().map(render).collect();
            format!("record {{ {} }}", body.join("; "))
        }
        Ty::Variant(arms) => {
            let body: Vec<String> = arms
                .iter()
                .map(|(label, ty)| match ty {
                    Ty::Prim("null") => label.render(),
                    _ => format!("{} : {}", label.render(), render(ty)),
                })
                .collect();
            format!("variant {{ {} }}", body.join("; "))
        }
        Ty::Ref(index) => decl_name(*index),
        Ty::Func { args, rets, mode } => format!("func {}", render_signature(args, rets, mode)),
        Ty::Service(methods) => {
            let body: Vec<String> = methods
                .iter()
                .map(|(name, ty)| match ty {
                    Ty::Func { args, rets, mode } => {
                        format!("{name} : {}", render_signature(args, rets, mode))
                    }
                    other => format!("{name} : {}", render(other)),
                })
                .collect();
            format!("service {{ {} }}", body.join("; "))
        }
    }
}

/// A declaration body is never a bare reference: an alias chain could close
/// into a cycle with no constructor, which Candid rejects.
fn guard_body(ty: Ty) -> Ty {
    match ty {
        Ty::Ref(index) => Ty::Opt(Box::new(Ty::Ref(index))),
        other => other,
    }
}

pub struct Env {
    pub source: String,
    /// `families[i]` lists the declaration indices of one family.
    pub families: Vec<Vec<usize>>,
    pub decls: usize,
}

pub fn random_env(rng: &mut Rng) -> Env {
    let bases = 2 + rng.below(3);
    // Plan the declaration count first, so references may point forward.
    let mut plan: Vec<usize> = Vec::new();
    for _ in 0..bases {
        plan.push(rng.below(4));
    }
    let decls: usize = plan.iter().map(|siblings| 1 + siblings).sum();
    let mut bodies: Vec<Ty> = Vec::new();
    let mut families = Vec::new();
    for siblings in plan {
        let base_index = bodies.len();
        let base = guard_body(random_ty(rng, 0, decls));
        bodies.push(base.clone());
        let mut family = vec![base_index];
        for _ in 0..siblings {
            family.push(bodies.len());
            bodies.push(guard_body(perturb(rng, &base, decls)));
        }
        families.push(family);
    }
    let source = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| format!("type {} = {};\n", decl_name(index), render(body)))
        .collect();
    Env {
        source,
        families,
        decls,
    }
}

/// The recursive shapes of the deep environments (issue #196 review: the
/// depth regime of the iterative walkers). `T0` is the shape; the siblings
/// are a structural copy (`T1`) and a variation at the base (`T2`), so
/// decode cases also coerce across deep values. No shape is an `opt`-only
/// cycle (`T = opt T`; see `wire::opt_cycle`, and the compiler refuses one
/// since #234): the `opt` chain is pinned by exact regression vectors instead.
const DEEP_SHAPES: &[[&str; 3]] = &[
    ["opt vec T0", "opt vec T1", "opt opt vec T2"],
    ["vec T0", "vec T1", "vec opt T2"],
    [
        "record { opt T0 }",
        "record { opt T1 }",
        "record { opt T2; opt nat }",
    ],
    [
        "variant { a : T0; b }",
        "variant { a : T1; b }",
        "variant { a : T2; b : nat }",
    ],
    [
        "record { head : nat8; tail : opt T0 }",
        "record { head : nat8; tail : opt T1 }",
        "record { head : nat; tail : opt T2 }",
    ],
];

/// A deep environment: a recursive shape and its siblings, or a chain of
/// declarations `T0 = c(T1)`, `T1 = c(T2)`, … nested `chain` constructors
/// deep (a deep type table), whose values end early where an `opt` is absent,
/// a `vec` empty or a variant takes its `b` arm. The chain stays within the
/// compiler's `max_type_depth` (256), so both sides accept the environment;
/// values nest as deep as the case asks, below the generation bound.
pub fn deep_env(rng: &mut Rng) -> Env {
    if rng.chance(2, 3) {
        return shape_env(rng.pick(DEEP_SHAPES));
    }
    let chain = 16 + rng.below(240);
    chain_env(rng, chain)
}

/// How many deep kinds `deep_env_of_kind` takes: each shape of
/// `DEEP_SHAPES`, then a chain.
pub const DEEP_KINDS: usize = DEEP_SHAPES.len() + 1;

/// A deep environment of a chosen kind (the committed corpus cycles through
/// them, so every shape and a chain appear whatever the seeds draw): shape
/// `kind` of `DEEP_SHAPES`, or for `kind == DEEP_SHAPES.len()` a chain of
/// 129 to 255 constructors, a type table deeper than the generation bound
/// (`wire::GEN_LEVELS`; values stop below it).
pub fn deep_env_of_kind(rng: &mut Rng, kind: usize) -> Env {
    match DEEP_SHAPES.get(kind) {
        Some(shape) => shape_env(shape),
        None => {
            let chain = 129 + rng.below(127);
            chain_env(rng, chain)
        }
    }
}

fn shape_env(shape: &[&str; 3]) -> Env {
    let source = shape
        .iter()
        .enumerate()
        .map(|(index, body)| format!("type {} = {body};\n", decl_name(index)))
        .collect();
    Env {
        source,
        families: vec![vec![0, 1, 2]],
        decls: 3,
    }
}

fn chain_env(rng: &mut Rng, chain: usize) -> Env {
    let mut source = String::new();
    for index in 0..chain {
        let next = decl_name(index + 1);
        let body = match rng.below(4) {
            0 => format!("opt {next}"),
            1 => format!("vec {next}"),
            2 => format!("record {{ {next} }}"),
            _ => format!("variant {{ a : {next}; b }}"),
        };
        source.push_str(&format!("type {} = {body};\n", decl_name(index)));
    }
    source.push_str(&format!("type {} = nat;\n", decl_name(chain)));
    Env {
        source,
        families: vec![(0..=chain).collect()],
        decls: chain + 1,
    }
}
