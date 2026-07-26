//! Detached, exact-octet artifact identity (issue #25).
//!
//! `contract_id` and `interface_id` are *semantic Contract identities*. Two
//! documents that mean the same thing share them on purpose, which is exactly
//! why identifying a whole file by its `contract_id` is wrong: rewriting
//! `producer`, editing an envelope extension, replacing the provenance sidecar,
//! and re-encoding the JSON all leave both untouched. An artifact ID covers
//! those bytes and claims nothing else.
//!
//! `source_bundle_id` is a third thing again — a raw-source bundle content
//! identity over raw source bytes and import edges, which comments and
//! formatting inside a source therefore do move, while data derived from those
//! sources never enters it.
//!
//! Every case below comes in two halves — the artifact identity moves, and the
//! identity whose scope excludes the edit does not — because a test that only
//! proved the first half would pass just as happily if the boundary had been
//! erased in the other direction.
//!
//! The vectors are checked twice: here, and independently by
//! `tests/fixtures/artifact-identity/verify_artifact_ids.py`, which shares the
//! manifest and no code.

use candid_core::{
    artifact_id_with_context, artifact_id_with_limits, ArtifactKind, CancellationToken, Contract,
    ContractDraft, ContractEnvelope, ContractValidationError, Limits, ProducerInfo, RuntimeContext,
};
use serde_json::Value;
use std::path::PathBuf;

const CONTRACT: ArtifactKind = ArtifactKind::ContractJsonV1;
const ENVELOPE: ArtifactKind = ArtifactKind::ContractEnvelopeJsonV1;
const COMPILATION: ArtifactKind = ArtifactKind::CompilationJsonV1;
const RESOURCE: &str = "artifact_identity_work";
const CONTRACT_DOMAIN: &str = "candid-core:artifact:contract-json:v1";
const ENVELOPE_DOMAIN: &str = "candid-core:artifact:contract-envelope-json:v1";
const COMPILATION_DOMAIN: &str = "candid-core:artifact:compilation-json:v1";

/// Every kind and its frozen domain, in the order `manifest.json` lists them.
const KINDS: &[(ArtifactKind, &str)] = &[
    (CONTRACT, CONTRACT_DOMAIN),
    (ENVELOPE, ENVELOPE_DOMAIN),
    (COMPILATION, COMPILATION_DOMAIN),
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/artifact-identity")
}

fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(fixtures().join("manifest.json")).unwrap()).unwrap()
}

fn kind_of(name: &str) -> ArtifactKind {
    match name {
        "ContractJsonV1" => CONTRACT,
        "ContractEnvelopeJsonV1" => ENVELOPE,
        "CompilationJsonV1" => COMPILATION,
        other => panic!("unknown artifact kind in the manifest: {other}"),
    }
}

/// The kind a manifest vector declares.
fn vector_kind(name: &str) -> ArtifactKind {
    kind_of(vector(name)["kind"].as_str().unwrap())
}

fn artifact_bytes(relative: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(relative)).unwrap()
}

