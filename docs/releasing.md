# Releasing candid-core

This is the exact procedure for turning a commit into a published version. It
holds no stored crates.io credential anywhere: the publish step exchanges a
GitHub OIDC identity for a token that expires shortly and is revoked when the
job ends, so there is no long-lived token in Actions secrets and none on an
operator's workstation either.

Steps 1–6 are read-only with respect to the outside world, and every workflow
that runs on a pull request — `Verify` and `Release candidate` — holds no more
than `contents: read` and is unable to tag, publish, or release. The three steps
that mutate anything outside this repository — creating a tag, publishing to
crates.io, and creating a GitHub release — are gathered at the end under
[Authorized mutation](#7-authorized-mutation). They live in a separate
`workflow_dispatch`-only workflow, and each one waits for its own explicit human
approval after the evidence in steps 1–6 exists.

Read [§8 Irreversibility](#8-irreversibility-yanking-and-rollback) **before**
the first publish. A crates.io version cannot be deleted, and yanking is not
deletion.

Tool versions are pinned in exactly one place:
[`tests/fixtures/packaging/release-tools.env`](../tests/fixtures/packaging/release-tools.env).
Every command below and the `Release candidate` workflow read that file, so a
local run and a CI run cannot disagree about what was executed. Source it first:

```sh
. tests/fixtures/packaging/release-tools.env
rustup toolchain install "${RELEASE_TOOLCHAIN}"
```

`RELEASE_TOOLCHAIN` is the exact Cargo that packages and publishes. It is pinned
rather than left as `stable` because `cargo package` is byte-stable within one
Cargo version and not across versions: this tree at one commit packaged by
1.91.1 and by 1.94.1 unpacks identically and yields different `.crate` digests.
`cargo publish` re-packages rather than uploading an archive you hand it, so if
the operator's Cargo differs from CI's, the recorded SHA-256 describes an
archive nobody published. Use `cargo "+${RELEASE_TOOLCHAIN}"` for every
packaging and publishing command below.

## 1. Prepare a release candidate from a clean, exact commit

A release is identified by a commit, not by a branch name. Everything that
follows must run against one commit with nothing uncommitted, because the
archive's SHA-256 is only meaningful if the tree that produced it is pinned.

```sh
git switch main
git pull --ff-only
git status --porcelain          # must be empty
git rev-parse HEAD              # record this; it is the release commit
```

If `git status --porcelain` is not empty, stop. An untracked scratch directory
does not merely make `cargo package` refuse to run — it is a directory that a
default Cargo configuration would have published. The `include` allowlist in
`Cargo.toml` is what prevents that, and
`tests/fixtures/packaging/verify_package_manifest.py` is what proves it, but
neither is a reason to package a dirty tree.

Then confirm the version is the one being released, in all three places it is
recorded:

```sh
grep '^version' Cargo.toml
grep -A1 '^name = "candid-core"$' Cargo.lock
grep -A1 '^name = "candid-core"$' fuzz/Cargo.lock
```

`tests/release_metadata.rs` pins the expected version as a literal, so a bump
that forgets a lockfile fails the suite rather than surfacing as a mismatched
`ProducerInfo` in a consumer's build.

## 2. Run the full Verify matrix and the release gates

Everything the `Verify` workflow runs, plus everything the `Release candidate`
workflow runs. Both must be green on the release commit. Locally:

```sh
cargo fmt --check
git diff --check

# Debug feature matrix — every supported combination.
cargo test --all-targets --locked
cargo test --all-targets --locked --no-default-features
cargo test --all-targets --locked --no-default-features --features host-value
cargo test --all-targets --locked --no-default-features --features compiler
cargo test --all-targets --locked --no-default-features --features compiler,host-value
cargo test --all-targets --locked --no-default-features --features filesystem-compiler
cargo test --all-targets --locked --all-features

# Lints, with warnings denied, across the supported combinations.
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --locked --no-default-features -- -D warnings
cargo clippy --all-targets --locked --no-default-features --features host-value -- -D warnings
cargo clippy --all-targets --locked --no-default-features --features compiler -- -D warnings
cargo clippy --all-targets --locked --no-default-features --features filesystem-compiler -- -D warnings

# Release profile, and the advertised MSRV.
cargo test --release --all-targets --locked
cargo +1.78.0 test --all-targets --locked

# Doctests and rustdoc, reduced and full surfaces, warnings denied.
cargo test --doc --locked --no-default-features
cargo test --doc --locked --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --no-default-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --all-features

# WASM build checks and the browser runtime suite.
cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features
cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features compiler
cargo check --lib --target wasm32-unknown-unknown --locked
cargo clippy --lib --target wasm32-unknown-unknown --locked --no-default-features --features compiler -- -D warnings
wasm-pack test --headless --chrome --chromedriver "$CHROMEDRIVER_PATH" -- --locked --test browser_wasm --no-default-features --features compiler

# Benchmarks compile and run once; no wall-clock threshold is enforced (#39).
cargo bench --benches --locked -- --test

# Fuzzing: lockfile freshness, every target builds, tracked corpora replay.
cargo metadata --manifest-path fuzz/Cargo.toml --locked --format-version 1 > /dev/null
cargo +nightly fuzz build --dev
for target in $(cargo +nightly fuzz list); do
  cargo +nightly fuzz run --dev "$target" \
    "fuzz/corpus/$target" "fuzz/seeds/$target" "fuzz/regressions/$target" -- -runs=0
done

# The two independent references. Neither calls Rust.
python3 tests/fixtures/conformance/verify_vectors.py
python3 tests/fixtures/artifact-identity/verify_artifact_ids.py

# Feature dependency boundaries.
python3 tests/fixtures/packaging/verify_feature_graph.py
```

The full list of gates and what each one is evidence *for* is in
[verification.md](verification.md).

## 3. Inspect the archive: manifest, size, SHA-256, unpacked source

Build the archive and look at it. Do not skip to the dry run — the dry run
succeeding tells you the archive compiles, not that it contains the right files.

```sh
cargo "+${RELEASE_TOOLCHAIN}" --version                # must be ${RELEASE_TOOLCHAIN}
cargo "+${RELEASE_TOOLCHAIN}" package --locked

# What is in it, and nothing else.
python3 tests/fixtures/packaging/verify_package_manifest.py --locked
cargo "+${RELEASE_TOOLCHAIN}" package --list --locked | sort

# How big, and exactly which bytes.
ls -l target/package/candid-core-*.crate
shasum -a 256 target/package/candid-core-*.crate     # sha256sum -b on Linux

# The unpacked source, read as a consumer would receive it.
tar -tzf target/package/candid-core-*.crate | sort
tar -xzf target/package/candid-core-*.crate -C "$(mktemp -d)"
```

Record the byte size and the SHA-256 in the evidence table (step 6), together
with the Cargo version that produced them. The digest identifies an archive only
in combination with both the commit and the Cargo: change either and the bytes
change even though the unpacked source does not. The digest belongs in a release
record, never committed to the repository — the next commit changes
`.cargo_vcs_info.json` and invalidates it, so a committed checksum is a stale
checksum by construction.

Read the unpacked tree, not just the file list. Three things to confirm by eye:

- The normalized `Cargo.toml` still declares each `candid`/`candid_parser`
  dependency with an exact `version = "=X.Y.Z"`. `ProducerInfo::current` reads
  that text at compile time through `CARGO_MANIFEST_DIR`, and it panics if the
  pin is not there in a spelling it understands.
- `src/bounded.rs` is present. The binary reaches it through a `#[path]`
  attribute, so no `mod` declaration in `lib.rs` would reveal its absence.
- Cargo has dropped the `[[test]]` and `[[bench]]` sections, because the
  allowlist excludes their files. If they are still there, a consumer's manifest
  parse fails on a missing target file.

## 4. Verify the dry run and clean consumers built from the archive

```sh
cargo "+${RELEASE_TOOLCHAIN}" publish --dry-run --locked
bash tests/fixtures/packaging/verify_packaged_consumers.sh
```

`verify_packaged_consumers.sh` is the check a repository-relative test cannot
make. It unpacks the archive into a fresh temporary directory, refuses to run if
that directory is inside the repository, and then builds six external consumer
crates plus an installed CLI against it:

| Consumer | Feature selection |
| --- | --- |
| base | `default-features = false` |
| compiler | `default-features = false, features = ["compiler"]` |
| all-features | `["compiler", "filesystem-compiler", "host-value"]` |
| CLI | `cargo install --locked --path <unpacked> --no-default-features --features filesystem-compiler` |
| wasm base | `default-features = false`, `--target wasm32-unknown-unknown` |
| wasm compiler | `default-features = false, features = ["compiler"]`, `--target wasm32-unknown-unknown` |

It also runs `cargo tree` over the base consumer and fails if `candid`,
`candid_parser`, `cap-std`, or `ic_principal` appears — the feature boundary has
to hold in the *published* manifest, not only in this repository's. The CLI smoke
writes its own `.did` file, compiles it, extracts the Contract, and validates
that, because every fixture under `tests/` is deliberately outside the archive.

## 5. Review the changelog, migrations, limitations, and public API

- [`CHANGELOG.md`](../CHANGELOG.md) has an entry for the version being released,
  and it states the pre-1.0 API and wire instability, the migrations, and the
  known limitations honestly. Deferred work is named with its issue number
  rather than omitted. Between releases, user-visible changes that are on
  `main` but in no crate release accumulate in an `## Unreleased` section at
  the top of that file; release prep renames it to the heading of the version
  being released, as it does for the npm changelogs' ([every publish after
  that](#every-publish-after-that)), leaving no `## Unreleased` section behind.
  One gap is open: `verify_semver.py` looks for a `**BREAKING**`
  acknowledgement only under `## Unreleased`, and it compares against the
  last crates.io release, so on a release commit whose changelog has been
  renamed it finds no acknowledgement. A release that carries an acknowledged
  break therefore cannot pass that gate on its release commit until the gate
  also reads the section of the version being released.
- ADR status in [verification.md](verification.md) matches reality. An ADR is
  **Verified** only when every gate in its required-verification list has a
  *recorded run*. Wiring a CI job is not evidence that it ran.
- The public API inventory is current:

  ```sh
  bash tests/fixtures/packaging/verify_public_api.sh
  ```

  This fails on unreviewed drift. If the drift is intended, re-run with
  `--write` and commit the regenerated snapshot alongside the change that caused
  it — that commit is the review.

- The README's installation examples name the version being released. A
  prerelease is not selected by a caret requirement: `"0.1"` will not resolve to
  a prerelease, so the examples pin it exactly, `=<version>`.

## 6. Present the evidence

Fill in [the evidence template in verification.md](verification.md#release-candidate-evidence-template)
and present it for review *before* asking for authorization. Values that do not
exist yet are marked `pending`, never guessed. In particular: do not write a PR
number, a CI run URL, a merge commit, or a crates.io URL until it exists.

## 7. Authorized mutation

Everything above is read-only with respect to the outside world. Everything
below is not. The three mutations — the tag, the crates.io publish, and the
GitHub release — are performed by the `Release` workflow
([`.github/workflows/release.yml`](../.github/workflows/release.yml)), and each
one requires a separate explicit approval from the repository owner, given after
seeing the step 6 evidence.

Automating these steps did not move the authorization; it moved *where the
authorization is recorded*. Each mutation runs in its own GitHub Environment
with required reviewers, so the run pauses until a human approves that specific
job. Approving the tag does not approve the publish, and approving the publish
does not approve the release.

### Prerequisites, configured once

- **Trusted Publishing.** Configure this repository and `release.yml` as a
  trusted publisher for `candid-core` in the crates.io UI. The workflow holds no
  crates.io token: `rust-lang/crates-io-auth-action` exchanges the workflow's
  OIDC identity for a token that expires shortly and is revoked when the job
  ends. **Do not add a crates.io token to Actions secrets.** The `Release
  candidate` workflow runs on every pull request and is designed to be unable to
  publish; a stored token would remove that property, and Trusted Publishing
  means no such token needs to exist anywhere — including on an operator's
  workstation, which is where the credential used to live.
  Trusted Publishing cannot be configured for a crate that does not exist, which
  is why `0.1.0-beta.1` was published by hand.
- **Three protected environments** — `release-tag`, `crates-io`, and
  `github-release` — each with required reviewers. Without required reviewers
  the environments are not gates and the three approvals collapse into one.
- **A release note.** Write `.github/release-notes/<version>.md` and land it in
  the pull request that prepares the release, so its wording is reviewed. See
  [the convention](../.github/release-notes/README.md).

### Running it

Dispatch `Release` with the release commit, the version, and the `.crate`
SHA-256 recorded by the `Release candidate` run for that commit:

```sh
gh workflow run release.yml \
  --field commit=<40-character release commit SHA> \
  --field version=<version> \
  --field expected_sha256=<.crate SHA-256 from step 3>
```

The `guard` job runs before any approval is requested and fails closed on: a
commit that is not a full SHA or not reachable from `main`, a version that does
not match `Cargo.toml`, a tag that already exists, a Cargo other than
`RELEASE_TOOLCHAIN`, a missing release note, a package-manifest violation, and —
the check this procedure could previously only ask a human to perform by eye — a
repackaged archive whose digest differs from `expected_sha256`.

Then approve, in order:

1. **`release-tag`** — creates an annotated `v<version>` on the input commit,
   never on `HEAD`, and verifies the pushed tag is annotated and resolves to
   that commit.
2. **`crates-io`** — publishes with `RELEASE_TOOLCHAIN`. The toolchain is not
   optional: `cargo publish` builds its own archive rather than uploading the one
   step 3 measured, so publishing with a different Cargo uploads different bytes
   and silently detaches the recorded digest from the artifact.
3. **`github-release`** — creates the release from the tag with the reviewed
   notes, marked as a prerelease whenever the version carries a semver
   prerelease suffix. The flag is derived from the version string rather than
   supplied, so a typo cannot publish a beta as a stable release.

Between steps 2 and 3 the `confirm` job reads crates.io's own recorded checksum
back and compares it with `expected_sha256`. That value, not the locally
computed one, is what a consumer's `Cargo.lock` carries. If it differs, the
release gates measured an archive that was not published: say so in the release
record and in `CHANGELOG.md`, and treat it as a defect in this procedure rather
than a discrepancy to reconcile by hand. The version cannot be re-published
(§8), so the correction is a follow-up version.

The job order is the same order a human would use, and for the same reason: a
tag without a publish is recoverable; a publish without a tag leaves a version
on crates.io that no commit in the repository is identified with.

### Doing it by hand

The workflow is the supported path. If it is unavailable, the equivalent manual
commands are `git tag -a v<version> -m "candid-core <version>" <commit>` and
`git push origin refs/tags/v<version>`; `cargo "+${RELEASE_TOOLCHAIN}" publish
--locked` with the operator authenticating interactively; the checksum read-back
above; and `gh release create v<version> --verify-tag --prerelease`. Record that
the manual path was used and why.

## 8. Irreversibility, yanking, and rollback

**A published crates.io version can never be deleted or replaced.** The name,
the version number, and the exact archive bytes are permanent. There is no
force-push equivalent. This is the single most important sentence in this
document, and it is why steps 3 and 4 exist.

What *is* available:

- **Yanking** — `cargo yank --version <version>` — marks a version so that new
  dependency resolution will not pick it. It does **not** delete the archive, it
  does **not** remove it from the index, and it does **not** break builds that
  already have it in a `Cargo.lock`. Anyone who pins the version explicitly, as
  every consumer of this prerelease is instructed to, still resolves it. Yanking
  is a "stop new adoption" signal, not a recall.
- **Un-yanking** — `cargo yank --version <version> --undo` — reverses the mark.
- **A follow-up version.** This is the actual fix for a bad release. Publish
  the next version with the correction — the prerelease after the bad one, or
  the next patch once out of prerelease — record the reason in `CHANGELOG.md`,
  and yank the bad version so new consumers do not find it. Never attempt to re-publish the same version number; crates.io
  rejects it, and if it did not, it would silently change what a pinned
  requirement means.

If a release has to be withdrawn:

1. Yank the version and say so in `CHANGELOG.md`, with the reason.
2. Open an issue describing what was wrong and what the correcting version will
   change.
3. Prepare the follow-up version through this same document from step 1.
4. Leave the tag and the GitHub release in place, editing the release body to
   point at the correction. Deleting a tag that consumers may have fetched
   creates a worse problem than the one being fixed.

For a prerelease specifically: because `"0.1"` does not select any
prerelease, a broken beta reaches only consumers who asked for it by exact
version. That narrows the blast radius; it does not remove the permanence.


## npm: `@candid-core/schema`

npm names and versions are as permanent as crates.io's; the same three
properties hold. **Prerequisites, once, all three required before the first dispatch:**

1. Register the `@candid-core` npm organization — scope ownership is the
   anti-squat measure.
2. Enable OIDC trusted publishing for this repository on the package; no
   token is created, stored, or reachable from any PR-triggered workflow.
3. **Create the `npm-publish` environment with required reviewers.** GitHub
   creates a referenced-but-missing environment at run time with *no*
   protection rules, so without this step the "protected" publish job would
   run unapproved — the same failure mode the crates.io section warns about.
   The workflow's verify job checks this and refuses to proceed, so the
   omission fails closed rather than publishing silently.

### The first publish of a new package is manual, once

npm has no "pending publisher": a trusted publisher can only be attached to
a package that **already exists** on the registry (`npm trust` documents it
as a prerequisite — "the package you're configuring must already exist" —
and npm/cli#8544, open since September 2025, is the request to lift this).
So the workflow cannot perform a package's *first* publish, and it will fail
with an auth or 404 error if pointed at a name that has never been published.

The bootstrap, done once per package name by the owner, from a local
checkout of the merged commit:

```bash
cd crates/candid-core-ts/ts
npm ci                                          # the pinned compiler, not a global one
npm login                                       # interactive, 2FA
npm version 0.0.0-bootstrap --no-git-tag-version
npm run build
npm publish --access public --tag bootstrap     # never becomes `latest`
npm logout                                      # invalidate the credential login created
git checkout package.json package-lock.json     # discard the local version bump
```

`npm ci` comes first because `npm run build` resolves `tsc` from
`node_modules/.bin`: without it, a machine with a global TypeScript silently
builds the permanent artifact with the wrong compiler, and a machine without
one just fails. `npm version` updates the manifest and the lockfile
together, so they stay in sync and `npm ci` remains valid either side of the
bump.

`--tag bootstrap` is what keeps the placeholder inert: `latest` still moves
to the first real version, and a plain `npm i` never resolves to it. The
version bump is deliberately local and discarded — the repository's manifest
stays at the version the workflow will verify.

`npm login` **does** create a credential: it writes a registry token into
`~/.npmrc`, which is why `npm logout` follows immediately — that invalidates
the token rather than leaving a publish credential on the workstation. This
is the one moment in the whole pipeline where a publishing credential
exists; it lasts for the two commands between login and logout, and after
that the OIDC-only posture holds with nothing to revoke.

Then configure the trusted publisher (npmjs.com → the package → Settings →
Trusted publishing → GitHub Actions), with values that must match this
repository exactly:

| Field | Value |
| --- | --- |
| Organization or user | `b3hr4d` |
| Repository | `candid-core` |
| Workflow filename | `npm-release.yml` |
| Environment | `npm-publish` |
| Allowed actions | publish (at least one is required since 2026-05-20) |

npm does not validate this configuration when saved — a typo surfaces only
as a failed publish. After the first successful workflow publish, harden the
package: Settings → Publishing access → require two-factor authentication
and disallow tokens. That setting affects token auth only; trusted
publishing keeps working, because OIDC is not a token.

### Every publish after that

Publishing is `npm release` (`.github/workflows/npm-release.yml`),
dispatch-only, with `{commit, version, dist-tag}` and an optional
`stable-off-latest`:

```bash
# A stable version: it becomes what a plain `npm i` installs.
gh workflow run npm-release.yml \
  --field commit=<40-character SHA> --field version=0.3.0 --field dist-tag=latest

# A prerelease: it lands under its own dist-tag, and consumers opt in with
# `npm i @candid-core/schema@beta`. `latest` does not move.
gh workflow run npm-release.yml \
  --field commit=<40-character SHA> --field version=0.3.0-beta.1 --field dist-tag=beta
```

1. The `verify` job shape-checks the inputs (`commit` must be a full
   40-character SHA — a branch name could resolve differently after the
   approval pause — and it must be reachable from `origin/main`, so
   unreviewed bytes cannot be published; `dist-tag` must be lowercase
   letters, digits and dashes) and checks the dist-tag against the version:
   a prerelease (a version with a `-` suffix, derived exactly as `release.yml`
   derives its prerelease flag) is refused under `latest`, and a stable
   version is refused under any other tag unless `stable-off-latest=true`
   says so explicitly, which is for a backport that must not move `latest`.
   `stable-off-latest=true` with a prerelease or with `latest` is refused
   too, so the flag cannot sit on unnoticed. It then confirms the `npm-publish`
   environment is genuinely protected, refuses a `version` input that
   differs from `ts/package.json` at that commit, then reruns the type gate,
   the runtime suites, and the packaged-consumer verification
   (`tests/fixtures/packaging/verify_npm_package.py` — the artifact `npm
   pack` produces must ship exactly the promised files, compile under strict
   TypeScript *without* `skipLibCheck`, and execute standalone).
2. The `publish` job sits behind the protected `npm-publish` environment;
   the owner's approval there is the explicit authorization for the
   irreversible step. It publishes with `--provenance` and
   `--tag <dist-tag>`, always explicit.

The pinned Node (24.18.0) ships npm 11.16.0, above both floors that matter:
11.5.1 for OIDC publishing and 11.15.0 for the `npm trust` CLI. That npm
refuses to publish a prerelease without an explicit `--tag` ("You must
specify a tag using --tag when publishing a prerelease version."), but it
accepts a prerelease with an explicit `--tag latest`. So npm's own check
would stop a tagless beta only in the publish job, after the approval, and
would not stop a beta tagged `latest` at all. The verify job's dist-tag
check is what covers both cases, before the approval is requested. Trusted
publishing also requires a GitHub-hosted runner and, for automatic
provenance, a public repository publishing a public package — all true here.

The package has no runtime dependency and no peer: it exports exactly `.`,
`./validate`, `./contract`, and `./codec`, and no shipped module imports
`@icp-sdk/core`. The smoke asserts all of that — the export map and the
absence of any dependency field, a consumer of every subpath compiling and
executing with no `@icp-sdk/core` installed, and each subpath 0.2.0 exported
but this package no longer does (`./actor`, `./transport-icp`, `./forms`,
`./labels`) failing to resolve, at runtime with
`ERR_PACKAGE_PATH_NOT_EXPORTED` and at compile time with `TS2307`.

The package versions independently of the crate (pre-1.0). Bump
`ts/package.json` in an ordinary reviewed PR; state in that PR's body which
`candid-core` generator version the release pairs with, and record the pair
in the npm release notes, `.github/release-notes/npm/schema/<version>.md`
(the convention, and why npm notes never sit at that directory's top level,
is in [its README](../.github/release-notes/README.md)).

Between releases, changes accumulate in an `## Unreleased` section at the
top of `ts/CHANGELOG.md`. Release prep, in that same version-bump PR, renames
it to `## <version> — <YYYY-MM-DD>` and adds the pairing line, leaving no
`## Unreleased` section behind. The rename is what re-arms the packaged-
artifact gate's measured-number check: while an `## Unreleased` section
exists, `verify_npm_package.py` treats the tree as development past the
packed version and does not compare figures the released entry states (such
as the `schema.d.ts` line count) against the artifact, because that entry
measured the artifact it shipped, not the one a later PR packs. Once the
section is renamed, every figure the entry being released states must match
the artifact exactly, and the gate refuses a stale one.

## npm: `@candid-core/cli`

The second package under the scope (name recorded with owner sign-off on
issue #153, per the #106 precedent), publishing from
`crates/candid-core-wasm/npm` through
[`.github/workflows/npm-release-cli.yml`](../.github/workflows/npm-release-cli.yml)
— a faithful mirror of `npm-release.yml`: dispatch-only `{commit, version,
dist-tag}` with the optional `stable-off-latest`, the same input shape checks
and dist-tag guard, the same ancestor-of-main requirement, the same
protected `npm-publish` environment whose reviewers the verify job confirms
exist, and OIDC trusted publishing with `--provenance`. Everything in the
schema package's section above applies unchanged — the once-per-name
prerequisites, the manual bootstrap first publish (run from
`crates/candid-core-wasm/npm`, with `npm run build` building the wasm), and
the trusted-publisher configuration with **workflow filename
`npm-release-cli.yml`**.

The packaged-consumer verification is this package's own,
[`verify_npm_cli_package.py`](../tests/fixtures/packaging/verify_npm_cli_package.py):
what the tarball actually ships, that the wasm is in it and is a real
artifact, and that a clean consumer can install it, run `gen` end to end, and
compile against the `@candid-core/schema` peer it declares — with no DOM lib,
which is the shape a Node consumer of a CLI actually uses. It runs in the
release verify job *and* on every pull request through the `wasm CLI`
workflow, so its first execution is never the dispatch itself.

Two differences, both because the artifact embeds a wasm build:

1. The verify and publish jobs build with the exact-pinned release toolchain
   (`RELEASE_TOOLCHAIN` from `tests/fixtures/packaging/release-tools.env`)
   plus the pinned `wasm-pack` 0.14.0 — never the rolling `stable` of
   ordinary CI — and `wasm-opt` is deliberately off, so the published bytes
   are a pure function of those two pins.
2. The version pairing is a **revision** pairing: the package embeds
   `candid-core` and the unpublishable `candid-core-ts` generator from the
   dispatched commit itself. `npm/CHANGELOG.md` names the embedded
   `candid-core` version per release, and the release record names the exact
   SHA; the `wasm CLI` workflow's parity gates are what make that pairing a
   verified property rather than a claim.

The name's first publish was the owner's explicit act, per the standing
release discipline: the once-per-name bootstrap above, then the trusted
publisher, then a dispatch of this workflow like any other.

## npm: a coordinated beta of the pair

While `@candid-core/schema` changes shape for its downstream (the 0.3 line),
the two packages release as a pair of betas: `@candid-core/schema`
`0.3.0-beta.N` and `@candid-core/cli` `0.2.0-beta.N`, both under the `beta`
dist-tag, with `latest` left on the last stable versions. The CLI declares the
schema package as an optional peer at **exactly** the paired version, not a
range. That is the lockstep: a generated module needs the runtime release it
was generated for, so every schema beta forces a CLI beta that raises the
exact peer, even when the generator did not change. The schema package stays
0.x through ic-reactor's 4.0 release; 1.0 follows after a stability window.

### The version-bump pull request

One pull request carries both packages, because the CLI's packaging gate
installs the local schema tarball beside the CLI tarball and compiles against
it. A schema version the CLI's exact peer does not name fails that gate
either way, at one of two steps (both measured with npm 11.3.0). When the peer
names a version the registry does not hold, the install itself succeeds — the
peer is optional, so npm only warns `ERESOLVE overriding peer dependency` and
leaves the schema package out of the tree — and the gate fails compiling the
consumer (`TS2307` for `@candid-core/schema/contract`). When the peer names a
published version, such as the last beta, npm refuses the install with
`ERESOLVE could not resolve`. It contains, and contains only:

1. `crates/candid-core-ts/ts/package.json` and the two root entries of its
   `package-lock.json` at the schema beta; `crates/candid-core-wasm/npm/package.json`
   and the two root entries of its `package-lock.json` at the CLI beta, with
   `peerDependencies["@candid-core/schema"]` set to the schema beta exactly.
   `crates/candid-core-wasm/Cargo.toml`'s version is not the CLI's and is
   not bumped.
2. Both changelogs: `## Unreleased` renamed to `## <version> — <YYYY-MM-DD>`,
   the date being the day the dispatch is planned. The gates require a real
   date and refuse "prepared"; if the merge slips past that day, a commit
   correcting the date comes first, and that commit is the one dispatched. The
   entry is reconciled into one text for a reader upgrading from the last
   stable version: every **BREAKING** item kept, no per-PR "release ordering"
   notes left contradicting each other, and every measured figure re-measured
   (the rename re-arms `verify_npm_package.py`'s check of the `schema.d.ts`
   line count). The schema entry carries ``Pairs with `candid-core` X.``; the
   CLI entry carries ``Embeds `candid-core` X …``, where X is the root
   `Cargo.toml` version (the CLI gate compares the two).
3. When the tree's `candid-core` source has moved past the archive crates.io
   holds for X, but the change neither moves a Contract nor an identity of an
   input X accepts, no crate release is required first: the schema entry still
   pairs with X, because the documents it loads are X's, and the README's
   `cargo install candid-core --version X` (which the schema gate holds to the
   pairing) still installs a compiler whose output it loads. The CLI entry
   names X and says, in the same paragraph, what the embedded source has that
   X's archive does not; the release record names the SHA. A change that does
   move a Contract or an identity needs the crate released first, and both
   entries then pair with the new version.
4. The one-line compatibility non-goal, identical in both READMEs, both
   changelog entries and both release notes.
5. Release notes at `.github/release-notes/npm/schema/<version>.md` and
   `.github/release-notes/npm/cli/<version>.md`, each with the public-API diff
   against the previous release: the exported names of the shipped
   `dist/*.d.ts` (schema) and `lib/index.d.ts` plus the command grammar (CLI),
   every break marked. Build the baseline from the previous release's
   `gitHead` (`npm view <package>@<version> gitHead`) and confirm the rebuilt
   tarball's integrity equals `npm view <package>@<version> dist.integrity`
   before diffing against it.
6. The package READMEs, which ship in the tarballs, describing the beta they
   ship in, with `@beta` install lines and `--save-exact`.
7. The repository-side prose that the rename makes false the moment it merges
   (anything pointing at `## Unreleased`), and `website/check.mjs`'s
   `UNPUBLISHED_NPM_SPECS` naming each prepared version and the `beta` tag
   while the tag does not exist on the registry, so no page offers a copyable
   line that resolves to nothing. Pages say in prose what the line will be.

### Dispatch, schema first

After the merge, from the merge commit (`git rev-parse origin/main`), the owner
dispatches the schema package first, because the CLI's peer must exist before
a consumer can install the CLI:

```bash
gh workflow run npm-release.yml \
  --field commit=<merge SHA> --field version=0.3.0-beta.1 --field dist-tag=beta
# approve npm-publish once the verify job is green, then:
gh workflow run npm-release-cli.yml \
  --field commit=<merge SHA> --field version=0.2.0-beta.1 --field dist-tag=beta
# approve npm-publish once the verify job is green
```

`stable-off-latest` stays unset (false) for a beta; the verify job refuses a
prerelease under `latest`, and refuses `stable-off-latest=true` with a
prerelease.

### Checking the install from the tag

Read-only, after each publish:

```bash
npm view @candid-core/schema dist-tags          # beta: 0.3.0-beta.1, latest unchanged
npm view @candid-core/schema@0.3.0-beta.1 gitHead dist.integrity
npm view @candid-core/cli dist-tags             # beta: 0.2.0-beta.1, latest unchanged
npm view @candid-core/cli@0.2.0-beta.1 peerDependencies gitHead

# a clean consumer, from the tag rather than from a tarball
cd "$(mktemp -d)" && npm init -y >/dev/null
npm install --save-exact @candid-core/schema@beta @candid-core/cli@beta
printf 'service : { ping : () -> () };\n' > s.did
npx candid-core-cli gen ./s.did -o out            # writes out/s.ts and out/s.envelope.json
npx candid-core-cli gen ./s.did -o out --check    # exit 0: a second run reproduces the bytes
```

`gitHead` must be the dispatched SHA; a plain `npm install @candid-core/schema`
in the same directory must still resolve `latest`.

### Publish day

Once both are on the registry (`npm view <package> dist-tags` shows them under
`beta`), a follow-up pull request turns what was prepared into what is
installable: the published specs leave `UNPUBLISHED_NPM_SPECS`, the prose
install lines on the website become checked blocks, the one release note per
TypeScript page (`NOTE_TITLE` in `website/check.mjs`, "Published as a beta"
since 0.3.0-beta.1) says which beta the page describes and that `latest`
differs, the status page records the publish date and both release runs, and
each npm release note gains its release record (commit, run, shasum,
integrity, from `npm view`). Nothing in a tarball can change after the fact,
which is why the package READMEs carry their beta lines in the version-bump
pull request instead. For 0.3.0-beta.1 / 0.2.0-beta.1 this was the pull
request that closed #194.

### The next beta

The next round is the same pull request with N + 1: changes accumulate under
`## Unreleased` in both changelogs between rounds; the bump renames them to
`0.3.0-beta.N+1` and `0.2.0-beta.N+1` (the CLI gets a beta even if only the
schema changed, saying so in its entry), raises the exact peer, adds both
version specs to `UNPUBLISHED_NPM_SPECS`, and diffs the public API against
the previous beta. Dispatch order is again schema first.

### A stable release of the pair

A stable pair (`0.3.0` / `0.2.0`, the first) is the same pull request, items 1
to 7, with these differences:

- The versions carry no prerelease suffix, and the CLI's exact peer names the
  stable schema version.
- Each changelog entry is written for a reader upgrading from the last
  *stable* version (`0.2.0` / `0.1.0`), not from the previous beta. The beta
  entries stay as they were published: they are history, and they shipped in
  their own tarballs. The stable entry says that it and the beta entries
  below it are one upgrade, summarises every break between the last stable
  version and this one, and then lists, with each **BREAKING** item kept,
  what changed since the last beta.
- The public-API diff in the release notes is against the previous stable
  release, rebuilt from its `gitHead` as in item 5; a line says what moved
  since the last beta.
- The package READMEs describe the stable release with plain install lines,
  `npm install --save-exact @candid-core/schema` and
  `npm install --save-dev --save-exact @candid-core/cli`, and say what the
  previous stable version lacks.
- `UNPUBLISHED_NPM_SPECS` names the two exact versions only: `latest` and
  `beta` both resolve already.
- Both workflows are dispatched with `--field dist-tag=latest`, schema first,
  `stable-off-latest` unset. Afterwards `latest` names the stable pair and
  `beta` still names the last betas; nothing moves `beta`.
- On publish day the follow-up pull request empties `UNPUBLISHED_NPM_SPECS`,
  turns the prose install lines into blocks, and rewrites the release note on
  each TypeScript page (`NOTE_TITLE`), which until then says which beta the
  page describes.

If the merge slips past the date in the two changelog headings, a commit
correcting the date comes first and is the one dispatched, as for a beta.

### Promoting to `latest`

The supported way to move `latest` is a stable release through the workflows
(`0.3.0` with `dist-tag=latest`, above). Pointing `latest` at an already-published
version by hand, a beta included, is the owner's act outside the workflows,
with an interactive login exactly as for the bootstrap:

```bash
npm login                                             # interactive, 2FA
npm dist-tag add @candid-core/schema@<schema-version> latest
npm dist-tag add @candid-core/cli@<cli-version> latest   # the CLI that pairs with it
npm dist-tag rm @candid-core/schema beta              # only if the tag should go
npm dist-tag rm @candid-core/cli beta                 # likewise
npm view @candid-core/schema dist-tags
npm view @candid-core/cli dist-tags
npm logout
```

Both packages move in the one login, schema first. Moving only the schema
leaves `latest` on a CLI whose peer range does not admit it: a plain install
of both then pairs the new schema with the old CLI, npm warns and leaves the
optional peer unmet, and the generated module fails to compile. `<cli-version>`
is the CLI release whose exact peer is `<schema-version>`.

No guard checks this path: `npm dist-tag add` will point `latest` at a
prerelease, which makes a plain `npm install` resolve a beta.
