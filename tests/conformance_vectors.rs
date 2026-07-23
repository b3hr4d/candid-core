//! Issue #14: manifest-driven conformance vectors for `candid-core-canon-1`.
//!
//! The raw vectors mix deliberately noncanonical arrangements with
//! already-canonical forms (which pin idempotence); the Rust implementation
//! must reproduce the pinned canonical graph, canonical JSON bytes, domain
//! preimage, and IDs exactly, without repairing the pinned expectations
//! first. The same manifest is consumed by the independent Python reference
//! (`tests/fixtures/conformance/verify_vectors.py`), so a divergence between
//! the two implementations fails one side or the other.

use candid_core::{
    compile_did_file, Actor, Contract, ContractDraft, Declaration, RawContract, TypeNode, TypeRef,
    CANONICALIZATION_PROFILE, CONTRACT_FORMAT, FORMAT_VERSION, SEMANTICS_PROFILE,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The scenarios the manifest must cover; `conformance_manifest_pins_the_
/// required_scenarios` fails if the checked-in manifest drops any of them.
const REQUIRED_CASES: [&str; 11] = [
    "actorless",
    "empty_actor",
    "class",
    "basic",
    "recursive",
    "mutual_recursion",
    "hash_collision",
    "unicode",
    "duplicate_semantic_nodes",
    "arena_permutation",
    "declaration_root_order",
];

/// Scenarios whose point is convergence of several distinct raw arenas.
const CONVERGENCE_CASES: [&str; 2] = ["duplicate_semantic_nodes", "arena_permutation"];

const CONTRACT_DOMAIN: &str = "candid-core:contract:v1";
const INTERFACE_DOMAIN: &str = "candid-core:interface:v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    required_cases: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    #[allow(dead_code)]
    description: String,
    inputs: Vec<String>,
    expected: String,
    #[serde(default)]
    did: Option<String>,
    #[serde(default)]
    wire: Option<String>,
    #[serde(default)]
    identity_pins: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGraph {
    types: Vec<TypeNode>,
    #[serde(default)]
    declarations: Vec<Declaration>,
    // Mirrors the production wire rule (and the Python reference): an absent
    // actor is omitted entirely, and an explicit `"actor": null` is a decode
    // error rather than a second spelling of "no actor".
    #[serde(default, deserialize_with = "deserialize_actor_forbidding_null")]
    actor: Option<Actor>,
}

fn deserialize_actor_forbidding_null<'de, D>(deserializer: D) -> Result<Option<Actor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Actor::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    canonical: CanonicalGraph,
    contract_identity: IdentityPins,
    #[serde(default)]
    interface_identity: Option<IdentityPins>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalGraph {
    types: Vec<TypeNode>,
    declarations: Vec<Declaration>,
    #[serde(default)]
    actor: Option<Actor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityPins {
    domain: String,
    jcs: String,
    jcs_hex: String,
    preimage_hex: String,
    id: String,
}

fn conformance_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("conformance")
}

fn read(path: &str) -> String {
    let full = conformance_dir().join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|error| panic!("reading {full:?}: {error}"))
}

fn manifest() -> Manifest {
    serde_json::from_str(&read("manifest.json")).expect("manifest.json must decode")
}

fn expected(case: &Case) -> Expected {
    serde_json::from_str(&read(&case.expected))
        .unwrap_or_else(|error| panic!("{}: decoding {}: {error}", case.name, case.expected))
}

fn build(case: &Case, input: &str) -> Contract {
    let raw: RawGraph = serde_json::from_str(&read(input))
        .unwrap_or_else(|error| panic!("{}: decoding {input}: {error}", case.name));
    ContractDraft::new(raw.types, raw.declarations, raw.actor)
        .build()
        .unwrap_or_else(|error| panic!("{}: canonicalizing {input}: {error:?}", case.name))
}

