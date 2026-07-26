# Artifact identity v1 (normative)

This document specifies `candid-core:artifact:*` — a **detached, exact-octet**
identity for a declared artifact kind. It is deliberately separate from
[canonicalization v1](canonicalization-v1.md), which specifies the canonicalized
identities `candid-core:contract:v1` and `candid-core:interface:v1` (the
**semantic Contract identities**) and `candid-core:source-bundle:v1` (a
**raw-source bundle content identity** over raw source bytes and import edges).
Nothing in this document changes the bytes, payloads, domains, framing, or
interpretation of those three.

Artifact identity exists because none of those three identifies a complete
serialized `Contract`, `ContractEnvelope`, or `Compilation` artifact.
`source_bundle_id` does identify raw source-file content — the source bytes and
import edges are exactly what it covers — but that is the input bundle, not the
document compiled from it. And `contract_id` is unchanged by rewriting
`producer`, by adding, removing, or editing an envelope extension, by replacing
the `SourceInfo` sidecar, and by re-encoding the same document with different
whitespace. All of those are properties of a serialized document someone may
want to reference, publish, or commit to externally. This gives that document
its own identity — use it when exact octets are what must be committed to — and
gives it no other power.

## 1. Domains

Each artifact kind has one frozen, self-describing domain.

| `ArtifactKind` | Domain |
| --- | --- |
| `ContractJsonV1` | `candid-core:artifact:contract-json:v1` |
| `ContractEnvelopeJsonV1` | `candid-core:artifact:contract-envelope-json:v1` |
| `CompilationJsonV1` | `candid-core:artifact:compilation-json:v1` |

A domain is never redefined. A new artifact kind becomes a new variant with a
new domain; a change to what an existing kind covers would be a new `:v2`
domain, not an edit to `:v1`. `ArtifactKind` is `#[non_exhaustive]`, so naming a
further kind is additive rather than breaking.

## 2. Construction

For a kind whose domain is `D` and an artifact whose exact octet sequence is
`B`:

```text
preimage    = <D as UTF-8 bytes> || 0x00 || B
artifact_id = D + ":sha256:" + lowercase_hex(SHA-256(preimage))
```

Normative details:

- The digest is SHA-256 and the rendering is exactly 64 lowercase hexadecimal
  digits. Uppercase hex is not this format.
- `0x00` is the single separator byte. `D` is ASCII and contains no `0x00`, so
  the domain and the artifact bytes cannot be confused for one another.
- There is **no length field** and **no second kind label** in the preimage. The
  domain already names the kind, and the artifact is a single contiguous byte
  run whose length the caller supplied. Adding either would change every ID
  below.
- `B` is used verbatim. It is never parsed, re-encoded, canonicalized,
  normalized, trimmed, or re-indented before hashing. An empty `B` is
  well-defined: the preimage is then the domain plus the separator alone.

The reference implementation is `src/artifact_id.rs`
(`artifact_id_with_limits` / `artifact_id_with_context`). An independent
standard-library Python implementation of this section is
`tests/fixtures/artifact-identity/verify_artifact_ids.py`.

## 3. What an artifact ID claims

**Equality claim.** Two artifact IDs are equal if and only if the artifact kind
and the octet sequence were equal, under the SHA-256 collision assumption. That
is the entire claim.

**Security claim.** None on its own. No unkeyed content ID authenticates itself:
an artifact ID is a content address, not a credential. It does not establish:

- *semantic equality* — two byte-different encodings of the same Contract have
  different artifact IDs, deliberately;
- *structural validity* — the bytes are never parsed, so an artifact ID exists
  for input that would fail validation, including input that is not JSON at all;
- *authenticity, producer truth, or signature trust* — it says nothing about who
  produced the bytes or whether anyone vouched for them.

The intended use is: validate the artifact separately, through the bounded parse
entry point for its kind, and then use the detached ID as a content address or
as the value a signature or other external mechanism commits to. That mechanism,
not the digest, is what authenticates. This crate defines no signer model, key
format, signature algorithm, trust policy, credential handling, registry
protocol, or network behaviour, and this document introduces none.

**Consequences, stated plainly.** Reformatting, whitespace, JSON key order,
numeric spelling, a rewritten `producer`, an added or edited extension, and a
changed `SourceInfo` field all change the artifact ID when they change the
bytes. That is the intended behaviour, not a limitation.

