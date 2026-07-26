//! Detached, exact-octet identity for a serialized artifact.
//!
//! The three identities this crate already computes are not one family:
//!
//! * `contract_id` and `interface_id` are **semantic Contract identities**.
//!   Each hashes a canonicalized projection of *meaning*, so inputs that mean
//!   the same thing collide on purpose.
//! * `source_bundle_id` is a **raw-source bundle content identity**. It does
//!   identify raw source-file content — the source bytes and their import edges
//!   are exactly what it covers — so reformatting or editing a comment inside a
//!   source *does* move it, while data *derived* from those sources never
//!   enters it.
//!
//! None of the three identifies a complete serialized [`crate::Contract`],
//! [`crate::ContractEnvelope`], or `Compilation` document. `source_bundle_id`
//! identifies the input bundle, not the document compiled from it, and a
//! Contract's `contract_id` is unchanged by rewriting its `producer`, by adding,
//! removing, or editing an envelope extension, by reformatting the document, and
//! by replacing its `SourceInfo` sidecar. That is what makes a semantic identity
//! useful as a cache key and a compatibility key, and exactly what makes it the
//! wrong thing to commit to when the octets themselves are what matter.
//!
//! [`artifact_id_with_limits`] closes that gap without touching any existing
//! identity. It hashes the **exact octet sequence** a caller hands it, under a
//! kind-specific domain, and returns the digest to the caller. Nothing is
//! stored, nothing is serialized back into the artifact, and no existing
//! payload, domain, or framing changes.
//!
//! # What an `artifact_id` claims
//!
//! Two artifact IDs are equal if and only if the [`ArtifactKind`] and the byte
//! sequence were equal, subject to the SHA-256 collision assumption. That is
//! the whole claim.
//!
//! An `artifact_id` therefore does **not** establish:
//!
//! * semantic equality — two byte-different encodings of the same Contract have
//!   different artifact IDs, which is the point;
//! * structural validity — the bytes are never parsed, so an ID exists for
//!   input that would fail validation;
//! * authenticity, integrity against a chosen signer, producer truth, or
//!   signature trust — it is a content address, not a credential.
//!
//! No unkeyed content ID authenticates itself, and this one is no exception. A
//! caller validates the artifact separately, through the bounded parse entry
//! point for its kind, and uses the detached ID as a content address or as the
//! value a signature or other external mechanism commits to — that mechanism,
//! not the digest, is what authenticates. This crate has no signer model, key
//! format, signature algorithm, trust policy, or registry protocol, and this
//! module does not introduce one.
//!
//! # Coverage is exactly the bytes passed
//!
//! Only the octets actually supplied are covered — the bytes passed to the
//! call, whether or not they have already been persisted anywhere — so what an
//! artifact ID binds depends on which kind the caller names.
//!
//! [`ArtifactKind`] selects a domain and does nothing else: it neither parses
//! nor validates. What follows therefore describes a *valid serialized document
//! of the declared kind*. A `Contract` document contains the Contract alone —
//! including its `producer`, which no semantic identity covers — and contains
//! neither extensions nor `SourceInfo`. A `ContractEnvelope` document contains a
//! Contract and its extensions and does not contain `SourceInfo`. A
//! `Compilation` document contains a Contract and an optional `SourceInfo`
//! sidecar and does not contain envelope extensions. Package or application
//! version is covered only when it is literally present in the bytes passed.
//!
//! Arbitrary bytes hash just as well under any kind, and the resulting ID makes
//! no claim that they are a document of that kind at all.
//!
//! # Construction
//!
//! ```text
//! preimage    = <domain UTF-8 bytes> || 0x00 || <exact artifact bytes>
//! artifact_id = <domain> ":sha256:" <64 lowercase hex digits of SHA-256(preimage)>
//! ```
//!
//! The domain is self-describing and frozen per kind, so a digest computed for
//! one kind can never be mistaken for another's even when the bytes are
//! identical. There is no separate kind label and no length field in the
//! preimage: the domain already names the kind, and the digest is taken over a
//! single contiguous byte run whose length the caller supplied.
//!
//! `docs/artifact-identity-v1.md` specifies this normatively.

use crate::budget::{Budget, BudgetError};
use crate::{ContractValidationError, Limits, RuntimeContext};
use sha2::{Digest, Sha256};

