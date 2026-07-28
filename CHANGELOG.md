# Changelog

This file records what changed between released versions of `candid-core`. The
release procedure that produces an entry here is [docs/releasing.md](docs/releasing.md).

`candid-core` is pre-1.0. Until 1.0, **any** release may change the public Rust
API, the serialized Contract/Compilation/envelope shapes, the canonical bytes,
and therefore the identities computed over them. Pin an exact version.

## 0.1.0-beta.2 — prepared, not yet published

The second prerelease. As with every release, the archive is built and
digested before publication, so the copy of this file inside the published
archive necessarily says "prepared"; the release record carries the
publication evidence. This version follows the issue #81 bump protocol:
`ProducerInfo::current().version` (and therefore the octets of newly
serialized documents) is the only observable change from the bump itself — no
`contract_id`, `interface_id`, `source_bundle_id`, or frozen exact-octet
vector moved, and `tests/release_metadata.rs` asserts the change and the
non-change together.

### Release automation

- **The tag, crates.io publish, and GitHub release are performed by a
  dispatch-only workflow behind three separately-approved protected
  environments** ([issue #89]). The guard repackages the release commit with
  the pinned Cargo and refuses on any digest mismatch; publication uses
  crates.io Trusted Publishing, so no long-lived registry credential exists
  anywhere; the published checksum is read back from crates.io and compared
  automatically. [docs/releasing.md](docs/releasing.md) §7 now documents the
  dispatch-and-approve procedure, with the manual path retained as a recorded
  fallback. This version is the first released through it.

### Release gates

- **Semver compatibility is now checked against the published baseline**
  ([issue #92]). `cargo semver-checks` compares both published surfaces against
  the latest release and reports breaking changes. Because this crate is pre-1.0
  and reserves the right to break, the gate does not forbid them — it forbids
  *undocumented* ones: a reported break must be acknowledged by a list item in
  the Unreleased section of this file that begins with a bolded `BREAKING`
  marker followed by a colon. The marker has to start the item; a mention
  inside prose does not count, which is why this entry does not itself
  acknowledge anything.
  Adding this gate does not promise semver stability before 1.0, and does not
  promote the Contract format to a stable v1.

### Documentation

- **The effective ceiling on HostValue record width is now documented**
  ([issue #88]). Record validation is deliberately allocation-free: it scans
  pairwise and charges `canonicalization_work` per comparison rather than
  building a field-ID index, so the cost is roughly `1.5n²` for an `n`-field
  record. At the default `max_canonicalization_work` that binds a single record
  at **2 581 fields** — far below the 500 000 `max_fields` permits and the
  1 000 000 `max_value_elements` permits, neither of which is the binding limit
  for a wide record. The rustdoc on all three limits now says so, and a
  regression test pins the boundary and the exact resource metadata one field
  over. No behaviour changed: this release documents what the code already did.
  The failure was, and remains, structured and deadline-interruptible rather
  than a hang.
- **Benchmark comparison governance is documented**
  ([issue #39], now delivered and closed).
  [docs/benchmarks.md](docs/benchmarks.md) records the durable-baseline
  format, the comparison tool that refuses rather than guesses when runs are
  not comparable, the three CI tiers, and the standing decision that no timing
  or allocation measurement ever fails a workflow in this repository.

### Packaging

- **The repository became a Cargo workspace** ahead of the TypeScript
  generator crate ([issue #38]). This is invisible to consumers: Cargo strips
  the `[workspace]` table from the normalized manifest, the archive remains
  exactly the intentional allowlist, and the packaged-consumer gates prove it.
  The generator crate itself is unpublishable and outside the archive.

## 0.1.0-beta.1 — published 2026-07-27

The first published version, released from commit
`819a3c9062bf6420bec66fb6e8fd9c7c67add50c` with Cargo 1.94.1. The `.crate`
SHA-256 crates.io recorded is
`4082434fe0057bf9bccabd9e987cb9772137488e6294e05f28c094b544cf8224`, matching the
digest the release gates measured.

The archive published under this version was built before publication, so the
copy of this file inside it still describes the release as prepared rather than
published — unavoidable, since editing it would change the commit and therefore
the digest. The tag, the crates.io publish, and the GitHub prerelease were
separately authorized steps, set out in [docs/releasing.md](docs/releasing.md).

Because this is the first version, everything below describes the shape of the
initial surface rather than a change from a predecessor.

### The crate

- **Four build surfaces from one published package.** The implicit base — what
  is left when `default-features = false` removes every feature — is the pure
  Contract model: DTOs, validation, canonicalization, the semantic identities,
  detached artifact identity, `Limits`/`RuntimeContext`/`CancellationToken`,
  `ContractEnvelope`, and the diagnostics those need. It links no Candid engine
  and no filesystem capability crate. `compiler` adds Candid source compilation
  with no host filesystem. `filesystem-compiler` adds native filesystem access
  and the `candid-core` binary on top of `compiler`. `host-value` adds the
  lossless tagged host value ABI and graph-directed value validation. All three
  are on by default.

  These are *dependency-graph* boundaries, and they are verified as such:
  `tests/fixtures/packaging/verify_feature_graph.py` resolves `cargo metadata`
  per feature set and target and asserts what each one may and may not contain.
  Cargo unifies features across a build, so feature selection bounds what a
  dependency graph must contain, not what a single unified build produces.

- **Browser-WASM imported compilation.** `compiler` is the surface a browser
  host builds, and it covers multi-file bundles as well as self-contained
  sources. `compile_with_resolver` takes an entry ID plus a `SourceResolver` and
  compiles the whole bundle in memory through the official `candid_parser`
  merged-program APIs — no materialization, no temporary directory, no
  `cap-std`, no ambient authority. `tests/browser_wasm.rs` is the runtime
  evidence: it compiles both import kinds plus a diamond inside headless Chrome,
  pins the resulting identities and provenance, and asserts that resolver,
  resource, cancellation, and deadline failures stay structured on a target with
  no filesystem and no clock.

- **Semantic identity and exact-octet identity are different things.**
  `contract_id` and `interface_id` are *semantic* Contract identities: documents
  that mean the same thing share them on purpose, so rewriting `producer`,
  editing an envelope extension, or re-encoding the JSON leaves both untouched.
  `source_bundle_id` is a raw-source bundle content identity over source bytes
  and import edges. None of the three identifies a complete serialized document.
  `artifact_id_with_limits` does, per declared `ArtifactKind`, and returns the ID
  to the caller rather than writing it into the document. No unkeyed content ID
  authenticates itself.

- **All untrusted work is bounded.** Untrusted JSON goes through the
  `*_with_limits`/`*_with_context` parse APIs, which gate byte length before
  decoding and then share one budget with validation. `Contract`,
  `ContractEnvelope`, `Compilation`, and `HostValue` deliberately do not
  implement `Deserialize`, because a trait impl has no argument position for a
  resource policy.

### Pre-1.0 API and wire instability

- The public Rust API is unstable. `tests/fixtures/packaging/public-api-base.txt`
  and `public-api-all-features.txt` are committed inventories of both published
  surfaces so that a change is visible in review. They record the API; they do
  not promise it.
- The serialized Contract, Compilation, and envelope shapes are unstable, and so
  are the canonical bytes and every identity computed over them. A future
  release may move `contract_id` for an unchanged input; if that happens it will
  be a canonicalization-profile change recorded here.
- The Contract format is **not** promoted to a stable v1 by this release. See
  the ADR status note under Known limitations.
- `cargo semver-checks` is deliberately absent: there is no published
  `candid-core` baseline to compare against. Semver comparison begins with the
  release *after* this one.
- Because `0.1.0-beta.1` is a prerelease, a `"0.1"` requirement does not select
  it. Consumers ask for it by name: `candid-core = "=0.1.0-beta.1"`.

### Migrations already in place

These removals happened during the pre-1.0 API cleanup ([issue #23]) and are
documented in the README under "Migrating from the pre-cleanup producer APIs".
They are repeated here because a first-time reader of this changelog will not
have seen them:

- `RawContract::new` and `Contract::build_raw`/`build_raw_with_context` are
  gone. A producer-facing constructor that fabricated placeholder zero
  identities made the intuitive `RawContract::new` → `Contract::try_from_raw`
  pairing fail by construction. Use `ContractDraft`, which has no identity
  fields at all, plus `.with_producer(..)` when a caller-supplied producer is
  needed.
- `Limits` no longer exposes public fields or exhaustive struct literals.
  Construct through a profile plus builders (`Limits::default().with_…`) and
  read through getters.
- `ResourceLimitInfo.limit`/`.observed` and `SourceSpan.start_byte`/`.end_byte`
  moved from platform-width `usize` to fixed-width `u64`. The serialized JSON
  numeric text is unchanged.
- Serialized `Limits` documents moved from a bare field map to the versioned
  portable configuration.

### Packaging

- The published archive is a positive `include` allowlist: production source,
  runnable examples, the public documentation set, `README.md`, `CHANGELOG.md`,
  `LICENSE`, and the manifest material Cargo adds itself. Test suites and their
  fixtures, benchmarks and their corpus, the fuzz crate, CI workflows, and
  agent/skill assets are not published.
  `tests/fixtures/packaging/verify_package_manifest.py` asserts both halves:
  every required path present, every forbidden path absent, and no unexplained
  extra path.
- A root `LICENSE` carries the complete Apache License 2.0 text.
- The manifest declares `repository`, `homepage`, `documentation`, `readme`, and
  a narrow keyword/category set.
- A `Release candidate` workflow packages the crate with the exact Cargo pinned
  in `tests/fixtures/packaging/release-tools.env` — `cargo package` bytes are
  reproducible within a Cargo version and not across versions, so the digest and
  the publish have to share one — verifies the archive manifest, records its
  SHA-256, Cargo version, and exact file list as CI evidence, and builds
  external consumers — base, `compiler`, full native, the installed CLI, and two
  `wasm32-unknown-unknown` surfaces — against the *unpacked archive* rather than
  against this repository. It holds `contents: read`, accepts no crates.io
  token, and neither tags, releases, nor publishes anything.

### Known limitations

- **ADR verification is incomplete.** ADR 0002 (versioning and canonical bytes)
  is **Verified**: an independent Python reference reproduces all 11
  canonicalization vectors, and the run is recorded in
  [docs/verification.md](docs/verification.md). ADRs 0001 and 0003–0007 remain
  **Implemented, verification pending**. The implementations are in place and,
  for ADR 0007, an independent reference and a CI job exist, but no run of that
  job is recorded yet. Wiring is not evidence.
- **The host-value ↔ Candid binary bridge is deferred.** `HostValue` is a
  lossless tagged ABI validated against the Contract graph; encoding to and
  decoding from Candid's binary wire format is not part of this crate yet.
- **Bounded decoding is coarse.** `from_json_with_limits` and its siblings
  enforce `max_input_bytes` before decoding, which bounds peak decode allocation
  to a multiple of the caller's ceiling. They do not reject element by element
  during decode; that remains a follow-up.
- **`HostValue` uses a different byte gate.** `HostValue::from_json_with_limits`
  gates on `max_value_bytes`, not `max_input_bytes`, and reports
  `HostValueJsonError::Limit`, which carries no `resource` name. Lowering
  `max_input_bytes` alone does not bound HostValue decoding.
- **Raising `max_value_nesting` above 128 has no effect**, because serde_json's
  own fixed 128-frame ceiling is left in place underneath the crate-owned check.
- **TypeScript generation and comparison are not in this release**
  ([issue #38]).
- **Benchmark regression gating is not in this release** ([issue #39]). Pull
  requests compile and exercise every benchmark once; weekly runs retain
  Criterion estimates, allocation measurements, toolchain, host, and commit as
  CI artifacts, but no wall-clock threshold is enforced.

[issue #23]: https://github.com/b3hr4d/candid-core/issues/23
[issue #38]: https://github.com/b3hr4d/candid-core/issues/38
[issue #39]: https://github.com/b3hr4d/candid-core/issues/39
[issue #88]: https://github.com/b3hr4d/candid-core/issues/88
[issue #92]: https://github.com/b3hr4d/candid-core/issues/92