## 4. Coverage is exactly the bytes passed

Only the octets actually supplied are covered — the bytes passed to the call,
whether or not they have already been persisted anywhere — so what an artifact
ID binds depends on which kind the caller names.

`ArtifactKind` selects a domain and does nothing else: it neither parses nor
validates the bytes. The table below therefore describes a **valid serialized
document of the declared kind**. Arbitrary bytes hash just as well under any
kind, and the resulting ID makes no claim that they are a document of that kind
at all — see §3.

| Kind | Covers | Does not cover |
| --- | --- | --- |
| `ContractJsonV1` | Every byte of the Contract document: the format and profile markers, `identities` and `producer` as serialized, the type nodes, the declarations, and the actor when present | Envelope extensions and a `SourceInfo` sidecar — a bare Contract document contains neither |
| `ContractEnvelopeJsonV1` | Every byte of the envelope document: the nested Contract (including `identities` and `producer` as serialized) and the namespaced extension map | A `SourceInfo` sidecar — an envelope document does not contain one |
| `CompilationJsonV1` | Every byte of the compilation document: the Contract (including `identities` and `producer` as serialized) and the optional `SourceInfo` provenance sidecar when the document carries one | Envelope extensions — a compilation document does not contain any |

A valid serialized document of any of the three kinds above contains `producer`,
which neither semantic Contract identity covers, so producer coverage is a
property of the artifact kind rather than of the identity family:
`ContractJsonV1` binds the Contract's octets and nothing else, while the other
two bind them alongside extensions or the provenance sidecar.

There is no combined artifact envelope or package schema, and this document does
not define one. Package or application version is covered exactly when it is
literally present in the supplied artifact bytes — for example inside an
extension value in an envelope document — and is not covered otherwise. No
serialized artifact gains an `artifact_id` field: the identity is computed by an
explicit call and returned to the caller, never stored inside the artifact it
hashes.

## 5. Every identity in this crate, side by side

No unkeyed content ID authenticates itself. Each row below is a content address
over a different projection; signing one commits to that row's covered data and
to nothing else.

| ID | Kind of identity | Exactly what is covered | Equality means | Excluded |
| --- | --- | --- | --- | --- |
| `interface_id`<br>`candid-core:interface:v1` | Semantic Contract | The canonical type graph reachable from the actor, plus `semantics_profile` and `canonicalization_profile` | The same actor wire interface under the same profiles | Declaration names, actor-unreachable declarations, `producer`, `identities`, extensions, `SourceInfo`, source text, formatting. Absent for a declaration-only Contract |
| `contract_id`<br>`candid-core:contract:v1` | Semantic Contract | The complete canonical Contract payload: `format`, `format_version`, both profiles, every retained type node, declarations and their names, and the actor when present | The same complete semantic Contract | `producer`, `identities` itself, envelope extensions, `SourceInfo`, source text, comments, formatting, packaging. Two files with different producers, different extensions, or different sidecars share it |
| `source_bundle_id`<br>`candid-core:source-bundle:v1` | Raw-source bundle content | The canonical list of raw logical sources (`{name, source}`) and their import edges. Comment and documentation text inside a source is source bytes, so it is covered | The same raw source bytes and import edges | Everything *derived* from those sources: derived declaration/method/label provenance, derived documentation fields, the Contract, `producer`, extensions. It identifies the bundle, not the complete `SourceInfo`; a presented `SourceInfo` is validated by rederivation at construction time, which is a separate operation |
| `artifact_id`<br>`candid-core:artifact:*` | Detached exact-octet, per declared kind | The exact octet sequence handed to the call, under the named kind | The same kind and the same bytes | Nothing that is in the bytes; everything that is not. It makes no validity, authenticity, or provenance claim |

The scopes differ in both directions, and each direction matters. `contract_id`
and `interface_id` exclude `producer`, which is what keeps a semantic identity
from changing when an unrelated tool re-emits the same Contract — and is exactly
why neither may be treated as the identity of a complete produced document.
`artifact_id` fills that gap without moving either, and the checked-in vectors
demonstrate both halves at once: one shared `contract_id` across nine documents
with nine distinct artifact IDs, spanning all three kinds. In the other
direction, `source_bundle_id` is not a semantic identity at all: it identifies
raw source-file content, so it moves when raw source bytes or import edges move,
including for a reformatted source or an edited comment, and a change confined
to derived sidecar data need not move it.