/// The kind of serialized artifact an identity is computed over.
///
/// The kind selects the frozen domain tag, and therefore separates the digest
/// space: identical bytes under two kinds produce two different IDs. Selecting
/// a domain is *all* it does — no variant parses or validates anything, so any
/// byte sequence hashes under any kind and the resulting ID makes no claim that
/// those bytes are a document of that kind. Each variant's coverage note below
/// therefore describes a valid serialized document of the declared kind.
///
/// `#[non_exhaustive]` for the same reason [`crate::LimitsProfile`] is: naming
/// a further artifact kind must not be a breaking change. The variants below
/// are frozen — a new kind becomes a new variant with a new domain, never a
/// redefinition of an existing one.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    /// A serialized [`crate::Contract`] document on its own: the strict
    /// Contract with no enclosing envelope and no provenance sidecar, as
    /// produced by [`crate::Contract::to_json_pretty_with_limits`] or by any
    /// other encoder the caller uses.
    ///
    /// Covers the Contract bytes only. `producer` *is* covered, because those
    /// bytes are in the document even though `contract_id` and `interface_id`
    /// deliberately exclude them. Envelope extensions and `SourceInfo` are not
    /// covered: a bare Contract document contains neither.
    ContractJsonV1,
    /// A serialized [`crate::ContractEnvelope`] document: a strict Contract
    /// plus its namespaced extension map, as produced by
    /// [`crate::ContractEnvelope::to_json_pretty_with_limits`] or by any other
    /// encoder the caller uses.
    ///
    /// Covers the envelope bytes only. Extensions *are* covered, because they
    /// are in those bytes and `contract_id` deliberately excludes them.
    /// `SourceInfo` is not covered: an envelope document does not contain one.
    ///
    /// This is not the same identity as [`Self::ContractJsonV1`] over the
    /// nested Contract: the two hash different byte sequences under different
    /// domains.
    ContractEnvelopeJsonV1,
    /// A serialized `Compilation` document: a Contract plus its optional
    /// `SourceInfo` provenance sidecar.
    ///
    /// Covers the compilation bytes only, including the sidecar when the
    /// document carries one and including `producer`, which `contract_id`
    /// deliberately excludes. Envelope extensions are not covered: a
    /// compilation document does not contain any.
    ///
    /// The variant is base surface even though `Compilation` itself is
    /// `compiler` surface — hashing bytes needs no Candid engine, so a base
    /// consumer can content-address a compilation document it was handed.
    CompilationJsonV1,
}

impl ArtifactKind {
    /// The frozen, self-describing domain tag for this kind.
    ///
    /// Kept private: it is already the literal prefix of every rendered ID, and
    /// `docs/artifact-identity-v1.md` states every value normatively, so
    /// exposing an accessor would widen the public surface without adding
    /// information.
    const fn domain(self) -> &'static str {
        match self {
            Self::ContractJsonV1 => "candid-core:artifact:contract-json:v1",
            Self::ContractEnvelopeJsonV1 => "candid-core:artifact:contract-envelope-json:v1",
            Self::CompilationJsonV1 => "candid-core:artifact:compilation-json:v1",
        }
    }
}

/// The resource name every artifact identity computation charges.
const RESOURCE: &str = "artifact_identity_work";

