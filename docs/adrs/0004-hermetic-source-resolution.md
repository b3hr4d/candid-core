# ADR 0004: Resolve imports through a hermetic capability boundary

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

The current file compiler walks imports for provenance and then delegates to
`candid_parser::check_file`, which reads them again for semantic checking. This
creates two snapshots, ambient filesystem authority, host-specific paths, and
no central place to impose import policy. Those properties are unsuitable for
agents, browser/WASM consumers, reproducible builds, or hosted registries.

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

`SourceId` is a normalized logical URI, not an ambient absolute path. The
resolver produces one immutable `SourceBundle`; the authoritative Candid
checker and provenance collector consume that exact bundle. If the upstream
checker cannot consume virtual sources directly, the adapter may materialize
the bundle inside a controlled temporary root with verified import rewriting;
it may not reread the caller's workspace.

Logical source paths use a platform-independent UTF-8 `/` grammar. Empty path
segments, leading `/`, backslashes, colons, control characters, and Windows
drive syntax are rejected. `.` segments are removed and `..` removes one
preceding segment, but may not escape the logical root. These rules do not
perform percent-decoding or Unicode normalization. Only `WorkspaceResolver`
converts the normalized logical segments to a native filesystem path.
Schemes contain at least two ASCII characters, begin with a lowercase letter,
and otherwise contain only lowercase letters, digits, or `-`; the minimum
length distinguishes logical schemes from Windows drive prefixes.

The supported resolver profiles are:

- `MemoryResolver` for tests, editors, agents, and network-fetched bundles.
- `WorkspaceResolver` rooted at an explicitly authorized directory; absolute
  imports, parent escapes, and symlink escapes are rejected by default.
- Future content-addressed registry resolvers with integrity verification.

Resolution detects cycles and duplicate logical identities, records import
edges, applies ADR 0005 limits, and returns structured diagnostics. Network
access is never implicit in `contract-core` or `candid-frontend`.

## Consequences

- Compilation becomes reproducible and testable without a filesystem.
- The same source bytes explain the same semantic result.
- Hosts can present explicit filesystem/network permission prompts.
- Filesystem convenience remains available through an opt-in adapter.

## Implementation

`compile_did` remains the self-contained convenience. `compile_did_file` is a
thin `WorkspaceResolver` adapter, while `compile_with_resolver` is the platform
primitive. `MemoryResolver` and `WorkspaceResolver` produce one immutable
logical-URI bundle which is materialized into an isolated temporary root for
the authoritative checker.

## Required verification

- In-memory multi-file and diamond-import tests.
- Path traversal, absolute path, symlink escape, cycle, and duplicate-ID tests.
- A test that mutating workspace files after snapshot creation has no effect.
- Identical bundle IDs and Contracts across operating systems.
