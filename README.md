# Candid Core

An early, deliberately narrow runtime foundation for turning Candid DID files into a canonical validated Contract graph. When the source compiler is enabled — it is, by default — the Rust core delegates parsing and type checking to the official `candid_parser` implementation; consumers never need to parse Candid source or reproduce its type rules. A consumer that only *consumes* Contracts can switch the compiler off and keep the model; see [Cargo features](#cargo-features).

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- validate ./contract.json
```

The compile command emits JSON containing a canonical validated `contract` and an optional, identity-bound `source_info` sidecar. The Contract exposes a full `contract_id` and an actor-only `interface_id`; source spelling/comments are identified separately by `source_bundle_id`. That source identity covers only the raw source files and import edges. At an external trust boundary, `SourceInfo::try_from_raw` recompiles that bundle and requires every presented provenance field to match the compiler-derived sidecar.

See [architecture](docs/architecture.md) and the [Contract graph](docs/contract-graph.md) for the v1 model, constraints, and the explicitly deferred host-value ↔ Candid binary bridge. The byte-level identity algorithm is specified normatively in [canonicalization v1](docs/canonicalization-v1.md). See [release verification gates](docs/verification.md) for the checks required before declaring the format stable across implementations. See [performance benchmarks](docs/benchmarks.md) for reproducible comparisons with the pinned official Candid checker and for allocation measurements.

## Command-line interface

The `candid-core` binary requires the `filesystem-compiler` feature, which is on by default. It accepts exactly this grammar, and nothing else:

```text
candid-core compile <path> [--no-source-info]
candid-core validate <path>
```

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- compile ./service.did --no-source-info
cargo run --bin candid-core -- validate ./contract.json
```

Anything outside the grammar — an unknown command, an unknown option, a missing path, an option before the path, a flag on `validate`, a duplicate `--no-source-info`, or any trailing argument — is a usage error: the process writes nothing to stdout, exits with status 64, and prints exactly this usage text on stderr:

```text
usage: candid-core compile <path> [--no-source-info]
       candid-core validate <path>
```

A token that begins with `-` is always treated as an option in the path position, never as a path; spell a dash-leading relative file with a `./` prefix (`candid-core compile ./-service.did`). Path arguments are taken as OS-native bytes, so a non-Unicode path is never an argument-parsing failure: `validate` hands the bytes to the filesystem unchanged, while `compile` additionally requires the entry file name to be valid UTF-8 — it becomes the source ID — and reports a `did_invalid_source_id` diagnostic otherwise. There are no other commands or flags — in particular no flags for custom limits; pass custom `Limits` through the library APIs instead.

Every non-usage outcome is one pretty-printed JSON document on stdout with an empty stderr:

| Outcome | Exit status | stdout | stderr |
| --- | --- | --- | --- |
| success | 0 | JSON with `"ok": true` | empty |
| read, parse, validation, or resource-limit failure | 1 | JSON with `"ok": false` | empty |
| usage error | 64 | empty | the usage text above |

`compile <path>` emits `{"ok": true, "contract": …, "source_info": …}` on success and `{"ok": false, "diagnostics": […]}` on failure. `--no-source-info` suppresses the provenance sidecar; the key stays present as `"source_info": null`. `validate <path>` emits `{"ok": true, "contract": …}` on success, `{"ok": false, "diagnostics": […]}` when the document cannot be read or is not JSON, and `{"ok": false, "violations": […]}` when it parses but fails validation or when the read exceeds the input byte bound (a single `resource_limit_exceeded` violation at path `$`). Codes are stable identifiers such as `did_parse_error`, `did_file_read_error`, `contract_file_read_error`, `malformed_contract_json`, and `resource_limit_exceeded`.

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
cargo run --example hermetic_bundle         # filesystem-compiler
cargo run --example host_value_validation   # compiler + host-value
```

Each example declares its `required-features`, so `cargo run --example …` under a reduced feature set reports that the example was skipped rather than failing to compile.

`contract_walkthrough` prints a canonical recursive Contract and its provenance summary. `semantic_equivalence` compares interface identity with source identity. `trust_boundary` demonstrates rejection of injected metadata and a tampered identity. `hermetic_bundle` shows filesystem-free import resolution, while `host_value_validation` preserves a large `nat` and an IEEE NaN payload. `bounded_parsing` rejects oversized untrusted documents before decoding and shows the second limit serialization consumes.

## Foundation decisions

Six implemented [foundation ADRs](docs/adrs/README.md) define the boundaries for large-ecosystem use:

1. separate interface, Contract, and source-bundle identities;
2. independently version schema, Candid semantics, and canonical bytes;
3. make validated artifacts and provenance binding explicit;
4. resolve imports through a hermetic capability boundary;
5. bound all untrusted work; and
6. use a lossless tagged HostValue ABI.

All six decisions are implemented in the Rust reference runtime. Because the crate has not been released, this profile is the clean starting point rather than a compatibility layer over an earlier format.

## Rust version and dependencies

The crate advertises Rust 1.78 as its minimum supported Rust version (MSRV). Direct dependencies are pinned to versions that are expected to build on that toolchain, and dependency updates should preserve the advertised MSRV unless the `rust-version` field is intentionally raised in the same change. CI runs the locked dependency graph against Rust 1.78, so an incompatible direct or transitive dependency update fails before merge.

## Cargo features

`candid-core` stays one published package with one library and one binary. What it *builds* is split into an always-present base plus three features, all enabled by default:

| Feature | Adds | Dependencies it pulls in |
| --- | --- | --- |
| *(base)* | `Contract`, `ContractDraft`, `RawContract`, `ContractEnvelope`, validation, canonicalization, identities, `Limits`/`RuntimeContext`/`CancellationToken`, `Diagnostic` | `serde`, `serde_json`, `sha2`, `hex` |
| `host-value` | `HostValue`, `HostFieldValue`, `validate_host_value`, `ContractTypeRef`/`ContractMethodRef`, `Contract::bind_type`/`bind_method` | `ic_principal` |
| `compiler` | `compile_did` and its option/context variants, `Compilation`, `CompileOptions`, `CompileError`, `SourceId`/`SourceResolver`/`ResolvedSource`/`MemoryResolver`, `SourceInfo`/`RawSourceInfo` provenance | `candid`, `candid_parser` |
| `filesystem-compiler` (implies `compiler`) | `WorkspaceResolver`, `compile_did_file` and its variants, `compile_with_resolver`, the `candid-core` binary | `cap-std` |

Because every feature is on by default, an existing dependency needs no change:

```toml
# unchanged: the full surface, exactly as before
candid-core = "0.1"

# a pure Contract consumer: model, validation, canonicalization, identities
candid-core = { version = "0.1", default-features = false }

# ... plus the lossless tagged host value ABI
candid-core = { version = "0.1", default-features = false, features = ["host-value"] }

# a browser/WASM host that compiles self-contained DID source it already has
candid-core = { version = "0.1", default-features = false, features = ["compiler"] }

# a native tool that reads .did files, or uses the CLI
candid-core = { version = "0.1", default-features = false, features = ["filesystem-compiler"] }
```

Items outside the selected set are **absent at compile time**, not runtime stubs: a build error names the missing item, and turning on the feature it belongs to is the fix. `tests/model_public_api.rs` pins the root exports of each surface, and `tests/fixtures/packaging/verify_feature_graph.py` proves the dependency claims in the table above against `cargo metadata` — the base graph resolves to 23 packages where the default graph resolves to 125.

Three caveats, all deliberate:

- **Cargo unifies features across a build.** If anything else in your dependency graph depends on `candid-core` with defaults, the whole surface is compiled once for every consumer in that build. Feature selection bounds what a *dependency graph* must contain; it cannot subtract from a graph that already asked for more.
- **Feature selection does not shrink the published `.crate` archive.** Every source file ships regardless of which features a consumer enables. Bounding archive contents is separate release-hardening work.
- **`compile_with_resolver` needs `filesystem-compiler` even with an in-memory `MemoryResolver`**, because its current implementation materializes the resolved bundle into a private temporary directory for the authoritative import-aware checker. Whether imported bundles can be compiled without that step is [issue #21]'s subject; nothing here claims imported browser compilation.

Producer metadata is unaffected by any of this: `ProducerInfo::current` reports the same `name`, `version`, `candid_version`, and `candid_parser_version` in every configuration, because it reads the pinned versions from this package's manifest at compile time rather than from a linked crate. It remains **unauthenticated** provenance held outside the semantic identities — see the [identity ADR](docs/adrs/0001-contract-identities.md).

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
  filesystem and no import resolution; it is the entry point that stays
  available on `wasm32-unknown-unknown`.
- *(`compiler`)* `RawSourceInfo` → `SourceInfo::try_from_raw` recompiles the
  embedded source bundle and rejects any derived provenance that does not match
  exactly.
- *(`filesystem-compiler`)* `compile_with_resolver` compiles an immutable logical source bundle through `MemoryResolver` or sandboxed `WorkspaceResolver`; `compile_did_file` is the thin `WorkspaceResolver` adapter over it, and the `candid-core` binary is built on the same path.
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
