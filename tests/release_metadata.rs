//! Issue #81's protocol, applied to the `0.1.0-beta.3` bump: what a version
//! bump is allowed to move.
//!
//! Bumping the crate version changes exactly one observable thing in the data
//! model: `ProducerInfo::current().version`, and therefore the octets of every
//! document newly serialized by this build. It must change nothing semantic —
//! not a `contract_id`, not an `interface_id`, not a `source_bundle_id`, and not
//! one byte of the frozen exact-octet artifact vectors.
//!
//! The two halves are both load-bearing. A suite that only pinned the new
//! version would pass just as happily if the version had been quietly bound
//! into the identity payload; a suite that only re-checked the identities would
//! pass if the bump had never happened. So each test below asserts the change
//! and the non-change together.
//!
//! Every anchor here is *reused*, never regenerated: the identity pins come out
//! of `tests/fixtures/conformance/expected/*.expected.json` and
//! `tests/fixtures/artifact-identity/manifest.json`, both of which this change
//! leaves untouched. Recomputing an expected value from the implementation
//! under test would make the whole file circular.

use candid_core::{
    artifact_id_with_limits, ArtifactKind, Contract, Limits, ProducerInfo, RawContract,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The prerelease this repository is preparing, spelled out rather than read
/// from the environment. `model_public_api.rs` already proves
/// `ProducerInfo::current().version == env!("CARGO_PKG_VERSION")`; the point of
/// a literal here is that changing the crate version has to be a deliberate
/// edit to a test that says why, not a silent consequence of editing
/// `Cargo.toml`.
const PRERELEASE_VERSION: &str = "0.1.0-beta.3";

/// The producer version frozen into `tests/fixtures/artifact-identity/`. Those
/// vectors are exact-octet protocol evidence, not current build output, so this
/// deliberately trails `PRERELEASE_VERSION` and must keep doing so.
const FROZEN_ARTIFACT_PRODUCER_VERSION: &str = "0.1.0";

/// Every producer version any frozen vector is allowed to carry. `9.9.9` is the
/// deliberately rewritten producer in the `contract_producer` and
/// `compilation_producer` vectors, which exist to show that rewriting producer
/// moves the artifact ID and leaves `contract_id` alone. Neither value is this
/// build's, and that is the invariant.
const FROZEN_PRODUCER_VERSIONS: [&str; 2] = [FROZEN_ARTIFACT_PRODUCER_VERSION, "9.9.9"];

fn repo(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn json(relative: &str) -> Value {
    serde_json::from_slice(&std::fs::read(repo(relative)).unwrap())
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

fn text(relative: &str) -> String {
    std::fs::read_to_string(repo(relative)).unwrap()
}

/// The five legacy wire fixtures, which `conformance_vectors.rs` and
/// `adr_conformance.rs` compare against *current compiler output*.
const WIRE_FIXTURES: [&str; 5] = ["actorless", "basic", "class", "empty_actor", "recursive"];

/// The ten frozen artifact vectors, whose IDs are digests of exact bytes.
const ARTIFACT_VECTORS: [&str; 10] = [
    "empty",
    "contract",
    "contract_producer",
    "contract_envelope",
    "contract_envelope_extension_name",
    "contract_envelope_extension_value",
    "contract_envelope_compact",
    "compilation",
    "compilation_source_doc",
    "compilation_producer",
];

// ---------------------------------------------------------------------------
// The change: producer metadata reports the prerelease.
// ---------------------------------------------------------------------------

#[test]
fn producer_reports_the_prerelease_version_in_every_configuration() {
    let producer = ProducerInfo::current();
    assert_eq!(producer.name, "candid-core");
    assert_eq!(producer.version, PRERELEASE_VERSION);
    assert_eq!(producer.version, env!("CARGO_PKG_VERSION"));

    // A prerelease, not a release. `"0.1"` does not select this version, which
    // is why README and docs/releasing.md require `=0.1.0-beta.3` by name.
    let (release, prerelease) = producer
        .version
        .split_once('-')
        .expect("the beta version must carry a prerelease suffix");
    assert_eq!(release, "0.1.0");
    assert_eq!(prerelease, "beta.3");

    // The engine pins are read from the manifest, not from a linked crate, so
    // they are identical here whether or not `compiler` is on. Neither moved.
    assert_eq!(producer.candid_version, "0.10.30");
    assert_eq!(producer.candid_parser_version, "0.4.0");
}

#[test]
fn the_legacy_wire_fixtures_carry_the_current_producer_version() {
    // These five are compared against what this build compiles today, so they
    // are the fixtures the version bump *must* touch. If one is left behind,
    // `legacy_wire_fixtures_stay_exact_compatibility_anchors` fails; this test
    // says why, and names the boundary.
    for name in WIRE_FIXTURES {
        let fixture = json(&format!("tests/fixtures/conformance/{name}.contract.json"));
        assert_eq!(
            fixture["producer"]["version"], PRERELEASE_VERSION,
            "{name}.contract.json is current compiler output and must carry the current version"
        );
    }
}

// ---------------------------------------------------------------------------
// The non-change: no semantic identity moved.
// ---------------------------------------------------------------------------

#[test]
fn the_pinned_semantic_identities_are_unchanged_by_the_version_bump() {
    // Reuse the pins as they stand. `expected/*.expected.json` records the
    // canonical payload text, its hex, the domain preimage, and the resulting
    // IDs, and none of those files is touched by this change.
    for name in WIRE_FIXTURES {
        let expected = json(&format!(
            "tests/fixtures/conformance/expected/{name}.expected.json"
        ));
        let wire: RawContract = serde_json::from_str(&text(&format!(
            "tests/fixtures/conformance/{name}.contract.json"
        )))
        .unwrap();

        assert_eq!(
            wire.identities.contract,
            expected["contract_identity"]["id"].as_str().unwrap(),
            "{name}: contract_id moved"
        );
        assert_eq!(
            wire.identities.interface.as_deref(),
            expected["interface_identity"]
                .as_object()
                .map(|identity| identity["id"].as_str().unwrap()),
            "{name}: interface_id moved"
        );

        // The identity payload is the thing that must not have grown a
        // producer. Checking the pinned JCS text directly is stronger than
        // comparing IDs, because it fails with a readable diff rather than a
        // changed digest.
        let jcs = expected["contract_identity"]["jcs"].as_str().unwrap();
        assert!(
            !jcs.contains("producer"),
            "{name}: the pinned identity payload must never mention producer"
        );
        assert!(
            !jcs.contains(PRERELEASE_VERSION) && !jcs.contains(FROZEN_ARTIFACT_PRODUCER_VERSION),
            "{name}: no crate version may appear in the identity payload"
        );
    }
}

#[test]
fn rewriting_only_the_producer_version_moves_the_octets_and_no_identity() {
    // The compatibility boundary in one place: take a checked-in Contract,
    // change nothing but `producer.version`, and observe that the semantic
    // identities are byte-identical while the exact-octet artifact ID is not.
    let original: RawContract =
        serde_json::from_str(&text("tests/fixtures/conformance/basic.contract.json")).unwrap();
    let mut rewritten = original.clone();
    rewritten.producer.version = format!("{PRERELEASE_VERSION}+rewritten");
    assert_ne!(original.producer, rewritten.producer);

    let original = Contract::try_from_raw(original).unwrap();
    let rewritten = Contract::try_from_raw(rewritten).unwrap();

    assert_eq!(original.contract_id(), rewritten.contract_id());
    assert_eq!(original.interface_id(), rewritten.interface_id());
    assert_ne!(original.producer(), rewritten.producer());

    let original_bytes = original.to_json_pretty().unwrap();
    let rewritten_bytes = rewritten.to_json_pretty().unwrap();
    assert_ne!(
        original_bytes, rewritten_bytes,
        "producer is part of the canonical wire bytes"
    );
    assert_ne!(
        artifact_id(&original_bytes),
        artifact_id(&rewritten_bytes),
        "an exact-octet artifact ID must move when producer does"
    );
}

/// Walk a decoded document and collect the `version` of every embedded
/// `ProducerInfo`, identified by its exact four-key shape rather than by
/// position, so a producer at any depth is found.
fn collect_producer_versions(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let is_producer = map.len() == 4
                && ["name", "version", "candid_version", "candid_parser_version"]
                    .iter()
                    .all(|key| map.get(*key).is_some_and(Value::is_string));
            if is_producer {
                found.push(map["version"].as_str().unwrap().to_string());
            }
            for nested in map.values() {
                collect_producer_versions(nested, found);
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_producer_versions(item, found)),
        _ => {}
    }
}

fn artifact_id(document: &str) -> String {
    artifact_id_with_limits(
        ArtifactKind::ContractJsonV1,
        document.as_bytes(),
        &Limits::default(),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// The frozen vectors: not current output, and not to be regenerated.
// ---------------------------------------------------------------------------

#[test]
fn the_exact_octet_artifact_vectors_were_not_regenerated() {
    let manifest = json("tests/fixtures/artifact-identity/manifest.json");
    let vectors = manifest["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), ARTIFACT_VECTORS.len());

    let mut with_producer = 0;
    for vector in vectors {
        let name = vector["name"].as_str().unwrap();
        assert!(
            ARTIFACT_VECTORS.contains(&name),
            "unexpected artifact vector {name}"
        );

        let relative = format!(
            "tests/fixtures/artifact-identity/{}",
            vector["file"].as_str().unwrap()
        );
        let bytes = std::fs::read(repo(&relative)).unwrap();

        // Reuse the manifest's pinned ID. Recomputing it from the bytes is the
        // whole check: if a well-meaning regeneration had rewritten
        // `producer.version` in these documents, the digest would move and this
        // fails, which is exactly the accident this test exists to catch.
        let kind = match vector["kind"].as_str().unwrap() {
            "ContractJsonV1" => ArtifactKind::ContractJsonV1,
            "ContractEnvelopeJsonV1" => ArtifactKind::ContractEnvelopeJsonV1,
            "CompilationJsonV1" => ArtifactKind::CompilationJsonV1,
            other => panic!("unknown artifact kind {other}"),
        };
        assert_eq!(
            artifact_id_with_limits(kind, &bytes, &Limits::default()).unwrap(),
            vector["artifact_id"].as_str().unwrap(),
            "{name}: the frozen vector no longer hashes to its pinned artifact ID"
        );

        // Every producer object embedded anywhere in the document must still
        // carry a frozen version, never this build's. Found structurally rather
        // than by substring, so a producer nested under `contract` in the
        // envelope and compilation kinds is checked the same way as a top-level
        // one in a bare Contract.
        let mut versions = Vec::new();
        collect_producer_versions(
            &serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            &mut versions,
        );
        if !versions.is_empty() {
            with_producer += 1;
        }
        for version in versions {
            assert!(
                FROZEN_PRODUCER_VERSIONS.contains(&version.as_str()),
                "{name}: producer version {version} is not one of the frozen values \
                 {FROZEN_PRODUCER_VERSIONS:?}; an exact-octet vector must never be \
                 regenerated with the current build's version"
            );
            assert_ne!(
                version, PRERELEASE_VERSION,
                "{name}: an exact-octet vector was regenerated with the current version"
            );
        }
    }
    assert_eq!(
        with_producer, 9,
        "nine of the ten vectors are documents carrying producer metadata; \
         only the empty framing anchor has none"
    );
}

#[test]
fn one_semantic_contract_id_still_spans_every_frozen_artifact() {
    // The claim issue #25 shipped and issue #81 must not disturb: nine distinct
    // artifact IDs over nine distinct byte sequences, all embedding one shared
    // `contract_id`. Two of those nine differ from another only in producer.
    let manifest = json("tests/fixtures/artifact-identity/manifest.json");
    let shared = &manifest["shared_contract_id"];
    let expected_id = shared["contract_id"].as_str().unwrap();
    let paths = &shared["contract_id_path"];

    let mut artifact_ids = std::collections::BTreeSet::new();
    for member in shared["members"].as_array().unwrap() {
        let member = member.as_str().unwrap();
        let vector = manifest["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vector| vector["name"] == member)
            .unwrap_or_else(|| panic!("{member} is not a manifest vector"));

        let document = json(&format!(
            "tests/fixtures/artifact-identity/{}",
            vector["file"].as_str().unwrap()
        ));
        let mut cursor = &document;
        for key in paths[vector["kind"].as_str().unwrap()].as_array().unwrap() {
            cursor = &cursor[key.as_str().unwrap()];
        }
        assert_eq!(cursor.as_str().unwrap(), expected_id, "{member}");
        assert!(
            artifact_ids.insert(vector["artifact_id"].as_str().unwrap()),
            "{member} shares an artifact ID with another vector"
        );
    }
    assert_eq!(artifact_ids.len(), 9);
}

// ---------------------------------------------------------------------------
// The compiler surface: source identity is untouched too.
// ---------------------------------------------------------------------------

#[cfg(feature = "filesystem-compiler")]
#[test]
fn compiling_the_conformance_sources_reproduces_the_pinned_identities() {
    // The end-to-end statement: this build, with the bumped version, still
    // produces the identities pinned before the bump — and a `source_bundle_id`
    // that never saw the producer at all.
    for name in WIRE_FIXTURES {
        let expected = json(&format!(
            "tests/fixtures/conformance/expected/{name}.expected.json"
        ));
        let compilation =
            candid_core::compile_did_file(repo(&format!("tests/fixtures/conformance/{name}.did")))
                .unwrap_or_else(|error| panic!("{name}: {error:#?}"));
        let contract = compilation.contract();

        assert_eq!(
            contract.producer().version,
            PRERELEASE_VERSION,
            "{name}: freshly compiled output must carry the current version"
        );
        assert_eq!(
            contract.contract_id(),
            expected["contract_identity"]["id"].as_str().unwrap(),
            "{name}: contract_id moved with the version bump"
        );
        assert_eq!(
            contract.interface_id(),
            expected["interface_identity"]
                .as_object()
                .map(|identity| identity["id"].as_str().unwrap()),
            "{name}: interface_id moved with the version bump"
        );

        let source_info = compilation.source_info().expect("provenance sidecar");
        let bundle_id = source_info.source_bundle_id();
        assert!(bundle_id.starts_with("candid-core:source-bundle:v1:sha256:"));
        assert!(
            !serde_json::to_string(source_info)
                .unwrap()
                .contains(PRERELEASE_VERSION),
            "{name}: the provenance sidecar must not carry a crate version"
        );
    }
}