/// The named manifest vector.
fn vector(name: &str) -> Value {
    manifest()["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["name"] == name)
        .unwrap_or_else(|| panic!("no manifest vector named {name}"))
        .clone()
}

/// Bytes of the named manifest vector.
fn vector_bytes(name: &str) -> Vec<u8> {
    artifact_bytes(vector(name)["file"].as_str().unwrap())
}

fn id(kind: ArtifactKind, bytes: &[u8]) -> String {
    artifact_id_with_limits(kind, bytes, &Limits::default()).unwrap()
}

/// Exact `artifact_identity_work` cost of hashing `len` bytes under `domain`:
/// one unit per byte plus the domain tag and its single separator byte.
fn exact_work(domain: &str, len: usize) -> usize {
    domain.len() + 1 + len
}

fn resource_failure(error: &ContractValidationError) -> (String, u64, u64) {
    let violation = &error.violations[0];
    assert_eq!(violation.code, "resource_limit_exceeded", "{error:#?}");
    let info = violation
        .resource_limit
        .as_ref()
        .expect("resource limit failures must retain metadata");
    (info.resource.clone(), info.limit, info.observed)
}

// ---------------------------------------------------------------------------
// Golden vectors, shared with the independent Python reference.
// ---------------------------------------------------------------------------

#[test]
fn every_manifest_vector_reproduces_its_pinned_artifact_id() {
    let manifest = manifest();
    let vectors = manifest["vectors"].as_array().unwrap();
    assert!(!vectors.is_empty());

    for vector in vectors {
        let name = vector["name"].as_str().unwrap();
        let kind = kind_of(vector["kind"].as_str().unwrap());
        let bytes = artifact_bytes(vector["file"].as_str().unwrap());
        assert_eq!(
            id(kind, &bytes),
            vector["artifact_id"].as_str().unwrap(),
            "{name}"
        );
    }

    // A dropped fixture must fail here rather than shrink the evidence.
    let names: Vec<&str> = vectors
        .iter()
        .map(|vector| vector["name"].as_str().unwrap())
        .collect();
    for required in manifest["required_cases"].as_array().unwrap() {
        let required = required.as_str().unwrap();
        assert!(
            names.contains(&required),
            "missing required case {required}"
        );
    }
    assert!(
        names.contains(&"empty"),
        "the framing anchor must stay in the manifest"
    );
}

/// The rendered shape is part of the contract: the domain, then `:sha256:`,
/// then exactly 64 lowercase hex digits.
#[test]
fn a_rendered_id_is_its_own_domain_plus_sha256_hex() {
    for &(kind, domain) in KINDS {
        let rendered = id(kind, b"{}");
        let digest = rendered
            .strip_prefix(&format!("{domain}:sha256:"))
            .unwrap_or_else(|| panic!("{rendered} must carry its own domain"));
        assert_eq!(digest.len(), 64);
        assert!(digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
}

/// The manifest's `domains` map and this crate's frozen domains must agree, in
/// both directions: a kind Rust renders but the manifest omits, or a domain the
/// manifest spells differently, fails here rather than in review.
#[test]
fn the_manifest_declares_exactly_this_crates_frozen_domains() {
    let manifest = manifest();
    let declared = manifest["domains"].as_object().unwrap();
    assert_eq!(declared.len(), KINDS.len());
    for (name, domain) in declared {
        let domain = domain.as_str().unwrap();
        assert!(
            id(kind_of(name), b"").starts_with(&format!("{domain}:sha256:")),
            "{name}: the manifest domain must be the one this crate renders"
        );
    }
    for &(_, domain) in KINDS {
        assert!(
            declared.values().any(|declared| declared == domain),
            "{domain} is missing from the manifest"
        );
    }
}

#[test]
fn identical_bytes_under_different_kinds_are_different_identities() {
    let manifest = manifest();
    let entries = manifest["cross_kind"].as_array().unwrap();
    assert!(!entries.is_empty());

    for entry in entries {
        let name = entry["vector"].as_str().unwrap();
        let bytes = vector_bytes(name);
        let declared = vector_kind(name);
        let other = kind_of(entry["kind"].as_str().unwrap());
        assert_ne!(other, declared, "{name}: a cross-kind pin must cross kinds");

        let rehashed = id(other, &bytes);
        assert_eq!(rehashed, entry["artifact_id"].as_str().unwrap(), "{name}");
        assert_ne!(
            rehashed,
            id(declared, &bytes),
            "{name}: the domain must separate the digest space"
        );
    }
}

/// The same statement without the manifest in the way: one byte sequence yields
/// as many distinct identities as there are kinds, every kind included.
#[test]
fn every_kind_gives_one_byte_sequence_its_own_identity() {
    for bytes in [b"".as_slice(), b"{}", &vector_bytes("contract")] {
        let rendered: Vec<String> = KINDS.iter().map(|&(kind, _)| id(kind, bytes)).collect();
        for (position, one) in rendered.iter().enumerate() {
            for other in &rendered[position + 1..] {
                assert_ne!(one, other, "{} bytes", bytes.len());
            }
        }
        assert_eq!(rendered.len(), KINDS.len());
    }
}

/// Golden artifact IDs for raw Contract JSON, pinned as literals here as well
/// as in the manifest and — for the framing anchor — in `src/artifact_id.rs`,
/// so moving one of them means editing three files that were written apart.
#[test]
fn raw_contract_json_has_pinned_golden_artifact_ids() {
    // The framing anchor for this domain: no artifact bytes at all, so the tag
    // and its single 0x00 separator are the whole preimage.
    assert_eq!(
        id(CONTRACT, b""),
        "candid-core:artifact:contract-json:v1:sha256:66c1371d29c896c2b292edc5dc1d344bf39103c5a1011141ed6883ace3e95401"
    );
    assert_eq!(
        id(CONTRACT, &vector_bytes("contract")),
        "candid-core:artifact:contract-json:v1:sha256:db27c5c23308bcfb793ac04b8e290261c90516e35ef4cbf67c1261e1c41f5d9c"
    );
    assert_eq!(
        id(CONTRACT, &vector_bytes("contract_producer")),
        "candid-core:artifact:contract-json:v1:sha256:ef28cfa48e30fb103fd5153a5c195e8118049e3823c499b41145353f007ab530"
    );
}

// ---------------------------------------------------------------------------
// Exact octets: what moves the artifact identity.
// ---------------------------------------------------------------------------

#[test]
fn one_changed_byte_changes_the_artifact_id() {
    let bytes = vector_bytes("contract_envelope");
    let baseline = id(ENVELOPE, &bytes);

    let mut flipped = bytes.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x01;
    assert_ne!(flipped, bytes);
    assert_ne!(id(ENVELOPE, &flipped), baseline);
}

#[test]
fn whitespace_changes_the_artifact_id() {
    let bytes = vector_bytes("contract_envelope");
    let baseline = id(ENVELOPE, &bytes);

    let mut with_newline = bytes.clone();
    with_newline.push(b'\n');
    assert_ne!(id(ENVELOPE, &with_newline), baseline);

    // The same JSON value from a different encoder: same meaning, no
    // whitespace, different octets. That difference is the whole claim.
    let compact = vector_bytes("contract_envelope_compact");
    assert_ne!(compact, bytes);
    assert_ne!(id(ENVELOPE, &compact), baseline);
    assert_eq!(
        serde_json::from_slice::<Value>(&compact).unwrap(),
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        "the compact vector must be the identical JSON value"
    );
}

/// Extensions live outside `contract_id` by design, so an envelope whose
/// extension name or value changed keeps both semantic identities and must not
/// keep its artifact identity.
#[test]
fn extension_edits_move_the_artifact_id_and_not_the_semantic_ids() {
    let limits = Limits::default();
    let baseline = vector_bytes("contract_envelope");
    let base_envelope = ContractEnvelope::from_slice_with_limits(&baseline, &limits).unwrap();

    for edited in [
        "contract_envelope_extension_name",
        "contract_envelope_extension_value",
    ] {
        let bytes = vector_bytes(edited);
        assert_ne!(bytes, baseline, "{edited}");
        assert_ne!(id(ENVELOPE, &bytes), id(ENVELOPE, &baseline), "{edited}");

        let envelope = ContractEnvelope::from_slice_with_limits(&bytes, &limits).unwrap();
        assert_ne!(
            envelope.extensions(),
            base_envelope.extensions(),
            "{edited}: the fixture must genuinely differ in its extensions"
        );
        assert_eq!(
            envelope.contract().contract_id(),
            base_envelope.contract().contract_id(),
            "{edited}: contract_id must not move"
        );
        assert_eq!(
            envelope.contract().interface_id(),
            base_envelope.contract().interface_id(),
            "{edited}: interface_id must not move"
        );
    }
}

/// A raw Contract document contains `producer`, and no semantic Contract
/// identity does. Mutating `ProducerInfo` in the checked-in Contract bytes
/// therefore moves the Contract artifact ID, while the bounded parse returns
/// `contract_id` and `interface_id` byte-identical — the precise reason a
/// semantic identity cannot stand in for an artifact identity.
#[test]
fn mutating_producer_in_raw_contract_bytes_moves_only_the_artifact_id() {
    let limits = Limits::default();
    let baseline_bytes = vector_bytes("contract");
    let rebranded_bytes = vector_bytes("contract_producer");
    assert_ne!(rebranded_bytes, baseline_bytes);
    assert_ne!(
        id(CONTRACT, &rebranded_bytes),
        id(CONTRACT, &baseline_bytes),
        "producer bytes are inside a raw Contract artifact"
    );

    let baseline = Contract::from_slice_with_limits(&baseline_bytes, &limits).unwrap();
    let rebranded = Contract::from_slice_with_limits(&rebranded_bytes, &limits).unwrap();
    assert_ne!(
        rebranded.producer(),
        baseline.producer(),
        "the fixture pair must genuinely differ in producer"
    );
    assert_eq!(rebranded.contract_id(), baseline.contract_id());
    assert_eq!(rebranded.interface_id(), baseline.interface_id());

    // And it is the *same* semantic Contract the envelope and compilation
    // vectors carry, recomputed by the bounded parse rather than trusted from
    // the wire — so the manifest's shared-`contract_id` group is anchored from
    // Rust too, not only by the Python reference.
    let shared = manifest()["shared_contract_id"]["contract_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(baseline.contract_id(), shared);
    assert_eq!(rebranded.contract_id(), shared);

    // Nothing but `producer` moved, so the artifact ID above cannot be crediting
    // some other incidental difference between the two fixtures.
    let without_producer = |bytes: &[u8]| {
        let mut document: Value = serde_json::from_slice(bytes).unwrap();
        assert!(
            document
                .as_object_mut()
                .unwrap()
                .remove("producer")
                .is_some(),
            "a raw Contract document must carry producer"
        );
        document
    };
    assert_eq!(
        without_producer(&rebranded_bytes),
        without_producer(&baseline_bytes)
    );
}

/// The issue's original example, live rather than from a fixture: `producer` is
/// caller-supplied provenance that `contract_id` deliberately excludes, so a
/// rewritten producer is invisible to both semantic Contract identities and
/// visible to the artifact identity.
#[test]
fn rewritten_producer_bytes_move_the_artifact_id_and_not_the_semantic_ids() {
    let limits = Limits::default();
    let original =
        ContractEnvelope::from_slice_with_limits(&vector_bytes("contract_envelope"), &limits)
            .unwrap();
    let contract = original.contract();
    let draft = ContractDraft::new(
        contract.types().to_vec(),
        contract.declarations().to_vec(),
        contract.actor().cloned(),
    );

    let rebranded = draft
        .clone()
        .with_producer(ProducerInfo {
            name: "candid-core-fork".to_string(),
            version: "9.9.9".to_string(),
            ..contract.producer().clone()
        })
        .build_with_limits(&limits)
        .unwrap();
    let unchanged = draft
        .with_producer(contract.producer().clone())
        .build_with_limits(&limits)
        .unwrap();

    assert_ne!(rebranded.producer(), unchanged.producer());
    assert_eq!(rebranded.contract_id(), unchanged.contract_id());
    assert_eq!(rebranded.interface_id(), unchanged.interface_id());

    let render = |contract: &candid_core::Contract| {
        ContractEnvelope::new(contract.clone())
            .to_json_pretty_with_limits(&limits)
            .unwrap()
    };
    let rebranded_bytes = render(&rebranded);
    let unchanged_bytes = render(&unchanged);
    assert_ne!(
        id(ENVELOPE, rebranded_bytes.as_bytes()),
        id(ENVELOPE, unchanged_bytes.as_bytes()),
        "producer bytes are part of the artifact even though no semantic ID sees them"
    );
}

/// The `compiler` half of the same boundary: a `SourceInfo` sidecar is part of
/// a compilation document and part of no semantic Contract identity.
#[cfg(feature = "compiler")]
#[test]
fn sidecar_and_producer_edits_move_the_artifact_id_and_not_the_semantic_ids() {
    use candid_core::Compilation;

    let limits = Limits::default();
    let baseline_bytes = vector_bytes("compilation");
    let baseline = Compilation::from_slice_with_limits(&baseline_bytes, &limits).unwrap();

    for edited in ["compilation_source_doc", "compilation_producer"] {
        let bytes = vector_bytes(edited);
        assert_ne!(bytes, baseline_bytes, "{edited}");
        assert_ne!(
            id(COMPILATION, &bytes),
            id(COMPILATION, &baseline_bytes),
            "{edited}"
        );

        let compilation = Compilation::from_slice_with_limits(&bytes, &limits).unwrap();
        assert_eq!(
            compilation.contract().contract_id(),
            baseline.contract().contract_id(),
            "{edited}: contract_id must not move"
        );
        assert_eq!(
            compilation.contract().interface_id(),
            baseline.contract().interface_id(),
            "{edited}: interface_id must not move"
        );
    }

    // Each edit must have moved the part of the document it claims to.
    let redocumented =
        Compilation::from_slice_with_limits(&vector_bytes("compilation_source_doc"), &limits)
            .unwrap();
    assert_ne!(
        redocumented.source_info().unwrap().source_bundle_id(),
        baseline.source_info().unwrap().source_bundle_id(),
        "the doc-comment vector must change the source bundle it identifies"
    );
    let rebranded =
        Compilation::from_slice_with_limits(&vector_bytes("compilation_producer"), &limits)
            .unwrap();
    assert_ne!(
        rebranded.contract().producer(),
        baseline.contract().producer(),
        "the producer vector must genuinely differ in producer"
    );
}

/// The sharpest version of the same point, and the one no existing identity
/// covers at all: a *derived* `SourceInfo` field.
///
/// `source_bundle_id` is a raw-source bundle content identity — it covers raw
/// source bytes and import edges and nothing derived from them. An edit confined
/// to a rederived doc string therefore need not move it, and here does not, so
/// all three of `contract_id`, `interface_id`, and `source_bundle_id` come back
/// byte-identical. (A raw source or import-edge change is the other case: that
/// does move `source_bundle_id`, which is what
/// `sidecar_and_producer_edits_move_the_artifact_id_and_not_the_semantic_ids`
/// shows with the `compilation_source_doc` vector.) Only the artifact ID moves
/// here.
///
/// The edit is deliberately performed on bytes rather than through the API:
/// the resulting document would fail rederivation, which is the second half of
/// the claim. An artifact identity exists for bytes that are not a valid
/// artifact, because computing one is not a validity claim.
#[test]
fn a_derived_source_info_edit_moves_only_the_artifact_id() {
    const DERIVED_DOCS: &[u8] = b"\"/ An item.\"";
    const CONTRACT_ID: &[u8] =
        b"candid-core:contract:v1:sha256:5b4d7090e72cefb298b0d5e2941dc2ed1f6ac44957163d75dd406ac5e30c930d";
    const SOURCE_BUNDLE_ID: &[u8] =
        b"candid-core:source-bundle:v1:sha256:52cb9ba7ed7105a36de5a5f2665ef82261080406133498ed4aa8b3cac6f9bcca";

    let bytes = vector_bytes("compilation");
    let text = String::from_utf8(bytes.clone()).expect("the fixture is UTF-8");
    let needle = std::str::from_utf8(DERIVED_DOCS).unwrap();
    assert_eq!(
        text.matches(needle).count(),
        1,
        "the derived docs entry must be the only occurrence, so the edit is unambiguous"
    );
    let edited = text.replace(needle, "\"/ A ledger item.\"").into_bytes();
    assert_ne!(edited, bytes);

    // Every identity string in the document is still present, unchanged, and as
    // many times as before — the edit touched only derived provenance.
    for (label, id_bytes) in [
        ("contract_id", CONTRACT_ID),
        ("source_bundle_id", SOURCE_BUNDLE_ID),
    ] {
        assert_eq!(
            count_occurrences(&edited, id_bytes),
            count_occurrences(&bytes, id_bytes),
            "{label} must be untouched by a derived-provenance edit"
        );
        assert!(count_occurrences(&edited, id_bytes) > 0, "{label}");
    }

    assert_ne!(
        id(COMPILATION, &edited),
        id(COMPILATION, &bytes),
        "a derived SourceInfo field is inside the artifact and inside none of the three content IDs"
    );
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

// ---------------------------------------------------------------------------
// Resources, precedence, and failing closed.
// ---------------------------------------------------------------------------

#[test]
fn max_input_bytes_is_enforced_before_hashing() {
    let bytes = vector_bytes("contract_envelope");
    let limits = Limits::default().with_max_input_bytes(bytes.len() - 1);
    let error = artifact_id_with_limits(ENVELOPE, &bytes, &limits).unwrap_err();
    assert_eq!(
        resource_failure(&error),
        (
            "input_bytes".to_string(),
            (bytes.len() - 1) as u64,
            bytes.len() as u64
        )
    );

    // Exactly at the gate the artifact still hashes.
    artifact_id_with_limits(
        ENVELOPE,
        &bytes,
        &Limits::default().with_max_input_bytes(bytes.len()),
    )
    .expect("the byte gate must accept an artifact of exactly its size");
}

/// The byte gate wins even when the work budget is exhausted too, so an
/// oversized artifact always reports the same resource.
#[test]
fn the_byte_gate_takes_precedence_over_the_work_budget() {
    let bytes = vector_bytes("compilation");
    let limits = Limits::default()
        .with_max_input_bytes(1)
        .with_max_artifact_identity_work(0);
    let error = artifact_id_with_limits(COMPILATION, &bytes, &limits).unwrap_err();
    assert_eq!(resource_failure(&error).0, "input_bytes");
}

#[test]
fn artifact_identity_work_succeeds_at_the_exact_bound_and_fails_one_unit_below() {
    for (kind, domain, name) in [
        (CONTRACT, CONTRACT_DOMAIN, "contract"),
        (ENVELOPE, ENVELOPE_DOMAIN, "contract_envelope"),
        (COMPILATION, COMPILATION_DOMAIN, "compilation"),
    ] {
        let bytes = vector_bytes(name);
        let work = exact_work(domain, bytes.len());

        artifact_id_with_limits(
            kind,
            &bytes,
            &Limits::default().with_max_artifact_identity_work(work),
        )
        .unwrap_or_else(|error| panic!("{name}: the exact bound must succeed: {error:#?}"));

        let error = artifact_id_with_limits(
            kind,
            &bytes,
            &Limits::default().with_max_artifact_identity_work(work - 1),
        )
        .unwrap_err();
        assert_eq!(
            resource_failure(&error),
            (RESOURCE.to_string(), (work - 1) as u64, work as u64),
            "{name}"
        );
    }
}

/// Artifact identity has its own counter. Zeroing both pre-existing identity
/// budgets must not affect it, and — the direction that matters for a shared
/// budget — hashing must not be able to starve them.
#[test]
fn no_other_work_counter_is_consumed() {
    let bytes = vector_bytes("compilation");
    let limits = Limits::default()
        .with_max_canonicalization_work(0)
        .with_max_source_identity_work(0);
    assert_eq!(
        artifact_id_with_limits(COMPILATION, &bytes, &limits).unwrap(),
        id(COMPILATION, &bytes),
        "artifact identity must not consume canonicalization or source identity work"
    );
}

#[test]
fn an_empty_artifact_costs_only_the_domain_framing() {
    for &(kind, domain) in KINDS {
        let framing = exact_work(domain, 0);
        artifact_id_with_limits(
            kind,
            b"",
            &Limits::default().with_max_artifact_identity_work(framing),
        )
        .expect("framing alone must fit the framing bound");
        let error = artifact_id_with_limits(
            kind,
            b"",
            &Limits::default().with_max_artifact_identity_work(framing - 1),
        )
        .unwrap_err();
        assert_eq!(
            resource_failure(&error),
            (RESOURCE.to_string(), (framing - 1) as u64, framing as u64)
        );
    }
}

#[test]
fn cancellation_fails_closed() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let context = RuntimeContext::new(Limits::default()).with_cancellation(cancellation);
    let error =
        artifact_id_with_context(COMPILATION, &vector_bytes("compilation"), &context).unwrap_err();
    assert_eq!(error.violations[0].code, "operation_cancelled");
    assert!(error.violations[0].resource_limit.is_none());
}

#[test]
fn an_elapsed_deadline_fails_closed() {
    let context = RuntimeContext::new(Limits::default().with_deadline_unix_ms(Some(1)));
    let error =
        artifact_id_with_context(COMPILATION, &vector_bytes("compilation"), &context).unwrap_err();
    assert_eq!(error.violations[0].code, "operation_deadline_exceeded");
}

/// Nothing about artifact identity is implicit. Every existing entry point must
/// behave exactly as it did, including under a zero artifact-identity budget.
#[test]
fn no_existing_entry_point_computes_an_artifact_identity() {
    let bytes = vector_bytes("contract_envelope");
    let limits = Limits::default().with_max_artifact_identity_work(0);
    let envelope = ContractEnvelope::from_slice_with_limits(&bytes, &limits)
        .expect("decoding must not charge artifact identity work");
    envelope
        .validate(&limits)
        .expect("validation must not charge artifact identity work");
    let rendered = envelope
        .to_json_pretty_with_limits(&limits)
        .expect("serialization must not charge artifact identity work");
    assert!(
        !rendered.contains("artifact"),
        "no serialized artifact may gain an artifact_id field"
    );
}

/// `Limits::default()` must admit any artifact its own byte gate admits, for
/// every kind. Two defaults that contradicted each other would make a
/// maximum-size artifact unhashable out of the box, and a new kind with a longer
/// domain is exactly how that could happen silently.
#[test]
fn the_default_work_limit_covers_the_default_byte_gate() {
    let limits = Limits::default();
    for &(_, domain) in KINDS {
        assert!(
            exact_work(domain, limits.max_input_bytes()) <= limits.max_artifact_identity_work(),
            "{domain} at the byte gate must fit the default work limit"
        );
    }
    // The documented worst case, named rather than merely satisfied.
    assert_eq!(
        KINDS.iter().map(|(_, domain)| domain.len()).max(),
        Some(ENVELOPE_DOMAIN.len())
    );
    assert_eq!(
        exact_work(ENVELOPE_DOMAIN, limits.max_input_bytes()),
        4_194_351
    );
    assert_eq!(limits.max_artifact_identity_work(), 10_000_000);
}

// ---------------------------------------------------------------------------
// The additive `Limits` override, and what it does and does not change.
// ---------------------------------------------------------------------------

/// Adding an override key is one-directional by construction: documents that do
/// not set it are unchanged in both directions, and a document that does set it
/// is rejected — not silently ignored — by a build that predates the key.
#[test]
fn the_new_limit_override_is_additive_and_one_directional() {
    // A default configuration still serializes with no overrides at all, so
    // every previously written document still round-trips byte for byte.
    assert_eq!(
        serde_json::to_string(&Limits::default()).unwrap(),
        r#"{"version":1,"profile":"interactive_v1","overrides":{}}"#
    );

    // An unrelated override still serializes exactly as it did.
    assert_eq!(
        serde_json::to_string(&Limits::default().with_max_input_bytes(512)).unwrap(),
        r#"{"version":1,"profile":"interactive_v1","overrides":{"max_input_bytes":512}}"#
    );

    // The new key appears only when it is explicitly overridden, and round-trips.
    let overridden = Limits::default().with_max_artifact_identity_work(4_096);
    let document = serde_json::to_string(&overridden).unwrap();
    assert_eq!(
        document,
        r#"{"version":1,"profile":"interactive_v1","overrides":{"max_artifact_identity_work":4096}}"#
    );
    assert_eq!(
        serde_json::from_str::<Limits>(&document).unwrap(),
        overridden
    );

    // The one-directional consequence, stated as a test: `deny_unknown_fields`
    // means an older build rejects this document rather than applying a policy
    // it cannot honour. The same rejection is what this build gives an unknown
    // key from a future one.
    let from_the_future =
        r#"{"version":1,"profile":"interactive_v1","overrides":{"max_future_work":1}}"#;
    let error = serde_json::from_str::<Limits>(from_the_future).unwrap_err();
    assert!(
        error.to_string().contains("max_future_work"),
        "an unknown override must be rejected by name: {error}"
    );

    // And the schema version did not move: adding a limit is not a schema break.
    assert_eq!(candid_core::LIMITS_CONFIG_VERSION, 1);
}
