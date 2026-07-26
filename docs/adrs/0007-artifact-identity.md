# ADR 0007: Give artifacts whose exact octets must be committed to a detached identity

- Status: Implemented, verification pending
- Date: 2026-07-26
- Owners: Contract runtime maintainers

## Context

ADR 0001 established three domain-separated content identities over canonical
bytes — `contract_id` and `interface_id`, which are **semantic Contract
identities**, and `source_bundle_id`, which is a **raw-source bundle content
identity** covering raw source bytes and import edges — and, in its original
wording, recommended `contract_id` for "registries, persisted references,
signatures, and extension envelopes". Those four uses are not the same claim,
and `contract_id` supports only one of them.

`contract_id` hashes the canonical Contract payload and deliberately excludes
`producer` (`src/canonical.rs`), so rewriting producer metadata leaves it
untouched. It is computed over the Contract alone, so an envelope's extension
map and a compilation's `SourceInfo` sidecar are outside it as well. Canonical
bytes are not the only encoding of a Contract, so a re-indented or
differently-encoded file carries the same `contract_id` as the original. A
registry keyed on `contract_id`, a signature over `contract_id`, or a persisted
reference by `contract_id` therefore commits to *strictly less* than the file
the user believes they published: the same ID can accompany a different
producer, different extensions, different provenance, and different bytes.

Wording elsewhere compounded the problem by describing those identities as
"authenticated", which reads as a claim about who produced the artifact. No
unkeyed content ID authenticates itself; they are content addresses over
different projections, and a signature or other external mechanism is what
authenticates one.

## Decision

Add a fourth identity that is explicitly **not** semantic: a detached,
exact-octet artifact identity, and state the equality and security claim of all
four in one place.

For a frozen per-kind domain `D` and the exact artifact octets `B`:

```text
preimage    = <D as UTF-8 bytes> || 0x00 || B
artifact_id = D + ":sha256:" + lowercase_hex(SHA-256(preimage))
```

```text
candid-core:artifact:contract-json:v1:sha256:<lowercase hex>
candid-core:artifact:contract-envelope-json:v1:sha256:<lowercase hex>
candid-core:artifact:compilation-json:v1:sha256:<lowercase hex>
```

Five properties are load-bearing:

1. **Detached.** The identity is returned to the caller and never serialized
   inside the artifact it hashes. No existing serialized artifact gains an
   `artifact_id` field, and no bounded parse computes one implicitly, so
   validation order and error precedence are unchanged.
2. **Exact octets.** `B` is hashed verbatim — never parsed, canonicalized,
   normalized, or re-encoded. Reformatting, whitespace, key order, numeric
   spelling, a rewritten producer, an extension edit, and a `SourceInfo` edit
   change the ID precisely when they change the bytes.
3. **Kind-separated.** The domain names the artifact kind and is
   self-describing, so identical bytes under two kinds are two identities.
   There is no separate kind label and no length field in the preimage. A
   Contract document therefore has its own identity distinct from the identity
   of an envelope or a compilation that embeds the very same Contract.
4. **Bounded like everything else.** `max_input_bytes` is enforced against the
   slice before any hashing; the hash then charges its own
   `max_artifact_identity_work` counter, one unit per artifact byte plus the
   fixed domain framing, and consumes no other counter.
5. **Narrow.** No default-limits convenience function, no verification helper,
   no stored field, no new `Deserialize` surface, no `source_info_id`, no
   combined package schema.

The pre-existing identities are untouched. The semantic Contract identities
`contract_id` and `interface_id`, and the raw-source bundle content identity
`source_bundle_id`, keep their existing payloads, domains, framing, and
interpretation; `contract_id` continues to exclude `ProducerInfo`;
`source_bundle_id` continues to identify only raw sources and import edges
rather than a complete `SourceInfo`.

ADR 0001's recommendation is corrected rather than extended: `contract_id`
identifies the semantic Contract, and a registry, persisted reference, or
signature that must commit to a *file* commits to its `artifact_id` instead —
alongside `contract_id` when it also wants the semantic claim.

## Non-goals

Deliberately out of scope, and not deferred work this ADR promises: any signer
model, key format, signature algorithm, trust policy, credential handling,
registry protocol, or network behaviour; canonical JSON or JCS for arbitrary
extension values; a `source_info_id`; a combined artifact envelope or package
schema; a persisted `artifact_id` field; and a verification convenience API.
This crate computes a digest over bytes a caller supplies and stops there.

## Consequences

- A caller whose claim is about exact octets — because those octets are what is
  being committed to — has an identity that actually covers them, and one that
  changes when the file changes.