/// Bytes hashed between two budget observations.
///
/// The input is hashed as borrowed slices of this length, so a large artifact
/// never becomes one uninterruptible block and never becomes a second full
/// allocation. Cancellation, deadlines, and work exhaustion are all observed at
/// each boundary, because `Budget::charge` checkpoints before it charges.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Compute the detached artifact identity of `bytes` under caller-supplied
/// limits.
///
/// This is an explicit call, never implicit in a decode: no bounded parse
/// computes it, no serialized artifact gains a field for it, and existing
/// validation and error precedence are unchanged by its existence.
///
/// # Equality and security claim
///
/// Equal IDs mean equal `kind` and an equal octet sequence, under the SHA-256
/// collision assumption — nothing more. Reformatting, whitespace, JSON key
/// order, numeric spelling, a rewritten `producer`, an added or edited
/// extension, and a changed `SourceInfo` field all change the ID when they
/// change the bytes.
///
/// The ID authenticates nothing by itself — no unkeyed content ID does. It
/// establishes neither semantic equality, nor structural validity — the bytes
/// are never parsed, so an ID exists for input that would fail validation — nor
/// authenticity, producer truth, or signature trust. It is a content address,
/// not a credential: validate the artifact separately through the bounded parse
/// entry point for its kind, and use this ID as the value a signature or other
/// external mechanism commits to. This crate defines no signer model, key
/// format, signature algorithm, trust policy, or registry protocol.
///
/// Coverage is exactly the bytes passed to this call, whether or not they have
/// already been persisted anywhere, so what travels with them depends on `kind`.
/// `kind` selects a domain and neither parses nor validates, so the following
/// describes a *valid serialized document of the declared kind*: a `Contract`
/// document carries the Contract alone, `producer` included; a
/// `ContractEnvelope` document carries extensions and no `SourceInfo`; a
/// `Compilation` document carries a `SourceInfo` sidecar and no extensions;
/// package or application version is covered only when it is literally present
/// in those bytes. Arbitrary bytes hash just as well under any kind, and the ID
/// makes no validity claim about them. `docs/artifact-identity-v1.md` specifies
/// all of this normatively.
///
/// # Resources
///
/// 1. [`Limits::max_input_bytes`] is enforced against `bytes.len()` *before*
///    any hashing, reported as resource `input_bytes`.
/// 2. [`Limits::max_artifact_identity_work`] is then charged one unit per
///    artifact byte plus the fixed domain framing cost, reported as resource
///    `artifact_identity_work`. No other counter is consumed — in particular
///    not `canonicalization_work` or `source_identity_work`.
///
/// # Examples
///
/// ```
/// use candid_core::{artifact_id_with_limits, ArtifactKind, Limits};
///
/// let document = br#"{"contract":{}}"#;
/// let id = artifact_id_with_limits(
///     ArtifactKind::ContractEnvelopeJsonV1,
///     document,
///     &Limits::default(),
/// )?;
/// assert!(id.starts_with("candid-core:artifact:contract-envelope-json:v1:sha256:"));
///
/// // The same bytes under any other kind are a different identity.
/// for other in [ArtifactKind::ContractJsonV1, ArtifactKind::CompilationJsonV1] {
///     let rehashed = artifact_id_with_limits(other, document, &Limits::default())?;
///     assert_ne!(id, rehashed);
/// }
///
/// // One byte of whitespace is a different artifact.
/// let reformatted = artifact_id_with_limits(
///     ArtifactKind::ContractEnvelopeJsonV1,
///     br#"{"contract": {}}"#,
///     &Limits::default(),
/// )?;
/// assert_ne!(id, reformatted);
/// # Ok::<(), candid_core::ContractValidationError>(())
/// ```
pub fn artifact_id_with_limits(
    kind: ArtifactKind,
    bytes: &[u8],
    limits: &Limits,
) -> Result<String, ContractValidationError> {
    artifact_id_with_context(kind, bytes, &RuntimeContext::new(limits.clone()))
}

/// [`artifact_id_with_limits`] under a full [`RuntimeContext`], so a caller's
/// [`CancellationToken`](crate::CancellationToken) and deadline are observed
/// while hashing.
///
/// Cancellation and an elapsed deadline are checked before the first byte is
/// hashed and again at every chunk boundary, and both fail closed with
/// `operation_cancelled` / `operation_deadline_exceeded` rather than returning
/// a partial digest.
pub fn artifact_id_with_context(
    kind: ArtifactKind,
    bytes: &[u8],
    context: &RuntimeContext,
) -> Result<String, ContractValidationError> {
    let mut budget = context.budget();
    artifact_id_with_budget(kind, bytes, &mut budget)
}