#[test]
fn conformance_manifest_pins_the_required_scenarios() {
    let manifest = manifest();
    assert_eq!(manifest.format, "candid-core-conformance-manifest");
    assert_eq!(manifest.version, 1);
    assert_eq!(
        manifest.required_cases, REQUIRED_CASES,
        "manifest required_cases drifted from the asserted scenario set"
    );

    let names: Vec<&str> = manifest
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "case names must be unique");
    for required in REQUIRED_CASES {
        assert!(
            names.contains(&required),
            "manifest is missing required case {required}"
        );
    }

    for case in &manifest.cases {
        assert!(!case.inputs.is_empty(), "{} has no raw inputs", case.name);
        if CONVERGENCE_CASES.contains(&case.name.as_str()) {
            assert!(
                case.inputs.len() >= 2,
                "{} must keep multiple noncanonical raw inputs",
                case.name
            );
        }
        // Every referenced file must exist and decode; `read` panics on a
        // missing file, the typed decodes panic on a malformed one.
        let _ = expected(case);
        for input in &case.inputs {
            let _: RawGraph = serde_json::from_str(&read(input)).unwrap();
        }
        for optional in [&case.did, &case.wire, &case.identity_pins]
            .into_iter()
            .flatten()
        {
            let _ = read(optional);
        }
    }
}

/// Every raw input reproduces the pinned canonical graph and IDs, and all
/// inputs of a case converge to one Contract. Nothing here re-canonicalizes
/// the expectations: the pins are compared as decoded.
#[test]
fn every_raw_vector_reproduces_its_pinned_canonical_graph_and_identities() {
    for case in &manifest().cases {
        let expected = expected(case);
        let contracts: Vec<Contract> = case.inputs.iter().map(|input| build(case, input)).collect();
        for (input, contract) in case.inputs.iter().zip(&contracts) {
            assert_eq!(
                contract.types(),
                expected.canonical.types,
                "{}: {input} canonical types drifted",
                case.name
            );
            assert_eq!(
                contract.declarations(),
                expected.canonical.declarations,
                "{}: {input} canonical declarations drifted",
                case.name
            );
            assert_eq!(
                contract.actor(),
                expected.canonical.actor.as_ref(),
                "{}: {input} canonical actor drifted",
                case.name
            );
            assert_eq!(
                contract.contract_id(),
                expected.contract_identity.id,
                "{}: {input} contract ID drifted",
                case.name
            );
            assert_eq!(
                contract.interface_id(),
                expected
                    .interface_identity
                    .as_ref()
                    .map(|identity| identity.id.as_str()),
                "{}: {input} interface ID drifted",
                case.name
            );
        }
        for (input, contract) in case.inputs.iter().zip(&contracts).skip(1) {
            assert_eq!(
                contract, &contracts[0],
                "{}: {input} does not converge with {}",
                case.name, case.inputs[0]
            );
        }
    }
}

/// A minimal canonical-JSON writer local to this test. It intentionally
/// duplicates the constrained profile (sorted object keys by UTF-8 bytes,
/// serde_json string escaping, plain decimal u32 numbers) so the pinned
/// canonical text is linked to the pinned canonical graph without trusting
/// the library's private writer.
fn canonical_json_bytes(value: &serde_json::Value) -> Vec<u8> {
    fn write(value: &serde_json::Value, output: &mut Vec<u8>) {
        match value {
            serde_json::Value::Number(number) => {
                let number = number
                    .as_u64()
                    .filter(|value| *value <= u64::from(u32::MAX))
                    .expect("identity payload numbers are u32");
                output.extend_from_slice(number.to_string().as_bytes());
            }
            serde_json::Value::String(string) => {
                output.extend_from_slice(serde_json::to_string(string).unwrap().as_bytes());
            }
            serde_json::Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output);
                }
                output.push(b']');
            }
            serde_json::Value::Object(object) => {
                output.push(b'{');
                let mut entries: Vec<_> = object.iter().collect();
                entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    assert!(
                        key.bytes().all(|byte| (0x20..=0x7e).contains(&byte)),
                        "identity payload object keys are fixed ASCII schema keys"
                    );
                    output.extend_from_slice(serde_json::to_string(key).unwrap().as_bytes());
                    output.push(b':');
                    write(value, output);
                }
                output.push(b'}');
            }
            other => panic!("value {other:?} is outside the identity payload vocabulary"),
        }
    }
    let mut output = Vec::new();
    write(value, &mut output);
    output
}

fn node_children(node: &TypeNode) -> Vec<TypeRef> {
    match node {
        TypeNode::Primitive { .. } => Vec::new(),
        TypeNode::Opt { inner } | TypeNode::Vec { inner } => vec![*inner],
        TypeNode::Record { fields } | TypeNode::Variant { fields } => {
            fields.iter().map(|field| field.ty).collect()
        }
        TypeNode::Func { args, results, .. } => args.iter().chain(results).copied().collect(),
        TypeNode::Service { methods } => methods.iter().map(|method| method.function).collect(),
        TypeNode::Class { init, service } => init
            .iter()
            .copied()
            .chain(std::iter::once(*service))
            .collect(),
    }
}