- An artifact ID authenticates nothing by itself, as no unkeyed content ID does.
  It is a content address: it makes no validity, authenticity, provenance, or
  trust claim, and callers must validate the artifact separately through the
  bounded parse entry point for its kind.
- Semantic equality and artifact equality now visibly disagree, which is the
  point. One `contract_id` spans many artifact IDs, and consumers must pick the
  identifier whose claim matches their use — the same rule ADR 0001 already
  stated for `interface_id` versus `contract_id`.
- Coverage is exactly the bytes passed to the call — persisting them first is a
  use case, not a prerequisite — so what travels with them depends on the kind
  named. `ArtifactKind` selects a domain and neither parses nor validates, so
  the following describes a *valid serialized document of the declared kind*: a
  `Contract` document contains the Contract alone, `producer` included and no
  extensions or `SourceInfo`; a `ContractEnvelope` document contains extensions
  and no `SourceInfo`; a `Compilation` document contains a `SourceInfo` sidecar
  and no extensions. Package or application version is covered only when it is
  literally present in the supplied artifact bytes. Arbitrary bytes hash just as
  well under any kind, and the ID makes no validity claim about them.
- Producer coverage therefore depends on the artifact kind rather than on the
  identity family: raw Contract JSON binds `ProducerInfo` bytes because those
  bytes are in the exact artifact, while `contract_id` and `interface_id` still
  exclude producer metadata entirely.
- `Limits` gains one additive override key. Documents that do not set it are
  unchanged in both directions; a document that does set it is rejected, not
  ignored, by a build that predates the key. `LIMITS_CONFIG_VERSION` is
  unchanged, because no existing key, value, or default moved.

## Implementation

`src/artifact_id.rs` is base surface: `artifact_id_with_limits`,
`artifact_id_with_context`, and a `#[non_exhaustive]` `ArtifactKind` whose three
frozen variants are `ContractJsonV1`, `ContractEnvelopeJsonV1`, and
`CompilationJsonV1`. The enum is `#[non_exhaustive]` so that naming a further
kind later is not a breaking change. Hashing bytes needs no Candid engine, so
`CompilationJsonV1` is reachable with defaults disabled even though
`Compilation` itself is `compiler` surface, and the base dependency graph is
unchanged — `sha2` and `hex` were already base dependencies.

The digest is computed over borrowed chunks of the caller's slice, so a large
artifact is never copied a second time and never becomes one uninterruptible
block; cancellation, deadlines, and work exhaustion are observed at every chunk
boundary and all fail closed rather than returning a partial digest.
`src/limits.rs` carries the dedicated `max_artifact_identity_work` limit, whose
default of `10_000_000` is proven against the default 4 MiB byte gate rather
than guessed.

The byte-level format is specified normatively in
[`docs/artifact-identity-v1.md`](../artifact-identity-v1.md), which also carries
the four-row table stating every identity's kind, covered data, equality claim,
and exclusions.

## Required verification

- Golden artifact IDs for every `ArtifactKind` variant, including a framing
  anchor whose whole preimage is pinned as hex, and a checked-in raw Contract
  JSON vector with its own pinned ID.
- A test that identical bytes under different kinds produce different IDs,
  covering `ContractJsonV1` against each other kind.
- Tests proving that one changed byte, whitespace, re-encoding, rewritten
  producer bytes, an extension name, an extension value, and a changed
  `SourceInfo` field each change `artifact_id`, and that none of them changes
  `contract_id` or `interface_id` where the semantic Contract is unchanged —
  including a `ProducerInfo` rewrite inside raw Contract bytes, where the
  producer octets are part of the artifact and part of neither semantic
  identity.
- Resource tests: `max_input_bytes` rejected before hashing and taking
  precedence over an exhausted work budget; the exact `artifact_identity_work`
  bound accepted and one unit below rejected with stable `{resource, limit,
  observed}` metadata; cancellation and elapsed deadlines failing closed between
  hashing chunks; and no consumption of `canonicalization_work` or
  `source_identity_work`.
- Public-API and feature-boundary tests proving the surface exists with default
  features disabled and adds nothing to the base dependency graph
  (`tests/fixtures/packaging/verify_feature_graph.py`).
- A `wasm32-unknown-unknown` build plus a browser runtime assertion that the
  digests are identical there, with a framing pin per kind
  (`tests/browser_wasm.rs`).
- Independent standard-library Python vectors in
  `tests/fixtures/artifact-identity/`, run by their own CI job, kept separate
  from the closed semantic conformance manifest.
- Evidence that every pre-existing identity anchor and every existing
  canonicalization vector is unchanged.