fn artifact_id_with_budget(
    kind: ArtifactKind,
    bytes: &[u8],
    budget: &mut Budget<'_>,
) -> Result<String, ContractValidationError> {
    // The byte gate first, exactly as every bounded parse entry point does it,
    // so an oversized artifact is rejected on `input_bytes` before any hashing
    // work is charged or performed.
    crate::budget::observe_input_bytes(budget, bytes.len())?;

    let domain = kind.domain();
    let limit = budget.limits().max_artifact_identity_work;
    let mut hasher = Sha256::new();

    // The fixed framing cost: the domain tag plus its one-byte separator. The
    // preimage carries no length field and no second kind label, so this is the
    // whole constant overhead.
    charge(budget, limit, domain.len().saturating_add(1))?;
    hasher.update(domain.as_bytes());
    hasher.update([0]);

    // `chunks` borrows into the caller's slice, so hashing a large artifact
    // never allocates a second copy of it.
    for chunk in bytes.chunks(HASH_CHUNK_BYTES) {
        charge(budget, limit, chunk.len())?;
        hasher.update(chunk);
        observe_chunk_boundary();
    }

    budget
        .checkpoint()
        .map_err(BudgetError::into_contract_error)?;
    Ok(format!(
        "{domain}:sha256:{}",
        hex::encode(hasher.finalize())
    ))
}

fn charge(
    budget: &mut Budget<'_>,
    limit: usize,
    amount: usize,
) -> Result<(), ContractValidationError> {
    budget
        .charge(RESOURCE, limit, amount)
        .map(|_| ())
        .map_err(BudgetError::into_contract_error)
}

