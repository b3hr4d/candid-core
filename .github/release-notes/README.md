# Release notes

One file per released version, named `<version>.md` — for example
`0.1.0-beta.2.md`. The `Release` workflow reads the file matching the version
being released and uses it verbatim as the GitHub release body.

Notes live here rather than under `docs/` for two reasons. They are reviewed
before they are published: the file lands in the pull request that prepares the
release, so the wording, the disclosed limitations, and the deferred work get
the same review as code. And `.github/**` is outside the `include` allowlist in
`Cargo.toml`, so adding a file here never changes the published archive — a
release note is a statement *about* a release, not part of it.

The `guard` job fails if the file is missing or empty, before anything is
tagged or published. That check is deliberately early: a crates.io version can
never be republished, so discovering a missing release note afterwards cannot be
fixed for that version.

A note cannot state its own release commit or the resulting archive digest:
committing those values would change the commit, and therefore
`.cargo_vcs_info.json` and the digest, again. So the workflow appends a **Release
provenance** table — commit, Cargo version, `.crate` SHA-256, and crates.io URL —
from values that run has verified. Do not hand-write those into the note; they
would be stale by construction.

What the note itself is expected to state honestly, following
[`docs/releasing.md`](../../docs/releasing.md) step 5:

- installation syntax, naming the exact version — a caret requirement does not
  select a prerelease;
- pre-1.0 API, wire-format, canonical-byte, and identity instability;
- ADR verification status, without promoting an ADR that has no recorded run;
- known limitations and deferred work, each named with its issue number;
- what is irreversible about the publication.

`0.1.0-beta.1` predates this workflow and was released by hand, so it has no
file here; its notes are the body of
[its GitHub release](https://github.com/b3hr4d/candid-core/releases/tag/v0.1.0-beta.1).

## npm packages

The two npm packages keep their notes under `npm/<package>/<version>.md` —
`npm/schema/0.3.0-beta.1.md` for `@candid-core/schema`, `npm/cli/0.2.0-beta.1.md`
for `@candid-core/cli` — never at this directory's top level. The npm versions
are independent of the crate's, so a top-level `0.2.0-beta.1.md` written for the
CLI is exactly the file the `Release` workflow would read, and publish as the
crate's GitHub release body, if the crate ever released that number.

No workflow reads these files: the npm release workflows create no GitHub
release. They are the reviewed record of a release, landed in the version-bump
pull request like the crate's notes, and they carry what that pull request
reviewed: the installation lines, the pairing (the `candid-core` version, and
for the CLI the embedded revision and the schema peer), the public-API diff
against the previous release with every break marked, and the known
limitations.
