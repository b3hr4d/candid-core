# Candid Core

> **Unstable beta.** The version being prepared is `0.1.0-beta.2`;
> `0.1.0-beta.1` is published on crates.io. Until 1.0, any release may change the
> public Rust API, the serialized Contract/Compilation/envelope shapes, the
> canonical bytes, and every identity computed over them. Pin an exact version.
> See the [changelog](CHANGELOG.md) for the beta's scope and
> [known limitations](CHANGELOG.md#known-limitations), and
> [docs/releasing.md](docs/releasing.md) for the release procedure.

An early, deliberately narrow runtime foundation for turning Candid DID files into a canonical validated Contract graph. When the source compiler is enabled — it is, by default — the Rust core delegates parsing and type checking to the official `candid_parser` implementation; consumers never need to parse Candid source or reproduce its type rules. A consumer that only *consumes* Contracts can switch the compiler off and keep the model; see [Cargo features](#cargo-features).

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- validate ./contract.json
```

The compile command emits JSON containing a canonical validated `contract` and an optional, identity-bound `source_info` sidecar. The Contract exposes a full `contract_id` and an actor-only `interface_id`; source spelling/comments are identified separately by `source_bundle_id`. That source identity covers only the raw source files and import edges. At an external trust boundary, `SourceInfo::try_from_raw` recompiles that bundle and requires every presented provenance field to match the compiler-derived sidecar.

Those three are not one family. `contract_id` and `interface_id` are **semantic Contract identities**, so documents that mean the same thing share them on purpose; `source_bundle_id` is a **raw-source bundle content identity**, so it does identify raw source-file content — source bytes and import edges included — and formatting and comments inside a source therefore do move it. What none of the three identifies is a complete serialized `Contract`, `ContractEnvelope`, or `Compilation` document. A fourth does: `artifact_id_with_limits` hashes the exact octets of such a document and returns the ID to the caller — use it when exact octets are what must be committed to. See [what each identity claims](#what-each-identity-claims) below; no unkeyed content ID authenticates itself.

See [architecture](docs/architecture.md) and the [Contract graph](docs/contract-graph.md) for the v1 model, constraints, and the explicitly deferred host-value ↔ Candid binary bridge. The byte-level algorithm for the three canonicalized identities is specified normatively in [canonicalization v1](docs/canonicalization-v1.md), and the detached artifact identity in [artifact identity v1](docs/artifact-identity-v1.md). See [release verification gates](docs/verification.md) for the checks required before declaring the format stable across implementations. See [performance benchmarks](docs/benchmarks.md) for reproducible comparisons with the pinned official Candid checker and for allocation measurements. See the [changelog](CHANGELOG.md) for what this beta contains and what it does not, and [releasing](docs/releasing.md) for the exact release-candidate procedure and what is irreversible about publishing.

## TypeScript: `@candid-core/schema`

This repository also produces a TypeScript package, published on npm as [`@candid-core/schema`](https://www.npmjs.com/package/@candid-core/schema): a Zod-style schema runtime driven by the same canonical Contract model — builders with static inference, fail-closed structural validation, a TypeScript-native Candid binary codec, typed actors over a transport-only agent, and form metadata.

```sh
npm install @candid-core/schema @icp-sdk/core
```

Its source of truth, README, and changelog live in [crates/candid-core-ts/ts/](crates/candid-core-ts/ts/), and it versions independently of this crate (pre-1.0, like everything here). The `candid-core` binary above is what turns a `.did` file into the Contract JSON that package's `schemaFromContract` consumes; the generator crate around it, `candid-core-ts`, is unpublishable by design.

## What each identity claims

Every ID below is a content address over a different projection. **No unkeyed content ID authenticates itself**: a signature or other external mechanism is what authenticates, and signing one of these commits to exactly what its second column lists and to nothing else.

| ID | Kind of identity | Exactly what is covered | Equality means | Excluded |
| --- | --- | --- | --- | --- |
| `interface_id` | Semantic Contract | The canonical type graph reachable from the actor, plus both profile markers | The same actor wire interface under the same profiles | Declaration names, actor-unreachable declarations, `producer`, extensions, `SourceInfo`, source text, formatting. Absent for a declaration-only Contract |
| `contract_id` | Semantic Contract | The complete canonical Contract payload: format markers, both profiles, every retained type node, declarations and their names, and the actor when present | The same complete semantic Contract | `producer`, envelope extensions, `SourceInfo`, source text, comments, formatting, packaging. Two files with different producers, extensions, or sidecars share it |
| `source_bundle_id` | Raw-source bundle content | The canonical list of raw logical sources and their import edges — comment and documentation text inside a source is source bytes, so it is covered | The same raw source bytes and import edges | Everything *derived* from those sources: derived declaration/method/label provenance and derived documentation fields, so it identifies the bundle, not the complete `SourceInfo`. A presented sidecar is validated by rederivation at construction time, not by this ID |
| `artifact_id` | Detached exact-octet, per declared `ArtifactKind` | The exact octet sequence passed to `artifact_id_with_limits` | The same kind and the same bytes | Nothing that is in the bytes; everything that is not. It makes no validity, authenticity, or provenance claim |

```rust,ignore
use candid_core::{artifact_id_with_limits, ArtifactKind, Limits};

// Detached: the ID is returned to you, never written into the document, and
// never computed implicitly by a decode. Validate the artifact separately.
let id = artifact_id_with_limits(
    ArtifactKind::ContractEnvelopeJsonV1,
    document_bytes,
    &Limits::default(),
)?;
```

Reformatting, whitespace, key order, a rewritten `producer`, an extension edit, and a `SourceInfo` edit all change `artifact_id` and leave `contract_id` and `interface_id` exactly where they were. Coverage is exactly the bytes passed to the call — whether or not those bytes have been persisted anywhere — so what travels with them depends on the kind named. `ArtifactKind` selects a domain and neither parses nor validates, so the following describes a *valid serialized document of the declared kind*: a `Contract` document carries the Contract alone, `producer` included; an envelope document carries extensions and no `SourceInfo`; a compilation document carries a `SourceInfo` sidecar and no extensions; and package or application version is covered only when it is literally in the supplied artifact bytes. Arbitrary bytes hash just as well under any kind, and the resulting ID claims nothing about their validity. There is no signer model, key format, signature algorithm, trust policy, or registry protocol here — [artifact identity v1](docs/artifact-identity-v1.md) and [ADR 0007](docs/adrs/0007-artifact-identity.md) state the boundary precisely.

## Command-line interface

The `candid-core` binary requires the `filesystem-compiler` feature, which is on by default. It accepts exactly this grammar, and nothing else:

```text
candid-core compile <path> [--no-source-info | --envelope]
candid-core validate <path>
```

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- compile ./service.did --no-source-info
cargo run --bin candid-core -- compile ./service.did --envelope
cargo run --bin candid-core -- validate ./contract.json
```

Anything outside the grammar — an unknown command, an unknown option, a missing path, an option before the path, a flag on `validate`, a duplicate flag, `--no-source-info` combined with `--envelope` (the envelope exists to carry the field names that flag suppresses), or any trailing argument — is a usage error: the process writes nothing to stdout, exits with status 64, and prints exactly this usage text on stderr:

```text
usage: candid-core compile <path> [--no-source-info | --envelope]
       candid-core validate <path>
```

A token that begins with `-` is always treated as an option in the path position, never as a path; spell a dash-leading relative file with a `./` prefix (`candid-core compile ./-service.did`). Path arguments are taken as OS-native bytes, so a non-Unicode path is never an argument-parsing failure: `validate` hands the bytes to the filesystem unchanged, while `compile` additionally requires the entry file name to be valid UTF-8 — it becomes the source ID — and reports a `did_invalid_source_id` diagnostic otherwise. There are no other commands or flags — in particular no flags for custom limits; pass custom `Limits` through the library APIs instead.

Every non-usage outcome is one pretty-printed JSON document on stdout with an empty stderr:

| Outcome | Exit status | stdout | stderr |
| --- | --- | --- | --- |
| success | 0 | JSON with `"ok": true` | empty |
| read, parse, validation, or resource-limit failure | 1 | JSON with `"ok": false` | empty |
| usage error | 64 | empty | the usage text above |

`compile <path>` emits `{"ok": true, "contract": …, "source_info": …}` on success and `{"ok": false, "diagnostics": […]}` on failure. `--no-source-info` suppresses the provenance sidecar; the key stays present as `"source_info": null`. `compile <path> --envelope` emits the `ContractEnvelope` document itself on success — `{"contract": …, "extensions": {"org.candid-core.field-names/v1": [[container, id, name], …]}}`, with the named field labels sorted and deduplicated — rather than an `ok`-wrapped response, so the output can be saved whole and handed directly to `@candid-core/schema`'s `schemaFromContract`; extensions live outside `contract_id` and `interface_id` by the envelope's design, and failures use the same `{"ok": false, "diagnostics": […]}` channel as plain `compile`. `validate <path>` emits `{"ok": true, "contract": …}` on success, `{"ok": false, "diagnostics": […]}` when the document cannot be read or is not JSON, and `{"ok": false, "violations": […]}` when it parses but fails validation or when the read exceeds the input byte bound (a single `resource_limit_exceeded` violation at path `$`). Codes are stable identifiers such as `did_parse_error`, `did_file_read_error`, `contract_file_read_error`, `malformed_contract_json`, and `resource_limit_exceeded`.

Diagnostics and violations share one item schema (see the Diagnostics section of `docs/architecture.md`): codes, paths, and resource metadata are the stable machine surface, message text is not. Items may additionally carry an optional structured `path`, a `span` naming the logical source ID (with byte offsets only when they are exact for the original text), and ordered `related` locations; these keys are omitted when the data does not exist, so pre-existing output shapes are unchanged. Locations always name logical source IDs (`workspace:/…`) — never the temporary files the compiler materializes for import checking.

Both commands bound their reads under `Limits::default()` — the versioned `interactive_v1` profile — before decoding, so an oversized file fails with a `resource_limit_exceeded` code carrying `{resource, limit, observed}` metadata (fixed-width `u64` values, identical on every platform) instead of allocating without bound, and the byte bound takes precedence over UTF-8 and parse errors:

- `compile` reads every source file through the workspace resolver: each file is bounded by `max_source_bytes` (1 MiB, resource `source_bytes`), and the import bundle in aggregate by `max_bundle_bytes` (8 MiB, resource `bundle_bytes`); an over-limit source fails in the `diagnostics` shape.
- `validate` reads the contract document bounded by `max_input_bytes` (4 MiB, resource `input_bytes`); an over-limit document fails in the `violations` shape.

The real-binary suite in `tests/cli.rs` pins this contract: the argument matrix, exit statuses, output channels, JSON shapes and codes, source-info suppression, and both byte bounds at the exact limit and one byte over.

## Runnable examples

The examples show why the Contract is a graph, how semantically equivalent DID sources share an identity, and how strict JSON validation protects the core:

```sh
cargo run --example contract_walkthrough    # compiler
cargo run --example semantic_equivalence    # compiler
cargo run --example trust_boundary          # compiler
cargo run --example bounded_parsing         # compiler
cargo run --example hermetic_bundle         # compiler
cargo run --example host_value_validation   # compiler + host-value
```

Each example declares its `required-features`, so `cargo run --example …` under a reduced feature set reports that the example was skipped rather than failing to compile.

`contract_walkthrough` prints a canonical recursive Contract and its provenance summary. `semantic_equivalence` compares interface identity with source identity. `trust_boundary` demonstrates rejection of injected metadata and a tampered identity. `hermetic_bundle` shows filesystem-free import resolution — the browser path, which is why it needs only `compiler` — while `host_value_validation` preserves a large `nat` and an IEEE NaN payload. `bounded_parsing` rejects oversized untrusted documents before decoding and shows the second limit serialization consumes.

## Foundation decisions

Seven implemented [foundation ADRs](docs/adrs/README.md) define the boundaries for large-ecosystem use:

1. separate interface, Contract, and source-bundle identities;
2. independently version schema, Candid semantics, and canonical bytes;
3. make validated artifacts and provenance binding explicit;
4. resolve imports through a hermetic capability boundary;
5. bound all untrusted work;
6. use a lossless tagged HostValue ABI; and
7. give artifacts whose exact octets must be committed to a detached identity.

All seven decisions are implemented in the Rust reference runtime. Because the crate has not been released, this profile is the clean starting point rather than a compatibility layer over an earlier format.

Implemented is not the same as verified, and the two are tracked separately. **ADR 0002** (independently version schema, Candid semantics, and canonical bytes) is **Verified**: a Python reference outside this crate reproduces all 11 canonicalization vectors, and that run is recorded in [release verification gates](docs/verification.md). **ADRs 0001 and 0003–0007** remain **Implemented, verification pending** — each has an implementation, ADR 0007 also has an independent reference and a dedicated CI job, but no run of that job is recorded yet, and wiring is not evidence. The Contract format is therefore **not** a stable v1, and this beta does not promote it to one.

## Rust version and dependencies

The crate advertises Rust 1.78 as its minimum supported Rust version (MSRV). Direct dependencies are pinned to versions that are expected to build on that toolchain, and dependency updates should preserve the advertised MSRV unless the `rust-version` field is intentionally raised in the same change. CI runs the locked dependency graph against Rust 1.78, so an incompatible direct or transitive dependency update fails before merge.

## Cargo features

`candid-core` stays one published package with one library and one binary. What it *builds* is split into an always-present base plus three features, all enabled by default:

| Feature | Adds | Dependencies it pulls in |
| --- | --- | --- |
| *(base)* | `Contract`, `ContractDraft`, `RawContract`, `ContractEnvelope`, validation, canonicalization, the semantic Contract identities, detached `artifact_id_with_limits`/`ArtifactKind`, `Limits`/`RuntimeContext`/`CancellationToken`, `Diagnostic` | `serde`, `serde_json`, `sha2`, `hex` |
| `host-value` | `HostValue`, `HostFieldValue`, `validate_host_value`, `ContractTypeRef`/`ContractMethodRef`, `Contract::bind_type`/`bind_method` | `ic_principal` |
| `compiler` | `compile_did` and its option/context variants, `compile_with_resolver`, `Compilation`, `CompileOptions`, `CompileError`, `SourceId`/`SourceResolver`/`ResolvedSource`/`MemoryResolver`, `SourceInfo`/`RawSourceInfo` provenance | `candid`, `candid_parser` |
| `filesystem-compiler` (implies `compiler`) | `WorkspaceResolver`, `compile_did_file` and its variants, source materialization for `candid_parser::check_file`, the `candid-core` binary | `cap-std` |

Because every feature is on by default, the full surface needs no feature selection at all. The version requirement, however, has to name the prerelease explicitly: `0.1.0-beta.2` is a prerelease, and a caret requirement such as `"0.1"` will **not** resolve to it. Cargo only selects a prerelease when the requirement mentions one, so every example below pins the exact version.

```toml
# the full surface, every feature on by default
candid-core = "=0.1.0-beta.2"

# a pure Contract consumer: model, validation, canonicalization, identities
candid-core = { version = "=0.1.0-beta.2", default-features = false }

# ... plus the lossless tagged host value ABI
candid-core = { version = "=0.1.0-beta.2", default-features = false, features = ["host-value"] }

# a browser/WASM host that compiles DID source it already has — self-contained
# or a multi-file bundle with imports
candid-core = { version = "=0.1.0-beta.2", default-features = false, features = ["compiler"] }

# a native tool that reads .did files, or uses the CLI
candid-core = { version = "=0.1.0-beta.2", default-features = false, features = ["filesystem-compiler"] }
```

`cargo add candid-core@=0.1.0-beta.2` does the same thing from the command line. Pinning with `=` is the right choice for a prerelease for a second reason: with the API and the wire format both unstable before 1.0, an automatic upgrade to `0.1.0-beta.3` is a change a consumer should opt into deliberately.

Items outside the selected set are **absent at compile time**, not runtime stubs: a build error names the missing item, and turning on the feature it belongs to is the fix. `tests/model_public_api.rs` pins the root exports of each surface, and `tests/fixtures/packaging/verify_feature_graph.py` proves the dependency claims in the table above against `cargo metadata` — the base graph resolves to 23 packages where the default graph resolves to 126, and a `compiler`-only browser-WASM graph to 106.

Two caveats, both deliberate:

- **Cargo unifies features across a build.** If anything else in your dependency graph depends on `candid-core` with defaults, the whole surface is compiled once for every consumer in that build. Feature selection bounds what a *dependency graph* must contain; it cannot subtract from a graph that already asked for more.
- **Feature selection does not shrink the published `.crate` archive.** Every *published* source file ships regardless of which features a consumer enables; a `default-features = false` consumer still downloads the `compiler` sources it will not build. What the archive contains is bounded separately, by the positive `include` allowlist in `Cargo.toml`: production source, runnable examples, the public `docs/` set, and the three root documents. Test suites and their fixtures, benchmarks, the fuzz crate, and CI assets are not published, and `tests/fixtures/packaging/verify_package_manifest.py` asserts that in both directions.

Producer metadata is unaffected by any of this: `ProducerInfo::current` reports the same `name`, `version`, `candid_version`, and `candid_parser_version` in every configuration, because it reads the pinned versions from this package's manifest at compile time rather than from a linked crate. It remains **unverified** provenance held outside the semantic Contract identities: neither `contract_id` nor `interface_id` covers it, so rewriting it leaves both byte-identical. A caller that needs to commit to the producer bytes it actually received commits to an `artifact_id`, and what else travels with them depends on the kind named — `ContractJsonV1` binds the Contract document's octets, `producer` included and nothing more; the envelope and compilation kinds bind those octets plus extensions or the provenance sidecar respectively. See the [identity ADR](docs/adrs/0001-contract-identities.md) and [ADR 0007](docs/adrs/0007-artifact-identity.md).

## Browsers and bare WASM

`compiler` is the browser surface, and it now covers imported bundles as well as self-contained sources ([issue #21]). `compile_with_resolver` takes an entry ID plus a `SourceResolver` and compiles the whole bundle in memory: the sources are merged into one virtual program and type-checked through the official `candid_parser` merged-program APIs. There is no materialization, no temporary directory, no `cap-std`, and no ambient authority anywhere on that path.

```rust,ignore
use candid_core::{compile_with_resolver, CompileOptions, MemoryResolver, RuntimeContext};

// Sources the host already holds — fetched, bundled, typed into an editor.
let resolver = MemoryResolver::new()
    .with_source("memory:/entry.did", r#"import service "api.did"; import "types.did";
                                         service : { local: (Item) -> () };"#)?
    .with_source("memory:/api.did", "service : { imported: () -> (nat) query };")?
    .with_source("memory:/types.did", "type Item = record { id: nat };")?;

let compilation = compile_with_resolver(
    "memory:/entry.did",
    &resolver,
    CompileOptions::default(),
    &RuntimeContext::default(),
)?;
```

The supported boundary is **synchronous immutable source data supplied by the host**. Implement `SourceResolver` yourself when the bundle is not a `MemoryResolver` — the resolver decides identity and returns bytes, and the compiler owns every limit. `candid-core` never fetches anything, resolves asynchronously, consults a registry, or generates bindings.

Two target facts, stated plainly:

- **`WorkspaceResolver` and `compile_did_file` are native.** They still compile for `wasm32-unknown-unknown` (`cap-std` is declared under `cfg(not(target_os = "unknown"))`), but there is no directory to open there, so `WorkspaceResolver::new` fails with `did_workspace_root_error`. Use a memory or host-provided resolver.
- **Bare `wasm32-unknown-unknown` has no clock.** Cancellation and every quantitative limit work exactly as they do natively. An explicit `Limits::with_deadline_unix_ms` cannot be measured there, so it fails closed with `operation_deadline_exceeded` rather than calling an unsupported `std::time` function and aborting the module; `Limits::deadline_exceeded` reports the same. No deadline configured stays unbounded, as everywhere else. The crate takes no `web-time`, `js-sys`, or `wasm-bindgen` production dependency to paper over this.

This is a runtime claim, not a build claim: `tests/browser_wasm.rs` compiles a self-contained source and an imported multi-file bundle — both import kinds plus a diamond — inside headless Chrome, pins the resulting `contract_id`, `interface_id`, `source_bundle_id`, logical sources and import edges, and asserts that resolver, resource, cancellation, and deadline failures stay structured. CI runs it against an exactly pinned Chrome for Testing build (`150.0.7871.124`) driven by the ChromeDriver from that same build, not against a rolling channel. The same file's assertions run natively in every other job, which is what keeps the two in step. See [release verification gates](docs/verification.md).

[issue #21]: https://github.com/b3hr4d/candid-core/issues/21

## Platform APIs

Each item below is tagged with the feature that provides it; untagged items are in the base.

- `ContractDraft` → `build`/`build_with_limits`/`build_with_context` is the
  producer path: a draft carries only types, declarations, an optional actor,
  and optional producer metadata — never format markers or identities — and
  building calculates fresh identities under the same budgets as every other
  entry point.
- `RawContract` → `Contract::try_from_raw` validates a decoded external
  artifact, verifying its presented identities against recomputation.
- *(`compiler`)* `compile_did` compiles one self-contained DID source with no
  filesystem and no import resolution. A source that declares imports is
  rejected with `did_import_requires_file`, which points at
  `compile_with_resolver`.
- *(`compiler`)* `compile_with_resolver` compiles an immutable logical source
  bundle — imports included — through `MemoryResolver` or any host-supplied
  synchronous `SourceResolver`, with no filesystem. Together with `compile_did`
  this is the surface that runs on `wasm32-unknown-unknown`.
- *(`compiler`)* `RawSourceInfo` → `SourceInfo::try_from_raw` recompiles the
  embedded source bundle and rejects any derived provenance that does not match
  exactly. It shares one backend with `compile_with_resolver`, so authentication
  cannot drift from compilation.
- *(`filesystem-compiler`)* `compile_did_file` compiles a `.did` file from a
  sandboxed `WorkspaceResolver` through `candid_parser::check_file` over a
  materialized copy of the resolved bundle; the `candid-core` binary is built on
  that path.
- `Limits` and constructor-based `RuntimeContext` bound untrusted compilation
  and validation with one shared budget, monotonic deadlines, and cooperative
  `CancellationToken` support. Defaults come from the versioned
  `LimitsProfile::InteractiveV1`; individual fields are overridden with
  `with_*` builders, and the serialized form is the versioned portable
  configuration `{"version":1,"profile":"interactive_v1","overrides":{…}}`
  with fixed-width `u64` override values (see the architecture doc for the
  full wire contract, including zero, overflow, and unknown-version
  behavior).
- *(`host-value`)* `HostValue` plus `validate_host_value` provide the lossless tagged value ABI.
- `ContractEnvelope` keeps namespaced extensions outside the strict core.
- `artifact_id_with_limits`/`artifact_id_with_context` compute a detached,
  exact-octet identity for a serialized Contract, envelope, or compilation
  document, under the `ArtifactKind` the caller names. The ID is returned to the
  caller, never stored in the artifact, and never computed by a decode;
  `max_input_bytes` gates the slice before hashing and
  `max_artifact_identity_work` meters the hash on its own counter.

### Migrating from the pre-cleanup producer APIs

`RawContract::new` and `Contract::build_raw`/`build_raw_with_context` were
removed in the pre-1.0 API cleanup ([issue #23]): a producer-facing
constructor that fabricated placeholder zero identities made the intuitive
`RawContract::new` → `Contract::try_from_raw` pairing fail by construction.
`ContractDraft` has no identity fields at all, so the mistake is now
unrepresentable.

```rust,ignore
// Before:
let raw = RawContract::new(types, declarations, actor);
let contract = Contract::build_raw(raw, &limits)?;
// After:
let contract = ContractDraft::new(types, declarations, actor)
    .build_with_limits(&limits)?;               // .build() for Limits::default()
// A caller-supplied producer used to travel inside the RawContract; now:
let contract = ContractDraft::new(types, declarations, actor)
    .with_producer(producer)
    .build_with_limits(&limits)?;
```

`Limits` no longer exposes public fields or exhaustive struct literals;
construction goes through a profile plus builders, and reads through getters:

```rust,ignore
// Before:
let limits = Limits { max_input_bytes: 512, ..Limits::default() };
let ceiling = limits.max_canonicalization_work;
// After:
let limits = Limits::default().with_max_input_bytes(512);
let ceiling = limits.max_canonicalization_work();
```

`ResourceLimitInfo.limit`/`.observed` and `SourceSpan.start_byte`/`.end_byte`
changed from platform-width `usize` to fixed-width `u64`; the serialized JSON
numeric text is unchanged. Serialized `Limits` documents changed from a bare
field map to the versioned portable configuration shown above.

[issue #23]: https://github.com/b3hr4d/candid-core/issues/23

## Bounded parsing and trusted serde integration

Untrusted bytes and already-validated values take different paths, and the crate does not let the two be confused.

- `Contract`, `ContractEnvelope`, `Compilation` (`compiler`), and `HostValue` (`host-value`) do not implement `Deserialize`: a trait impl has no argument position for a resource policy, so it could only ever decode under limits the library chose.
- Untrusted Contract, Compilation, and envelope JSON goes through `from_json_with_limits`/`from_json_with_context` and `from_slice_with_limits`/`from_slice_with_context`. These enforce `max_input_bytes` before decoding and then share one budget with validation. `Contract::from_json` is the same path under `Limits::default`. The byte gate bounds peak decode allocation to a multiple of the caller's ceiling; it does not reject element by element during decode, which remains a follow-up.
- `HostValue` is the exception to that sentence: `HostValue::from_json_with_limits`/`from_json_with_context` gate on `max_value_bytes`, not `max_input_bytes`, and report `HostValueJsonError::Limit`, which carries no `resource` name. Lowering `max_input_bytes` alone does not bound HostValue decoding — lower `max_value_bytes` too.
- `Serialize` and the derived `Deserialize` on the raw DTOs (`RawContract`, `RawSourceInfo`) are the trusted serde integration: they consult no limits and revalidate nothing. Decoding a raw DTO is not a bounded operation, so callers must gate byte length themselves or use a bounded parse API.
- The `to_json_pretty_with_limits`/`to_json_pretty_with_context` serializers on `Contract`, `Compilation`, and `ContractEnvelope` validate before rendering and charge the rendered length against `max_canonicalization_work`. That is a second budget: raising a structural limit such as `max_string_bytes` to build a value does not by itself make that value renderable. (`Compilation` validates its Contract, not its already-authenticated sidecar; rederiving provenance is construction-time work.)

`HostValue` is the crate's one recursive value type, and it is bounded on both sides.

- **Decoding** runs a constant-stack scan of the JSON text before `serde_json` sees it, rejecting anything nested deeper than `max_value_nesting` as a `value_nesting` resource limit. That check counts JSON containers, the same unit serde_json counts, so keeping the limit below serde_json's fixed 128-frame ceiling means the crate-owned check is always the one that fires — with `{resource, limit, observed}` metadata instead of a serde string. The ceiling itself is left in place and unmodified underneath. Raising `max_value_nesting` above 128 therefore has no effect.
- **Construction** is bounded by `max_value_depth` and `max_value_elements`: `HostValue::opt`, `vector`, `record`, and `variant` take a `&Limits` and fail closed. This is what keeps the recursive operations on the type safe, since `Drop`, `Clone`, `PartialEq`, `Debug`, and `Serialize` all walk one stack frame per level and none of the first four can report an error.

Lexical nesting and semantic depth are deliberately separate limits, as they are for source (`max_source_nesting`) and types (`max_type_depth`): one `vec` level costs two JSON containers and one `record` level costs three, so a single limit could not report an honest `observed` value for both.