fn actor_reachable_prefix<'graph>(types: &'graph [TypeNode], actor: &Actor) -> &'graph [TypeNode] {
    let root = match actor {
        Actor::Service { service } => *service,
        Actor::Class { class } => *class,
    };
    let mut reached = vec![false; types.len()];
    let mut work = vec![root];
    while let Some(reference) = work.pop() {
        if reached[reference as usize] {
            continue;
        }
        reached[reference as usize] = true;
        work.extend(node_children(&types[reference as usize]));
    }
    let prefix_len = reached.iter().take_while(|reached| **reached).count();
    assert!(
        reached[prefix_len..].iter().all(|reached| !reached),
        "actor-reachable nodes must form a canonical arena prefix"
    );
    &types[..prefix_len]
}

fn assert_identity_bytes(
    case: &str,
    label: &str,
    domain: &str,
    payload: serde_json::Value,
    pins: &IdentityPins,
) {
    assert_eq!(pins.domain, domain, "{case}: {label} domain drifted");
    let canonical = canonical_json_bytes(&payload);
    assert_eq!(
        canonical,
        pins.jcs.as_bytes(),
        "{case}: {label} canonical JSON text drifted from the canonical graph"
    );
    assert_eq!(
        hex::decode(&pins.jcs_hex).unwrap(),
        canonical,
        "{case}: {label} pinned jcs_hex disagrees with the pinned text"
    );
    let mut preimage = domain.as_bytes().to_vec();
    preimage.push(0);
    preimage.extend_from_slice(&canonical);
    assert_eq!(
        hex::decode(&pins.preimage_hex).unwrap(),
        preimage,
        "{case}: {label} preimage must be domain, one zero byte, then canonical bytes"
    );
    assert_eq!(
        pins.id,
        format!("{domain}:sha256:{}", hex::encode(Sha256::digest(&preimage))),
        "{case}: {label} ID must be the SHA-256 of the pinned preimage"
    );
}

/// Golden-drift coverage for every pinned intermediate: canonical graph →
/// payload → canonical bytes → domain preimage → SHA-256 → rendered ID.
#[test]
fn pinned_identity_bytes_link_canonical_graphs_to_ids() {
    for case in &manifest().cases {
        let expected = expected(case);
        let canonical = &expected.canonical;

        let mut contract_payload = serde_json::json!({
            "format": CONTRACT_FORMAT,
            "format_version": FORMAT_VERSION,
            "semantics_profile": SEMANTICS_PROFILE,
            "canonicalization_profile": CANONICALIZATION_PROFILE,
            "types": canonical.types,
            "declarations": canonical.declarations,
        });
        if let Some(actor) = &canonical.actor {
            // An absent actor is omitted entirely from the payload; it is
            // never spelled "actor": null.
            contract_payload["actor"] = serde_json::to_value(actor).unwrap();
        }
        assert_identity_bytes(
            &case.name,
            "contract",
            CONTRACT_DOMAIN,
            contract_payload,
            &expected.contract_identity,
        );

        match (&canonical.actor, &expected.interface_identity) {
            (None, None) => {}
            (Some(actor), Some(pins)) => {
                let interface_payload = serde_json::json!({
                    "semantics_profile": SEMANTICS_PROFILE,
                    "canonicalization_profile": CANONICALIZATION_PROFILE,
                    "types": actor_reachable_prefix(&canonical.types, actor),
                    "actor": actor,
                });
                assert_identity_bytes(
                    &case.name,
                    "interface",
                    INTERFACE_DOMAIN,
                    interface_payload,
                    pins,
                );
            }
            (None, Some(_)) => panic!("{}: actorless vector pins an interface identity", case.name),
            (Some(_), None) => panic!(
                "{}: actor vector is missing its interface identity",
                case.name
            ),
        }
    }
}

