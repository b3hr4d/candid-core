# Releasing candid-core

This is the exact procedure for turning a commit into a published version. It is
credential-free by construction: no step here, and nothing in CI, holds a
crates.io token or a GitHub token beyond `contents: read`. The three steps that
mutate anything outside this repository — creating a tag, publishing to
crates.io, and creating a GitHub prerelease — are gathered at the end under
[Authorized mutation](#7-authorized-mutation) and are performed by a human, on
purpose, after the evidence in steps 1–6 exists.

Read [§8 Irreversibility](#8-irreversibility-yanking-and-rollback) **before**
the first publish. A crates.io version cannot be deleted, and yanking is not
deletion.

Tool versions are pinned in exactly one place:
[`tests/fixtures/packaging/release-tools.env`](../tests/fixtures/packaging/release-tools.env).
Every command below and the `Release candidate` workflow read that file, so a
local run and a CI run cannot disagree about what was executed. Source it first:

```sh
. tests/fixtures/packaging/release-tools.env
```

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
cargo package --locked

# What is in it, and nothing else.
python3 tests/fixtures/packaging/verify_package_manifest.py --locked
cargo package --list --locked | sort

# How big, and exactly which bytes.
ls -l target/package/candid-core-*.crate
shasum -a 256 target/package/candid-core-*.crate     # sha256sum -b on Linux

# The unpacked source, read as a consumer would receive it.
tar -tzf target/package/candid-core-*.crate | sort
tar -xzf target/package/candid-core-*.crate -C "$(mktemp -d)"
```

Record the byte size and the SHA-256 in the evidence table (step 6). The digest
belongs in a release record, never committed to the repository — the next commit
changes `.cargo_vcs_info.json` and invalidates it, so a committed checksum is a
stale checksum by construction.

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
cargo publish --dry-run --locked
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
  `0.1.0-beta.1`, so the examples use `=0.1.0-beta.1`.

## 6. Present the evidence

Fill in [the evidence template in verification.md](verification.md#release-candidate-evidence-template)
and present it for review *before* asking for authorization. Values that do not
exist yet are marked `pending`, never guessed. In particular: do not write a PR
number, a CI run URL, a merge commit, or a crates.io URL until it exists.

## 7. Authorized mutation

Everything above is read-only with respect to the outside world. Everything
below is not, and each item requires explicit authorization from the repository
owner, given after seeing the step 6 evidence. Nothing in CI performs any of
these, and no automation in this repository handles a credential.

1. **Tag.** `git tag -a v<version> -m "candid-core <version>"` on the exact
   release commit, then `git push origin v<version>`. Tag the commit the evidence
   names — not `HEAD`, which may have moved.
2. **Publish.** `cargo publish --locked` from that same clean commit. The
   operator authenticates interactively (`cargo login`, or a token supplied in
   that operator's own environment). Do not add a crates.io token to GitHub
   Actions secrets: the `Release candidate` workflow is designed to be
   unable to publish, and adding a token removes that property.
3. **GitHub prerelease.** Create a release from the tag, marked as a
   prerelease, with the changelog entry as its body and the archive's SHA-256
   quoted in it. `0.1.0-beta.1` is a prerelease in the semver sense and must be
   marked as one in GitHub too.

Do these in that order. A tag without a publish is recoverable; a publish
without a tag leaves a version on crates.io that no commit in the repository is
identified with.

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

For a prerelease specifically: because `"0.1"` does not select
`0.1.0-beta.1`, a broken beta reaches only consumers who asked for it by exact
version. That narrows the blast radius; it does not remove the permanence.
