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
  rather than omitted.
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
  `0.1.0-beta.2` (or `0.1.1`, once out of prerelease) with the correction,
  record the reason in `CHANGELOG.md`, and yank the bad version so new consumers
  do not find it. Never attempt to re-publish the same version number; crates.io
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