/// The scenarios cannot silently degrade into weaker fixtures: the collision
/// case must keep a real 32-bit `idl_hash` collision and an id-versus-name
/// order divergence, the Unicode case must keep the orderings and escapes it
/// exists to pin, the recursion cases must keep their cycles, the
/// declaration-root case must keep its traversal-order and interface-prefix
/// distinctions, and every multi-input case must keep genuinely distinct raw
/// arenas.
#[test]
fn conformance_scenarios_keep_their_distinguishing_structure() {
    let manifest = manifest();
    let case = |name: &str| -> &Case {
        manifest
            .cases
            .iter()
            .find(|case| case.name == name)
            .unwrap_or_else(|| panic!("missing required case {name}"))
    };

    let collision = expected(case("hash_collision"));
    let collision_service = collision
        .canonical
        .types
        .iter()
        .find_map(|node| match node {
            TypeNode::Service { methods } if methods.len() >= 2 => Some(methods),
            _ => None,
        })
        .expect("hash_collision must keep a multi-method service");
    let collides = collision_service.iter().enumerate().any(|(index, left)| {
        collision_service[index + 1..]
            .iter()
            .any(|right| left.id == right.id && left.name != right.name)
    });
    assert!(
        collides,
        "hash_collision must keep two distinct method names sharing one idl_hash u32"
    );
    // Canonical method order must diverge from plain name order somewhere in
    // the suite, so an implementation sorting methods by name as the primary
    // key cannot reproduce every pinned vector.
    let method_names: Vec<&str> = collision_service
        .iter()
        .map(|method| method.name.as_str())
        .collect();
    let mut name_sorted = method_names.clone();
    name_sorted.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    assert_ne!(
        method_names, name_sorted,
        "hash_collision must pin id — not name — as the primary method sort key"
    );
    assert!(
        collision_service
            .windows(2)
            .all(|pair| pair[0].id <= pair[1].id),
        "hash_collision canonical methods must stay in ascending id order"
    );

    // declaration_root_order pins that node traversal follows quotient class
    // IDs, not declaration names: the name-sorted output declarations must
    // reference canonical nodes out of ascending order, and its
    // actor-unreachable declared types must make the interface payload a
    // strict prefix of the canonical arena.
    let root_order = expected(case("declaration_root_order"));
    let declaration_targets: Vec<TypeRef> = root_order
        .canonical
        .declarations
        .iter()
        .map(|declaration| declaration.ty)
        .collect();
    assert!(
        declaration_targets.windows(2).any(|pair| pair[0] > pair[1]),
        "declaration_root_order must keep name order diverging from traversal order; \
         found targets {declaration_targets:?}"
    );
    let root_order_actor = root_order
        .canonical
        .actor
        .as_ref()
        .expect("declaration_root_order must keep an actor");
    let prefix = actor_reachable_prefix(&root_order.canonical.types, root_order_actor);
    assert!(
        !prefix.is_empty() && prefix.len() < root_order.canonical.types.len(),
        "declaration_root_order must keep the interface payload a strict arena prefix"
    );

    let unicode = expected(case("unicode"));
    let names: Vec<&str> = unicode
        .canonical
        .declarations
        .iter()
        .map(|declaration| declaration.name.as_str())
        .collect();
    let position = |predicate: fn(&str) -> bool, what: &str| -> usize {
        names
            .iter()
            .position(|name| predicate(name))
            .unwrap_or_else(|| panic!("unicode declarations must include {what}"))
    };
    let bmp_high = position(|name| name.contains('\u{ff61}'), "a high-BMP name (U+FF61)");
    let supplementary = position(
        |name| name.contains('\u{10000}'),
        "a supplementary-plane name (U+10000)",
    );
    assert!(
        bmp_high < supplementary,
        "canonical declaration order must be UTF-8/scalar order (U+FF61 before U+10000); \
         UTF-16 code-unit order would reverse it"
    );
    position(|name| name.contains('\u{e9}'), "an NFC-composed é");
    position(|name| name.contains('\u{301}'), "an NFD combining-mark é");
    position(
        |name| name.chars().any(|ch| ch < ' ' || ch == '"' || ch == '\\'),
        "a name exercising JSON string escaping",
    );

    for name in ["recursive", "mutual_recursion"] {
        let graph = expected(case(name)).canonical.types;
        let reaches = |from: usize, to: usize| -> bool {
            let mut reached = vec![false; graph.len()];
            let mut work = node_children(&graph[from]);
            while let Some(reference) = work.pop() {
                if reached[reference as usize] {
                    continue;
                }
                reached[reference as usize] = true;
                work.extend(node_children(&graph[reference as usize]));
            }
            reached[to]
        };
        let mut cyclic_pairs = Vec::new();
        for left in 0..graph.len() {
            for right in 0..graph.len() {
                if left < right && reaches(left, right) && reaches(right, left) {
                    cyclic_pairs.push((left, right));
                }
            }
        }
        assert!(
            !cyclic_pairs.is_empty(),
            "{name} must keep a cycle crossing node boundaries"
        );
    }

    // Every multi-input case — not only the convergence scenarios — must
    // keep genuinely distinct raw documents, so a permuted variant cannot
    // silently degrade into a byte-copy of its canonical sibling.
    for case in &manifest.cases {
        let raw_inputs: Vec<serde_json::Value> = case
            .inputs
            .iter()
            .map(|input| serde_json::from_str(&read(input)).unwrap())
            .collect();
        for (index, left) in raw_inputs.iter().enumerate() {
            for right in &raw_inputs[index + 1..] {
                assert_ne!(
                    left, right,
                    "{}: multi-input cases must keep genuinely distinct raw arenas",
                    case.name
                );
            }
        }
    }
}

