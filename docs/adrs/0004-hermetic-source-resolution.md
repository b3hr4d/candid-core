# ADR 0004: Resolve imports through a hermetic capability boundary

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The current file compiler walks imports for provenance and then delegates to `candid_parser::check_file`, which reads them again for semantic checking. This creates two snapshots, ambient filesystem authority, host-specific paths, and no central place to impose import policy. Those properties are unsuitable for agents, browser/WASM consumers, reproducible builds, or hosted registries.

## Decision

Compilation with imports will require an explicit resolver capability:

```rust
trait SourceResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str)
        -> Result<SourceId, ResolveError>;
    fn load(&self, id: &SourceId, limits: &Limits)
        -> Result<ResolvedSource, ResolveError>;
}

struct ResolvedSource {
    id: SourceId,
    bytes: Vec<u8>,
    digest: SourceDigest,
}
```

`SourceId` is a normalized logical URI, not an ambient absolute path. The resolver produces one immutable `SourceBundle`; the authoritative Candid checker and provenance collector consume that exact bundle. If the upstream checker cannot consume virtual sources directly, the adapter may materialize the bundle inside a controlled temporary root with verified import rewriting; it may not reread the caller's workspace.

Logical source paths use a platform-independent UTF-8 `/` grammar. Empty path segments, leading `/`, backslashes, colons, control characters, and Windows drive syntax are rejected. `.` segments are removed and `..` removes one preceding segment, but may not escape the logical root. These rules do not perform percent-decoding or Unicode normalization. Only `WorkspaceResolver` converts the normalized logical segments to a native filesystem path. Schemes contain at least two ASCII characters, begin with a lowercase letter, and otherwise contain only lowercase letters, digits, or `-`; the minimum length distinguishes logical schemes from Windows drive prefixes.

The supported resolver profiles are:

- `MemoryResolver` for tests, editors, agents, and network-fetched bundles (`compiler` feature).
- `WorkspaceResolver` rooted at an explicitly authorized directory; absolute imports, parent escapes, and symlink escapes are rejected by default (`filesystem-compiler` feature).
- Future content-addressed registry resolvers with integrity verification.

Resolution detects cycles and duplicate logical identities, records import edges, applies ADR 0005 limits, and returns structured diagnostics. Network access is never implicit in `contract-core` or `candid-frontend`.

## Consequences

- Compilation becomes reproducible and testable without a filesystem.
- The same source bytes explain the same semantic result.
- Hosts can present explicit filesystem/network permission prompts.
- Filesystem convenience remains available through an opt-in adapter — opt-in
  at two levels since issue #24: a different function to call, and the
  `filesystem-compiler` Cargo feature that supplies it at all.

## Implementation

`compile_did` remains the self-contained convenience. `compile_did_file` is a thin `WorkspaceResolver` adapter, while `compile_with_resolver` is the platform primitive. `MemoryResolver` and `WorkspaceResolver` produce one immutable logical-URI bundle which is materialized into an isolated temporary root for the authoritative checker.

Since issue #24 that split is also a Cargo feature boundary. `SourceId`,
`SourceResolver`, `ResolvedSource`, and `MemoryResolver` — the logical half,
which needs no host filesystem — are `compiler` surface, along with
`compile_did` and its option/context variants. `WorkspaceResolver`,
`compile_did_file` and its variants, `compile_with_resolver`, materialization,
the `cap-std` capability crate, and the `candid-core` binary are
`filesystem-compiler` surface. `compile_with_resolver` sits on the filesystem
side even for a purely in-memory `MemoryResolver`, because its current
implementation materializes the resolved bundle for `candid_parser::check_file`;
whether imported bundles can be checked without that step is issue #21's
subject, and nothing in issue #24 claims imported browser compilation. The
in-memory merged-program reconstruction used to authenticate a presented
`SourceInfo` is unchanged and stays available under `compiler` alone; it is an
internal provenance path, not a promoted compilation entry point.

On native hosts, `WorkspaceResolver` retains an open directory capability and opens every logical path relative to it. Relative symlinks are permitted only when their resolution remains beneath that capability; absolute symlinks and escapes are rejected. Authorization and reading use the same opened file handle so concurrent path replacement cannot substitute a file outside the workspace. `cap-std` is both target-conditional (`cfg(not(target_os = "unknown"))`) and feature-gated, so a browser-WASM graph never contains it and a `compiler`-only graph never contains it either. Hosts without the `filesystem-compiler` feature, and bare `wasm32-unknown-unknown` hosts regardless of features, have no workspace filesystem resolver and must use a memory or host-provided resolver.

## Required verification

- In-memory multi-file and diamond-import tests (`filesystem-compiler`, since
  they route through `compile_with_resolver`).
- Path traversal, absolute path, symlink escape, cycle, and duplicate-ID tests
  (`filesystem-compiler`).
- A test that mutating workspace files after snapshot creation has no effect
  (`filesystem-compiler`).
- Identical bundle IDs and Contracts across operating systems.
- A dependency-graph check that `cap-std` is absent from the base,
  `host-value`, and `compiler` graphs and from every browser-WASM graph
  (`tests/fixtures/packaging/verify_feature_graph.py`).
