# Candid Core

An early, deliberately narrow runtime foundation for turning Candid DID files into a canonical validated Contract graph. The Rust core delegates parsing and type checking to the official `candid_parser` implementation; consumers never need to parse Candid source or reproduce its type rules.

```sh
cargo run --bin candid-core -- compile ./service.did
cargo run --bin candid-core -- validate ./contract.json
```

The compile command emits JSON containing a canonical validated `contract` and an optional, identity-bound `source_info` sidecar. The Contract exposes a full `contract_id` and an actor-only `interface_id`; source spelling/comments are identified separately by `source_bundle_id`. That source identity covers only the raw source files and import edges. At an external trust boundary, `SourceInfo::try_from_raw` recompiles that bundle and requires every presented provenance field to match the compiler-derived sidecar.

See [architecture](docs/architecture.md) and the [Contract graph](docs/contract-graph.md) for the v1 model, constraints, and the explicitly deferred host-value ↔ Candid binary bridge. The byte-level identity algorithm is specified normatively in [canonicalization v1](docs/canonicalization-v1.md). See [release verification gates](docs/verification.md) for the checks required before declaring the format stable across implementations. See [performance benchmarks](docs/benchmarks.md) for reproducible comparisons with the pinned official Candid checker and for allocation measurements.

## Command-line interface

The `candid-core` binary accepts exactly this grammar, and nothing else:

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

Both commands bound their reads under `Limits::default()` before decoding, so an oversized file fails with a `resource_limit_exceeded` code carrying `{resource, limit, observed}` metadata instead of allocating without bound, and the byte bound takes precedence over UTF-8 and parse errors:

- `compile` reads every source file through the workspace resolver: each file is bounded by `max_source_bytes` (1 MiB, resource `source_bytes`), and the import bundle in aggregate by `max_bundle_bytes` (8 MiB, resource `bundle_bytes`); an over-limit source fails in the `diagnostics` shape.
- `validate` reads the contract document bounded by `max_input_bytes` (4 MiB, resource `input_bytes`); an over-limit document fails in the `violations` shape.

The real-binary suite in `tests/cli.rs` pins this contract: the argument matrix, exit statuses, output channels, JSON shapes and codes, source-info suppression, and both byte bounds at the exact limit and one byte over.

## Runnable examples

The examples show why the Contract is a graph, how semantically equivalent DID sources share an identity, and how strict JSON validation protects the core:

```sh
cargo run --example contract_walkthrough
cargo run --example semantic_equivalence
cargo run --example trust_boundary
cargo run --example hermetic_bundle
cargo run --example host_value_validation
cargo run --example bounded_parsing
```

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

## Platform APIs

- `RawContract` → `Contract::try_from_raw` validates an external artifact.
- `Contract::build_raw` is the producer path that calculates fresh identities.
- `RawSourceInfo` → `SourceInfo::try_from_raw` recompiles the embedded source
  bundle and rejects any derived provenance that does not match exactly.
- `compile_with_resolver` compiles an immutable logical source bundle through `MemoryResolver` or sandboxed `WorkspaceResolver`.
- `Limits` and constructor-based `RuntimeContext` bound untrusted compilation
  and validation with one shared budget, monotonic deadlines, and cooperative
  `CancellationToken` support.
- `HostValue` plus `validate_host_value` provide the lossless tagged value ABI.
- `ContractEnvelope` keeps namespaced extensions outside the strict core.

## Bounded parsing and trusted serde integration

Untrusted bytes and already-validated values take different paths, and the crate does not let the two be confused.

- `Contract`, `Compilation`, `ContractEnvelope`, and `HostValue` do not implement `Deserialize`: a trait impl has no argument position for a resource policy, so it could only ever decode under limits the library chose.
- Untrusted Contract, Compilation, and envelope JSON goes through `from_json_with_limits`/`from_json_with_context` and `from_slice_with_limits`/`from_slice_with_context`. These enforce `max_input_bytes` before decoding and then share one budget with validation. `Contract::from_json` is the same path under `Limits::default`. The byte gate bounds peak decode allocation to a multiple of the caller's ceiling; it does not reject element by element during decode, which remains a follow-up.
- `HostValue` is the exception to that sentence: `HostValue::from_json_with_limits`/`from_json_with_context` gate on `max_value_bytes`, not `max_input_bytes`, and report `HostValueJsonError::Limit`, which carries no `resource` name. Lowering `max_input_bytes` alone does not bound HostValue decoding — lower `max_value_bytes` too.
- `Serialize` and the derived `Deserialize` on the raw DTOs (`RawContract`, `RawSourceInfo`) are the trusted serde integration: they consult no limits and revalidate nothing. Decoding a raw DTO is not a bounded operation, so callers must gate byte length themselves or use a bounded parse API.
- The `to_json_pretty_with_limits`/`to_json_pretty_with_context` serializers on `Contract`, `Compilation`, and `ContractEnvelope` validate before rendering and charge the rendered length against `max_canonicalization_work`. That is a second budget: raising a structural limit such as `max_string_bytes` to build a value does not by itself make that value renderable. (`Compilation` validates its Contract, not its already-authenticated sidecar; rederiving provenance is construction-time work.)

`HostValue` is the crate's one recursive value type, and it is bounded on both sides.

- **Decoding** runs a constant-stack scan of the JSON text before `serde_json` sees it, rejecting anything nested deeper than `max_value_nesting` as a `value_nesting` resource limit. That check counts JSON containers, the same unit serde_json counts, so keeping the limit below serde_json's fixed 128-frame ceiling means the crate-owned check is always the one that fires — with `{resource, limit, observed}` metadata instead of a serde string. The ceiling itself is left in place and unmodified underneath. Raising `max_value_nesting` above 128 therefore has no effect.
- **Construction** is bounded by `max_value_depth` and `max_value_elements`: `HostValue::opt`, `vector`, `record`, and `variant` take a `&Limits` and fail closed. This is what keeps the recursive operations on the type safe, since `Drop`, `Clone`, `PartialEq`, `Debug`, and `Serialize` all walk one stack frame per level and none of the first four can report an error.

Lexical nesting and semantic depth are deliberately separate limits, as they are for source (`max_source_nesting`) and types (`max_type_depth`): one `vec` level costs two JSON containers and one `record` level costs three, so a single limit could not report an honest `observed` value for both.