// Test-only observation point at a hashed-chunk boundary.
//
// The chunk loop exists so cancellation and deadlines are observed *between*
// chunks rather than only before the first one. Proving that deterministically
// requires cancellation to flip while the loop is running, which no public API
// can do from the calling thread; this seam does it, fires once, and is
// compiled only for this crate's own unit tests. Production keeps an empty
// function, so the loop has exactly the shape it is documented to have.
#[cfg(test)]
thread_local! {
    static CANCEL_AT_CHUNK_BOUNDARY: std::cell::Cell<Option<crate::CancellationToken>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn observe_chunk_boundary() {
    CANCEL_AT_CHUNK_BOUNDARY.with(|slot| {
        if let Some(token) = slot.take() {
            token.cancel();
        }
    });
}

#[cfg(not(test))]
fn observe_chunk_boundary() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CancellationToken;

    /// Every frozen kind. `ArtifactKind::domain` matches exhaustively, so a new
    /// variant cannot compile without declaring its domain; this list is what
    /// the loops below iterate, and a new variant belongs in it too.
    const ALL_KINDS: &[ArtifactKind] = &[
        ArtifactKind::ContractJsonV1,
        ArtifactKind::ContractEnvelopeJsonV1,
        ArtifactKind::CompilationJsonV1,
    ];

    /// Exact `artifact_identity_work` cost of hashing `len` bytes under `kind`.
    fn exact_work(kind: ArtifactKind, len: usize) -> usize {
        kind.domain().len() + 1 + len
    }

    fn id(kind: ArtifactKind, bytes: &[u8]) -> String {
        artifact_id_with_limits(kind, bytes, &Limits::default()).unwrap()
    }

    fn resource_failure(error: &ContractValidationError) -> (String, u64, u64) {
        let violation = &error.violations[0];
        assert_eq!(violation.code, "resource_limit_exceeded", "{error:#?}");
        let info = violation.resource_limit.as_ref().unwrap();
        (info.resource.clone(), info.limit, info.observed)
    }

    /// The framing anchor: hashing an empty artifact pins the domain tag and
    /// the single separator byte, with no artifact bytes to hide a mistake in.
    /// The same two literals are pinned independently by
    /// `tests/fixtures/artifact-identity/manifest.json` and its Python verifier.
    #[test]
    fn empty_input_pins_the_domain_framing() {
        assert_eq!(
            id(ArtifactKind::ContractJsonV1, b""),
            "candid-core:artifact:contract-json:v1:sha256:66c1371d29c896c2b292edc5dc1d344bf39103c5a1011141ed6883ace3e95401"
        );
        assert_eq!(
            id(ArtifactKind::ContractEnvelopeJsonV1, b""),
            "candid-core:artifact:contract-envelope-json:v1:sha256:1642aac2ca520b95cc0c31068934081c206f8673bb3779058bb88e331ff21603"
        );
        assert_eq!(
            id(ArtifactKind::CompilationJsonV1, b""),
            "candid-core:artifact:compilation-json:v1:sha256:6e716227d7ae7ac930966faafa9812eeac2fa34a85c1f03b91d949ca88b21807"
        );
    }

    /// The domains are frozen literals, so they are pinned here as well as in
    /// the rendered golden IDs above: a typo that moved a domain would move
    /// every ID under it at once, and this states the intended strings directly.
    #[test]
    fn every_domain_is_frozen_distinct_and_well_formed() {
        assert_eq!(
            ALL_KINDS
                .iter()
                .map(|kind| kind.domain())
                .collect::<Vec<_>>(),
            [
                "candid-core:artifact:contract-json:v1",
                "candid-core:artifact:contract-envelope-json:v1",
                "candid-core:artifact:compilation-json:v1",
            ]
        );
        for (position, kind) in ALL_KINDS.iter().enumerate() {
            let domain = kind.domain();
            // ASCII with no NUL is what makes the single 0x00 separator
            // unambiguous, and the prefix keeps the namespace self-describing.
            assert!(domain.is_ascii() && !domain.contains('\0'), "{domain}");
            assert!(domain.starts_with("candid-core:artifact:"), "{domain}");
            for other in &ALL_KINDS[position + 1..] {
                assert_ne!(domain, other.domain());
            }
        }
    }

    #[test]
    fn the_preimage_is_the_domain_a_nul_byte_and_the_exact_bytes() {
        for &kind in ALL_KINDS {
            let bytes = b"{\"contract\":{}}";
            let mut preimage = kind.domain().as_bytes().to_vec();
            preimage.push(0);
            preimage.extend_from_slice(bytes);
            assert_eq!(
                id(kind, bytes),
                format!(
                    "{}:sha256:{}",
                    kind.domain(),
                    hex::encode(Sha256::digest(&preimage))
                )
            );
        }
    }

    #[test]
    fn identical_bytes_under_different_kinds_differ() {
        let bytes = b"{}";
        let rendered: Vec<String> = ALL_KINDS.iter().map(|&kind| id(kind, bytes)).collect();
        for (position, one) in rendered.iter().enumerate() {
            for other in &rendered[position + 1..] {
                assert_ne!(one, other, "the domain must separate the digest space");
            }
        }
    }

    #[test]
    fn input_bytes_is_enforced_before_any_hashing_work() {
        let limits = Limits::default()
            .with_max_input_bytes(3)
            // Zero work would fail the framing charge immediately, so a result
            // that reports `input_bytes` proves the byte gate ran first.
            .with_max_artifact_identity_work(0);
        let error =
            artifact_id_with_limits(ArtifactKind::CompilationJsonV1, b"abcd", &limits).unwrap_err();
        assert_eq!(
            resource_failure(&error),
            ("input_bytes".to_string(), 3, 4),
            "the byte gate must precede identity work"
        );
    }

    #[test]
    fn work_succeeds_at_the_exact_limit_and_fails_one_unit_below() {
        let kind = ArtifactKind::ContractEnvelopeJsonV1;
        let bytes = b"{\"contract\":{},\"extensions\":{}}";
        let work = exact_work(kind, bytes.len());

        artifact_id_with_limits(
            kind,
            bytes,
            &Limits::default().with_max_artifact_identity_work(work),
        )
        .expect("the exact work bound must succeed");

        let error = artifact_id_with_limits(
            kind,
            bytes,
            &Limits::default().with_max_artifact_identity_work(work - 1),
        )
        .unwrap_err();
        assert_eq!(
            resource_failure(&error),
            (RESOURCE.to_string(), (work - 1) as u64, work as u64)
        );
    }

    /// Neither of the two pre-existing identity counters may be touched: an
    /// artifact identity that spent `canonicalization_work` would let a caller
    /// starve Contract canonicalization by content-addressing a document, and
    /// one that spent `source_identity_work` would do the same to provenance.
    #[test]
    fn no_other_identity_counter_is_consumed() {
        let limits = Limits::default();
        let context = RuntimeContext::new(limits);
        let mut budget = context.budget();
        artifact_id_with_budget(
            ArtifactKind::CompilationJsonV1,
            &vec![b'x'; HASH_CHUNK_BYTES * 2 + 7],
            &mut budget,
        )
        .unwrap();
        assert_eq!(budget.consumed("canonicalization_work"), 0);
        assert_eq!(budget.consumed("source_identity_work"), 0);
        assert_eq!(
            budget.consumed(RESOURCE),
            exact_work(ArtifactKind::CompilationJsonV1, HASH_CHUNK_BYTES * 2 + 7)
        );
    }

    /// Work is charged per chunk, not as one lump sum: an artifact spanning
    /// several chunks with a budget that runs out mid-way stops at a chunk
    /// boundary, so the reported `observed` lands strictly inside the range.
    #[test]
    fn work_is_charged_incrementally_across_chunks() {
        let kind = ArtifactKind::CompilationJsonV1;
        let bytes = vec![b'x'; HASH_CHUNK_BYTES * 3];
        let framing = kind.domain().len() + 1;
        let limit = framing + HASH_CHUNK_BYTES + 1;

        let error = artifact_id_with_limits(
            kind,
            &bytes,
            &Limits::default().with_max_artifact_identity_work(limit),
        )
        .unwrap_err();
        let (resource, reported_limit, observed) = resource_failure(&error);
        assert_eq!(resource, RESOURCE);
        assert_eq!(reported_limit, limit as u64);
        assert!(
            observed > framing as u64 && observed < exact_work(kind, bytes.len()) as u64,
            "a chunked charge must fail part-way, not at the total: {observed}"
        );
    }

    #[test]
    fn a_cancelled_token_fails_closed_before_hashing() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = RuntimeContext::new(Limits::default()).with_cancellation(cancellation);
        let error =
            artifact_id_with_context(ArtifactKind::CompilationJsonV1, b"{}", &context).unwrap_err();
        assert_eq!(error.violations[0].code, "operation_cancelled");
    }

    /// The between-chunks claim, proved directly: cancellation flips at the
    /// first chunk boundary and the second chunk is never charged.
    #[test]
    fn cancellation_is_observed_between_chunks() {
        let kind = ArtifactKind::ContractEnvelopeJsonV1;
        let cancellation = CancellationToken::new();
        let context =
            RuntimeContext::new(Limits::default()).with_cancellation(cancellation.clone());
        let mut budget = context.budget();
        CANCEL_AT_CHUNK_BOUNDARY.with(|slot| slot.set(Some(cancellation)));

        let error = artifact_id_with_budget(kind, &vec![b'x'; HASH_CHUNK_BYTES * 3], &mut budget)
            .unwrap_err();
        assert_eq!(error.violations[0].code, "operation_cancelled");
        assert_eq!(
            budget.consumed(RESOURCE),
            kind.domain().len() + 1 + HASH_CHUNK_BYTES,
            "exactly one chunk may be charged before cancellation is observed"
        );
    }

    /// Native only: bare `wasm32-unknown-unknown` has no clock, and there every
    /// explicit deadline is already reported as elapsed by `Deadline::snapshot`.
    #[cfg(not(target_os = "unknown"))]
    #[test]
    fn an_elapsed_deadline_fails_closed() {
        let context = RuntimeContext::new(Limits::default().with_deadline_unix_ms(Some(1)));
        let error = artifact_id_with_context(
            ArtifactKind::CompilationJsonV1,
            &vec![b'x'; HASH_CHUNK_BYTES * 2],
            &context,
        )
        .unwrap_err();
        assert_eq!(error.violations[0].code, "operation_deadline_exceeded");
    }

    /// The default work limit must cover the largest artifact the default byte
    /// gate admits, for *every* kind — a new kind with a longer domain must not
    /// quietly make a maximum-size artifact unhashable out of the box. The
    /// longest domain is asserted explicitly so the worst case is named rather
    /// than merely satisfied.
    #[test]
    fn the_default_work_limit_covers_the_default_byte_gate() {
        let limits = Limits::default();
        let longest = ALL_KINDS
            .iter()
            .map(|kind| kind.domain().len())
            .max()
            .expect("at least one kind exists");
        assert_eq!(
            longest,
            ArtifactKind::ContractEnvelopeJsonV1.domain().len(),
            "the documented worst-case domain must still be the longest"
        );
        for &kind in ALL_KINDS {
            assert!(
                exact_work(kind, limits.max_input_bytes()) <= limits.max_artifact_identity_work(),
                "{kind:?} at the byte gate must fit the default work limit"
            );
        }
    }
}
