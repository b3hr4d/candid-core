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

What a release note is expected to state honestly, following
[`docs/releasing.md`](../../docs/releasing.md) step 5:

- the exact release commit, the `.crate` digest, and the Cargo that produced it;
- installation syntax, naming the exact version — a caret requirement does not
  select a prerelease;
- pre-1.0 API, wire-format, canonical-byte, and identity instability;
- ADR verification status, without promoting an ADR that has no recorded run;
- known limitations and deferred work, each named with its issue number;
- what is irreversible about the publication.

`0.1.0-beta.1` predates this workflow and was released by hand, so it has no
file here; its notes are the body of
[its GitHub release](https://github.com/b3hr4d/candid-core/releases/tag/v0.1.0-beta.1).
