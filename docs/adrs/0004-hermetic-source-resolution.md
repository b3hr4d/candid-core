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

`compile_did` remains the self-contained convenience. `compile_did_file` is a thin `WorkspaceResolver` adapter, while `compile_with_resolver` is the platform primitive. `MemoryResolver` and `WorkspaceResolver` each produce one immutable logical-URI bundle, and the authoritative Candid checker consumes that exact bundle.

Since issue #24 that split is also a Cargo feature boundary. `SourceId`,
`SourceResolver`, `ResolvedSource`, and `MemoryResolver` — the logical half,
which needs no host filesystem — are `compiler` surface, along with
`compile_did` and its option/context variants. `WorkspaceResolver`,
`compile_did_file` and its variants, materialization, the `cap-std` capability
crate, and the `candid-core` binary are `filesystem-compiler` surface.

Issue #21 moved `compile_with_resolver` to the `compiler` side, which is what
this ADR's "if the upstream checker cannot consume virtual sources directly"
escape clause was reserved for: it turns out it need not. The resolved bundle is
merged into one virtual program in memory through the public `candid_parser`
0.4.0 merged-program APIs (`IDLMergedProg::new`/`merge`/`decs`/`resolve_actor`)
and type-checked with `check_prog`, which is precisely the tail
`candid_parser::check_file` runs after it has read the files itself. No
materialization, no temporary root, no `cap-std`, no ambient authority, and no
fork of the upstream parser. That backend is exactly the one that already
authenticated a presented `SourceInfo`, so promoting it removed a second
implementation rather than adding one, and public compilation and provenance
rederivation cannot drift apart by construction.

`compile_did_file` deliberately did **not** move onto the promoted entry point.
Native file compilation keeps `candid_parser::check_file` over a materialized
copy of the bundle as its authority, and with it the filesystem diagnostic
mapping and the workspace snapshot semantics this ADR specifies. The two
backends are held to each other by `src/compile/differential.rs`, which requires
byte-identical Contracts, identities, and provenance for every valid bundle and
identical stable diagnostic codes, phases, and resource triples for invalid
ones.

The supported browser boundary is synchronous immutable source data supplied by
the host. Asynchronous resolution, network access, and registry semantics stay
out of scope here; the "future content-addressed registry resolvers" line above
remains future work.

On native hosts, `WorkspaceResolver` retains an open directory capability and opens every logical path relative to it. Relative symlinks are permitted only when their resolution remains beneath that capability; absolute symlinks and escapes are rejected. Authorization and reading use the same opened file handle so concurrent path replacement cannot substitute a file outside the workspace. `cap-std` is both target-conditional (`cfg(not(target_os = "unknown"))`) and feature-gated, so a browser-WASM graph never contains it and a `compiler`-only graph never contains it either. Hosts without the `filesystem-compiler` feature, and bare `wasm32-unknown-unknown` hosts regardless of features, have no workspace filesystem resolver and must use a memory or host-provided resolver.

## Required verification

- In-memory multi-file and diamond-import tests (`compiler`, since they route
  through `compile_with_resolver`, which no longer needs a filesystem).
- A differential suite comparing the in-memory backend against the materialized
  `check_file` backend on the same logical bundles — valid multi-file, diamond
  and repeated imports, type plus service inclusion, recursion, actorless and
  class actors, plus representative invalid merge and type-check cases
  (`src/compile/differential.rs`).
- Path traversal, absolute path, symlink escape, cycle, and duplicate-ID tests
  (`filesystem-compiler`).
- A test that mutating workspace files after snapshot creation has no effect
  (`filesystem-compiler`).
- Identical bundle IDs and Contracts across operating systems.
- A dependency-graph check that `cap-std` is absent from the base,
  `host-value`, and `compiler` graphs and from every browser-WASM graph
  (`tests/fixtures/packaging/verify_feature_graph.py`).
- Real browser execution of imported-bundle compilation, not only a
  `wasm32-unknown-unknown` build check: `tests/browser_wasm.rs` under
  `wasm-pack test --headless --chrome --chromedriver <driver>` with
  `--no-default-features --features compiler`, against an exactly pinned Chrome
  for Testing build and the ChromeDriver from that same build, pinning Contract
  identities and provenance and asserting that resolver, resource,
  cancellation, and deadline failures stay structured.