/// The checked-in wire fixtures are compatibility anchors: they must decode
/// exactly (no canonicalizing repair), match the pinned canonical graph and
/// IDs, and stay reproducible from their `.did` sources.
#[test]
fn legacy_wire_fixtures_stay_exact_compatibility_anchors() {
    let manifest = manifest();
    let mut anchors = 0;
    for case in &manifest.cases {
        let Some(wire_path) = &case.wire else {
            continue;
        };
        anchors += 1;
        let expected = expected(case);
        // Decode the DTO directly: nothing on this path re-canonicalizes or
        // recomputes identities, so a noncanonical checked-in fixture cannot
        // be silently repaired into passing.
        let wire: RawContract = serde_json::from_str(&read(wire_path)).unwrap();
        assert_eq!(wire.format, CONTRACT_FORMAT, "{}", case.name);
        assert_eq!(wire.format_version, FORMAT_VERSION, "{}", case.name);
        assert_eq!(wire.semantics_profile, SEMANTICS_PROFILE, "{}", case.name);
        assert_eq!(
            wire.canonicalization_profile, CANONICALIZATION_PROFILE,
            "{}",
            case.name
        );
        assert_eq!(
            wire.types, expected.canonical.types,
            "{}: wire fixture types are not the pinned canonical arena",
            case.name
        );
        assert_eq!(
            wire.declarations, expected.canonical.declarations,
            "{}: wire fixture declarations are not the pinned canonical order",
            case.name
        );
        assert_eq!(
            wire.actor, expected.canonical.actor,
            "{}: wire fixture actor drifted",
            case.name
        );
        assert_eq!(
            wire.identities.contract, expected.contract_identity.id,
            "{}: wire fixture contract ID drifted",
            case.name
        );
        assert_eq!(
            wire.identities.interface,
            expected
                .interface_identity
                .as_ref()
                .map(|identity| identity.id.clone()),
            "{}: wire fixture interface ID drifted",
            case.name
        );

        if let Some(did) = &case.did {
            let compiled = compile_did_file(conformance_dir().join(did))
                .unwrap_or_else(|error| panic!("{}: compiling {did}: {error:#?}", case.name))
                .into_parts()
                .0;
            assert_eq!(
                RawContract::from(&compiled),
                wire,
                "{}: compiling {did} no longer reproduces the wire fixture exactly",
                case.name
            );
        }

        if let Some(pins_path) = &case.identity_pins {
            let pins: serde_json::Value = serde_json::from_str(&read(pins_path)).unwrap();
            assert_eq!(
                pins["domain"], expected.contract_identity.domain,
                "{}",
                case.name
            );
            assert_eq!(pins["jcs"], expected.contract_identity.jcs, "{}", case.name);
            assert_eq!(
                pins["jcs_hex"], expected.contract_identity.jcs_hex,
                "{}",
                case.name
            );
            assert_eq!(
                pins["preimage_hex"], expected.contract_identity.preimage_hex,
                "{}",
                case.name
            );
            assert_eq!(
                pins["contract_id"], expected.contract_identity.id,
                "{}",
                case.name
            );
        }
    }
    assert_eq!(
        anchors, 5,
        "the five legacy wire fixtures must stay pinned as compatibility anchors"
    );
}