## 6. Resource accounting

Computing an artifact identity is an explicit, detached call. It is never
implicit in a decode, so no existing validation order, error precedence, or
diagnostic changes because this exists.

1. `max_input_bytes` is enforced against the supplied slice **before any
   hashing**, and reports the resource `input_bytes`. An oversized artifact
   therefore always fails on the byte gate, even when the work budget is
   exhausted too.
2. `max_artifact_identity_work` is then charged, reporting the resource
   `artifact_identity_work`. The cost is exactly:

   ```text
   work = len(B) + len(D) + 1
   ```

   one unit per artifact byte plus the fixed domain framing. Nothing else is
   charged, and no other counter is consumed — in particular not
   `canonicalization_work` and not `source_identity_work`, so content-addressing
   a document can neither starve nor be starved by Contract canonicalization or
   provenance identity on a shared budget.

The default `max_artifact_identity_work` is `10_000_000`, chosen against the
default byte gate rather than guessed: the largest artifact `max_input_bytes`
admits by default is 4 MiB (4 194 304 bytes) and the longest domain,
`candid-core:artifact:contract-envelope-json:v1`, is 46 bytes, so the worst case
is 4 194 351 units. A new kind must therefore not introduce a longer domain
without re-checking this bound. Raising `max_input_bytes` above this value
without raising the work limit makes over-sized artifacts fail on
`artifact_identity_work` instead of on `input_bytes`; raise both together.

The input is hashed in bounded chunks of borrowed slices, so a large artifact is
never copied a second time and never becomes one uninterruptible block.
Cancellation and deadlines are observed before the first byte and again at every
chunk boundary, and both fail closed with `operation_cancelled` /
`operation_deadline_exceeded` rather than returning a partial digest.

Behaviour is identical on 32- and 64-bit targets for identical accepted inputs;
every charge saturates rather than wrapping, and the reported `{resource, limit,
observed}` metadata is fixed-width `u64` as everywhere else. The whole surface is
base-feature and builds and runs on `wasm32-unknown-unknown`, which needs no
clock and no filesystem to hash bytes.

## 7. The `max_artifact_identity_work` override

`max_artifact_identity_work` is an additive key in the portable limits
configuration (see the `LimitsConfig` section of [architecture](architecture.md)).
`LIMITS_CONFIG_VERSION` is unchanged at `1`: no existing key, value, or default
moved, and `Limits` fields are private precisely so that adding a limit is not a
breaking change.

The compatibility implication is one-directional and deliberate. A configuration
that leaves this limit at its profile value still serializes with no
`max_artifact_identity_work` key at all, so every previously written document is
unchanged in both directions. A document that *does* carry an explicit
`max_artifact_identity_work` override is readable by this build and newer ones,
and is **rejected** by a build that predates the key, because `LimitsConfig`
denies unknown override fields. Rejection is the intended behaviour: silently
ignoring an unrecognized limit override would apply a resource policy the
document did not ask for.

## 8. Test vectors

`tests/fixtures/artifact-identity/` holds the independent vectors, deliberately
outside `tests/fixtures/conformance/` so the closed semantic canonicalization
conformance set keeps meaning exactly what it did.

- `manifest.json` pins, per vector, the kind, the artifact file, the artifact ID,
  and — for the empty framing anchor — the complete preimage as hex. It also pins
  the domain of every kind and, under `cross_kind`, the ID the same bytes take
  under another kind.
- `verify_artifact_ids.py` recomputes all of it with the Python standard library
  and no Rust involvement, re-checks kind separation, and asserts that the nine
  documents sharing one `contract_id` — spanning all three kinds — have nine
  distinct artifact IDs.
- `tests/artifact_identity.rs` checks the same manifest from Rust, additionally
  pinning the raw Contract goldens as literals, and `tests/browser_wasm.rs` pins
  the per-kind framing anchors inside a real browser so the digest cannot differ
  between native and WASM.

CI runs the Python reference as its own `artifact-identity-reference` job.
