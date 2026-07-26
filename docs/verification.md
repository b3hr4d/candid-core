# Release verification gates

Verification status is decision-specific. An ADR remains **Implemented, verification pending** until every gate in its required-verification list has recorded evidence. ADR 0002 is **Verified** because the independent-vector gate completed as recorded below; that status does not imply that any other ADR's gates are complete. The release-candidate gates in [their own section](#release-candidate-gates) are a separate axis again: they bound what a published archive contains and how it behaves for an external consumer, and passing them promotes no ADR.

## Enforced in this repository

- `Verify` CI runs the declared Rust 1.78 MSRV suite and current stable tests on Linux, macOS, and Windows.
- The feature-matrix job builds and tests every supported feature combination, so a change that only compiles with defaults fails before merge:

  ```sh
  cargo test --all-targets --locked                                                   # defaults
  cargo test --all-targets --locked --no-default-features                             # base model only
  cargo test --all-targets --locked --no-default-features --features host-value
  cargo test --all-targets --locked --no-default-features --features compiler
  cargo test --all-targets --locked --no-default-features --features compiler,host-value
  cargo test --all-targets --locked --no-default-features --features filesystem-compiler
  cargo test --all-targets --locked --all-features
  cargo clippy --all-targets --all-features --locked -- -D warnings
  ```

  Suites with meaningful pure-model coverage (`adr_conformance`, `adversarial_regression`, `api_portability`, `candid_name_hash`, `canonical_properties`, `conformance_vectors`, `contract_foundation`, `diagnostics_contract`, `model_public_api`) build under `--no-default-features` and gate individual cases, so all 11 conformance vectors and the canonicalization properties run with no features at all. Suites that need a feature throughout declare `required-features` in `Cargo.toml` and are skipped rather than emptied. Since [issue #21], the suites whose resolver coverage is `MemoryResolver`-only (`browser_wasm`, `input_bounds`, `provenance_bounds`, `source_identity_bounds`) and the `hermetic_bundle` example declare `compiler` rather than `filesystem-compiler`, so they run in the compiler-only configuration too; the individual `WorkspaceResolver` and `compile_did_file` cases inside them stay gated on `filesystem-compiler`.
- The WASM job builds the library for `wasm32-unknown-unknown` under each feature set that is meant to work there, and lints the browser configuration with warnings denied:

  ```sh
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features host-value
  cargo check --lib --target wasm32-unknown-unknown --locked --no-default-features --features compiler
  cargo check --lib --target wasm32-unknown-unknown --locked                        # defaults
  cargo clippy --lib  --target wasm32-unknown-unknown --locked --no-default-features --features compiler -- -D warnings
  cargo clippy --test browser_wasm --target wasm32-unknown-unknown --locked --no-default-features --features compiler -- -D warnings
  ```

  The default build still succeeds on `wasm32-unknown-unknown` because `cap-std` is declared under `cfg(not(target_os = "unknown"))` in addition to being gated on `filesystem-compiler`. These are *build* checks; the runtime claim is the browser job below.
- The browser job is the runtime evidence for [issue #21]. Three things are pinned exactly: `wasm-pack` at `0.14.0`, the browser at Chrome for Testing `150.0.7871.124`, and the ChromeDriver taken from that same Chrome for Testing build (`browser-actions/setup-chrome@v2` with `install-chromedriver: true`, which matches the pair by construction). The driver path is then handed to `wasm-pack` explicitly via `--chromedriver`, which is what makes the pin hold: left to itself, `wasm-pack` downloads its own *latest* ChromeDriver, which can be a different major version from the installed browser and fails the WebDriver session before any test runs. No part of this is a rolling channel, and the job prints both versions before running so the evidence names what it ran on.

  ```sh
  cargo install wasm-pack --version 0.14.0 --locked
  # Both paths come from the pinned setup-chrome step's outputs. `--chrome` is
  # explicit: `--chromedriver` is documented to imply it, but wasm-pack 0.14.0
  # does not apply that implication and exits with a usage error without it.
  wasm-pack test --headless --chrome --chromedriver "$CHROMEDRIVER_PATH" -- --locked --test browser_wasm --no-default-features --features compiler
  ```

  `tests/browser_wasm.rs` compiles a self-contained source and a four-source imported bundle — a type import, an `import service`, and a diamond where one target is reached from two importers — inside the browser; pins its `contract_id`, `interface_id`, `source_bundle_id`, exact logical sources and exact import edges; round-trips the provenance sidecar through `SourceInfo::try_from_raw`; and asserts that an imported service with no main service, an unbound imported type, an unparseable imported source, an unresolvable import, an exhausted `sources` limit, cancellation, and an explicit deadline all fail with stable codes, phases, and resource triples and never leak a materialized name, a temporary directory, or a native path. Every case body is shared: on `wasm32-unknown-unknown` it is a `wasm_bindgen_test` with `run_in_browser`, and on every other target the identical assertions run under the ordinary harness in the native jobs, so the pinned identities cannot drift between the two. The browser dev-dependency (`wasm-bindgen-test =0.3.58`, which pins `wasm-bindgen =0.2.108` and declares `rust-version = "1.71"`) is target-specific, so it never enters the native or MSRV graph. A non-zero exit from `wasm-pack` — a build failure, a browser that will not start, or a failed assertion inside Chrome — fails the job.
- The dependency-boundary job runs `python3 tests/fixtures/packaging/verify_feature_graph.py`, which resolves `cargo metadata` for each feature set and target and asserts that the base graph excludes `candid`, `candid_parser`, `cap-std`, and `ic_principal`; that `host-value` adds `ic_principal` and nothing from the Candid engine; that `compiler` adds the parser stack but no `cap-std`; and that `cap-std` appears only for `filesystem-compiler` on targets that have a filesystem. It follows normal and build edges only, because dev-dependencies never reach a downstream consumer — which is also why the browser harness is invisible to it. This is the *dependency* boundary, and it is a different question from the *archive* boundary: feature selection bounds what a consumer must build, while the `include` allowlist bounds what a consumer must download. Both gates live in the same directory; see [Release-candidate gates](#release-candidate-gates) below.
- The two compilation backends are pinned against each other by `src/compile/differential.rs`, a native unit-test module that runs the same `MemoryResolver` bundles through the promoted in-memory backend and through the materialized `candid_parser::check_file` backend `compile_did_file` uses. Valid bundles must produce byte-identical canonical Contracts, identities, and provenance; invalid bundles must produce identical stable diagnostic codes, phases, and resource triples. It covers plain type imports, service plus type imports, diamond and repeated imports, a target reached by both import kinds, recursion, actorless and class actors, and merge, type-check, duplicate-binding, and parse failures.
- The crate's internal Candid name hash is pinned against `candid_parser::candid::idl_hash` by `tests/candid_name_hash.rs` and by unit tests in `src/name_hash.rs`, in every feature configuration including `--no-default-features`. That is what keeps canonical bytes, `contract_id`, and `interface_id` unchanged now that base validation no longer links the parser.

[issue #21]: https://github.com/b3hr4d/candid-core/issues/21
- Property tests cover canonicalization idempotence, input-arena permutation (including a generated-permutation property over a graph with duplicate semantic nodes, an `idl_hash` collision, and mutual recursion), semantically equivalent source ordering, UTF-8/scalar declaration ordering, and the absence of Unicode normalization.
- Checked-in vectors are driven by `tests/fixtures/conformance/manifest.json`, whose required scenario set — actorless, empty actor, class, basic service, recursion, mutual recursion, `idl_hash` collision (with an id-versus-name method-order divergence), Unicode ordering/escaping, duplicate semantic nodes, arena permutations, and declaration-root traversal order (with a strict actor-reachable interface prefix) — is asserted by both the Rust tests and the Python reference, so a dropped scenario fails instead of passing silently. Every vector pins the canonical graph, canonical JSON text and UTF-8 hex, domain preimage, and IDs; the five legacy wire fixtures are additionally compared exactly, without re-canonicalizing them first, and the actorless vector keeps its byte-level pins in `tests/fixtures/conformance/actorless.identity.json`.
- An independent standard-library Python reference canonicalizer — `python3 tests/fixtures/conformance/verify_vectors.py` — recomputes every manifest vector's canonical graph, payload bytes, preimage, and IDs from the raw noncanonical inputs, without the Rust implementation, and the `Verify` workflow runs it as the dedicated `conformance-reference` job. It supersedes the earlier actorless-only `verify_actorless.py`. The recorded result below completes the independent-vector gate for ADR 0002.
- Detached artifact identity ([ADR 0007](adrs/0007-artifact-identity.md)) has its own independent reference and its own `artifact-identity-reference` job, deliberately separate from the closed semantic conformance set above so that set keeps meaning exactly what it did:

  ```sh
  python3 tests/fixtures/artifact-identity/verify_artifact_ids.py
  ```

  It recomputes every vector's `artifact_id` from the exact bytes on disk, pins the whole domain-framing preimage as hex for the empty vector, re-checks that identical bytes under two kinds produce two separately pinned IDs, and asserts that the nine documents embedding one shared `contract_id` — spanning all three kinds, including a raw Contract document and a `ProducerInfo`-rewritten copy of it — have nine distinct artifact IDs. `tests/artifact_identity.rs` drives the same manifest from Rust in the base configuration and additionally pins the raw Contract goldens as literals, and `tests/browser_wasm.rs` pins the framing anchor of each kind inside the browser so the digest cannot differ between native and WASM. `tests/fixtures/artifact-identity/**` is marked `text eol=lf` in `.gitattributes`, because an exact-octet identity cannot survive a checkout that rewrites line endings.
- The adversarial canonicalization test has deterministic work thresholds; a change that omits work charging or crosses the configured limit fails.
- Pull requests compile every fuzz target and replay its tracked seed and regression corpora with `-runs=0`, so a target that stops compiling, or a previously fixed crash that returns, fails on the pull request rather than on the next schedule. The replay performs no mutation and is therefore deterministic. Both fuzz jobs first assert that `fuzz/Cargo.lock` is current, since `cargo fuzz` accepts no `--locked` flag of its own.
- The weekly fuzz job exercises source parsing, Contract JSON, canonicalization, resolver IDs, provenance, HostValue JSON, and envelope parsing, seeded from the tracked corpora. The fuzz crate mirrors the library's features and each target declares the feature that owns the API it drives, so `cargo fuzz build` still builds all seven targets while a reduced feature set builds only the targets that remain meaningful. Both fuzz jobs upload their crash artifacts, so a red run yields a reproducer without re-running locally.
- Pull requests compile and exercise every benchmark once without enforcing wall-clock thresholds. Weekly and manually dispatched runs retain Criterion's raw estimates, allocation measurements, toolchain, host, and exact commit as downloadable CI artifacts.

## Release-candidate gates

Everything above answers "does this repository behave correctly". This section
answers a different question: "does the archive a consumer downloads behave
correctly". A consumer never sees this repository, and every gate here exists
because a repository-relative check cannot make the claim.

None of it publishes anything. The `Release candidate` workflow holds
`permissions: contents: read` and nothing more, references no crates.io token,
and neither tags, releases, nor mutates GitHub. The human steps that do mutate
something outside the repository are gathered in [releasing.md](releasing.md#7-authorized-mutation).

### Pinned tool versions

Two release tools are not in the dependency graph and must not become
dependencies of the crate they verify. Their versions are pinned in exactly one
place — [`tests/fixtures/packaging/release-tools.env`](../tests/fixtures/packaging/release-tools.env)
— which both the workflow and the local scripts read, so a local run and a CI
run cannot disagree about what was executed.

| Tool | Pinned version | Why it is pinned this way |
| --- | --- | --- |
| `cargo-deny` | `0.20.2` | Advisories, licenses, sources, bans. Run on current stable; 0.20.2 itself declares `rust-version = "1.88"`, so it is deliberately *not* an MSRV dependency of this crate. |
| `cargo-public-api` | `0.52.0` | Generates the committed public API inventory. |
| Nightly toolchain | `nightly-2026-07-15` | `cargo-public-api` builds rustdoc JSON, which is nightly-only and whose format is unstable. An unpinned nightly would rewrite the committed snapshots on its own schedule and the drift check would stop meaning anything. |

Neither tool is a `[dependencies]` or `[dev-dependencies]` entry, and the
packaging verifiers are Python standard library plus `cargo`, so no packaging
check adds a crate to any consumer's graph.

### The package allowlist policy

`Cargo.toml` carries a positive `include` allowlist rather than an `exclude`
list. The difference matters: with `exclude`, anything added to the repository
ships until somebody remembers to exclude it, and an untracked scratch directory
in a contributor's working tree is packaged by default. With `include`, a new
path is outside the archive until it is named on purpose.

The published set is:

```toml
include = [
    "/src/**/*.rs",
    "/examples/**/*.rs",
    "/docs/**/*.md",
    "/README.md",
    "/CHANGELOG.md",
    "/LICENSE",
]
```

Cargo adds `Cargo.toml` (normalized), `Cargo.toml.orig`, `Cargo.lock`, and
`.cargo_vcs_info.json` itself. `docs/**/*.md` rather than `docs/**` keeps editor
and OS droppings out even when a working tree has them.

Two consequences worth stating, because both were true and surprising:

- Cargo **removes** the `[[test]]` and `[[bench]]` sections from the normalized
  published manifest when the allowlist excludes their files, and sets
  `autotests = false`/`autobenches = false`. Without that, a consumer's manifest
  parse would fail on a missing target file. The `[[example]]` sections are
  retained, because the examples *are* published.
- Cargo's dirty-tree check considers only files that would be packaged, so
  `cargo package --locked` succeeds with an untracked scratch directory present
  once that directory is outside the allowlist. That is a convenience, not a
  licence to package a dirty tree; see
  [releasing.md step 1](releasing.md#1-prepare-a-release-candidate-from-a-clean-exact-commit).

`tests/fixtures/packaging/verify_package_manifest.py` asserts the policy in
four directions, and each one has its own failure mode:

```sh
python3 tests/fixtures/packaging/verify_package_manifest.py --locked
```

1. **Nothing internal ships.** `tests/`, `benches/`, `fuzz/`, `.github/`,
   `.codex/`, `.claude/`, `target/`, any `candid-scope/`, and root
   infrastructure files such as `deny.toml` and `.gitignore` must all be absent.
2. **Nothing required is missing.** Every `src/**.rs`, `examples/**.rs`, and
   `docs/**.md` on disk must be in the archive, along with the three root
   documents and Cargo's own four files. `src/bounded.rs` is named individually
   because the binary reaches it through a `#[path]` attribute, so no `mod`
   declaration would reveal its absence. Per-directory floors catch an `include`
   glob that silently matches nothing.
3. **No unexplained extra path**, and no file under a published directory that
   the allowlist does not match. A `src/table.json` behind an `include_str!` or a
   `docs/diagram.svg` an ADR links to compiles and renders in this repository and
   is simply absent from the archive; this is the check that catches it before a
   consumer does.
4. **The manifest has not drifted.** The `include` list, `repository`,
   `homepage`, `documentation`, `readme`, `license`, the keywords, and the
   categories are compared against recorded values, and both lockfiles must
   record the same `candid-core` version as `Cargo.toml`. Relaxing the allowlist
   is a deliberate edit to this script, in the same change.

### Clean packaged-consumer surfaces

```sh
cargo package --locked
cargo publish --dry-run --locked
bash tests/fixtures/packaging/verify_packaged_consumers.sh
```

The script unpacks the archive into a fresh temporary directory, **refuses to run
if that directory is inside the repository**, and builds six external consumer
crates plus an installed CLI against it. Each consumer is generated on the spot
with a `path` dependency on the unpacked package, so nothing resolves back to
this checkout:

| Consumer | Feature selection | What it proves |
| --- | --- | --- |
| base | `default-features = false` | The pure model builds and runs with no Candid engine; `ProducerInfo::current` finds its pinned engine versions in the *normalized* manifest Cargo generated |
| compiler | `default-features = false, features = ["compiler"]` | Self-contained and imported in-memory compilation |
| all-features | `["compiler", "filesystem-compiler", "host-value"]` | The full native surface including host-value validation |
| CLI | `cargo install --locked --path <unpacked> --no-default-features --features filesystem-compiler` | The binary installs from the archive, then compiles and validates a `.did` file the script writes itself |
| wasm base | `default-features = false`, `--target wasm32-unknown-unknown` | The base model still checks for bare WASM from the archive |
| wasm compiler | `default-features = false, features = ["compiler"]`, `--target wasm32-unknown-unknown` | The browser surface still checks for bare WASM from the archive |

Two details are load-bearing. The CLI smoke writes its own source because every
fixture under `tests/` is deliberately outside the archive — a smoke that read
`tests/fixtures/` would pass in this repository and fail for every real
consumer. And `cargo tree` is run over the base consumer and fails if `candid`,
`candid_parser`, `cap-std`, or `ic_principal` appears, because the feature
boundary has to hold in the published manifest and not only in this one.

The archive is also documented as docs.rs will build it:

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --no-default-features   # in the unpacked package
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --all-features          # in the unpacked package
```

This is additive to the existing `Verify` feature-matrix job, which already runs
the same warnings-denied rustdoc gate on the repository tree and continues to do
so. The packaged run answers the separate question of whether an intra-doc link
survives the allowlist.

### Dependency, license, and advisory review

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --all-features check advisories licenses sources bans
```

`deny.toml` blanket-allows nothing. Every exception is narrow and written down:

| Exception | Scope | Reason |
| --- | --- | --- |
| `Apache-2.0 WITH LLVM-exception` | `rustix`, `cap-primitives`, `cap-std`, `io-lifetimes`, `io-extras`, `linux-raw-sys`, `winx`, `ar_archive_writer`, `fs-set-times`, `ambient-authority`, `rustix-linux-procfs` | The standard permissive licence of the `rustix`/`cap-std` family this crate's filesystem capability layer is built on. Apache-2.0 plus an exception that only *grants* more permission. |
| `CC0-1.0` | `tiny-keccak` only, and only as a build-dependency of `lalrpop`, itself a build-dependency of `candid_parser` | Public-domain dedication. It never appears in a consumer's runtime graph, and it arrives through an exact-pinned upstream dependency this issue does not change. |
| `RUSTSEC-2024-0436` | `paste 1.0.15` | Unmaintained, **not** a vulnerability. Reached only through `candid 0.10.30`, which is exact-pinned; upstream offers no safe upgrade. `unmaintained = "all"` is kept — the strictest setting — so this stays one named advisory rather than a whole class being switched off. |

`Unlicense` needs no entry: the crates carrying it (`memchr`, `walkdir`,
`byteorder`, `aho-corasick`, `same-file`, `termcolor`, `winapi-util`) all offer
`MIT OR Unlicense`, and MIT is allowed. `multiple-versions` is `warn`, not
`deny`, because the duplicate pairs (`syn` 1/2, `thiserror` 1/2,
`io-lifetimes` 2/3, `unicode-width` 0.1/0.2, `windows-sys` 0.59/0.61) all come
from exact-pinned upstream graphs this crate does not control. `wildcards`,
`unknown-registry`, and `unknown-git` are all `deny`, and `allow-git` is empty.

### Public API inventory

```sh
cargo install cargo-public-api --version 0.52.0 --locked
rustup toolchain install nightly-2026-07-15
bash tests/fixtures/packaging/verify_public_api.sh            # fails on drift
bash tests/fixtures/packaging/verify_public_api.sh --write    # accept reviewed drift
```

| Surface | Snapshot |
| --- | --- |
| base (`--no-default-features`) | `tests/fixtures/packaging/public-api-base.txt` |
| full (`--all-features`) | `tests/fixtures/packaging/public-api-all-features.txt` |

Both are generated with `-s`, which omits blanket implementations inherited from
dependency traits. Auto-trait impls (`Send`, `Sync`, `Unpin`) and derived impls
(`Clone`, `Debug`, `PartialEq`) are deliberately kept, because losing one of
those is a breaking change and has to appear in the diff. Neither snapshot
contains a version string, so a version bump alone never regenerates them.

Two surfaces rather than one, because they are two published APIs: an item that
moves from the base surface to behind a feature is a breaking change for a
`default-features = false` consumer even though the full surface is unchanged.

`cargo semver-checks` is deliberately **absent**. It compares against a
published baseline, and `candid-core` has none: nothing has ever been published
under this name. Semver comparison begins with the release *after*
`0.1.0-beta.1` exists on crates.io, at which point it becomes a required gate
here.

### Release-candidate evidence template

Fill this in for the specific commit being proposed, and mark anything that does
not exist yet as `pending` rather than guessing it. A PR number, a CI run URL, a
merge commit, and a crates.io URL are all things that either exist or do not.

| Evidence | Value |
| --- | --- |
| Version | `pending` |
| Release commit | `pending` |
| Tree state | `git status --porcelain` empty — `pending` |
| Pull request | `pending` |
| `Verify` run | `pending` |
| `Release candidate` run | `pending` |
| `.crate` file name | `pending` |
| `.crate` bytes | `pending` |
| `.crate` SHA-256 | `pending` |
| Packaged path count | `pending` |
| Packaged contents | attached as the `crate-archive-<sha>` artifact — `pending` |
| `cargo publish --dry-run --locked` | `pending` |
| Packaged consumers (6 surfaces + CLI, Linux and macOS) | `pending` |
| Packaged rustdoc, both surfaces, warnings denied | `pending` |
| `cargo deny check advisories licenses sources bans` | `pending` |
| Public API drift, both surfaces | `pending` |
| MSRV 1.78 | `pending` |
| Browser runtime evidence | `pending` |
| Independent canonicalization reference (11 vectors) | `pending` |
| Independent artifact-identity reference (10 vectors) | `pending` |
| Fuzz build and deterministic replay | `pending` |
| Tag / crates.io / GitHub prerelease | not performed; requires explicit authorization — see [releasing.md](releasing.md#7-authorized-mutation) |

The SHA-256 belongs in a release record and never in the repository: the next
commit changes `.cargo_vcs_info.json` inside the archive, so a committed checksum
is stale by construction. CI reports it in the job summary and retains it, with
the exact file list, as a build artifact.

## Recorded canonicalization v1 evidence

ADR 0002 requires an implementation outside the Rust crate to reproduce every checked-in vector's canonical bytes and IDs. The Rust reference test alone is deliberately insufficient evidence, and CI wiring without a recorded result is not evidence of execution.

| Evidence | Recorded value |
| --- | --- |
| Canonicalization profile | `candid-core-canon-1` |
| Independent implementation | `tests/fixtures/conformance/verify_vectors.py` (Python standard library only; does not call Rust) |
| Exact command | `python3 tests/fixtures/conformance/verify_vectors.py` |
| Required scenarios | 11, asserted by `tests/fixtures/conformance/manifest.json`, Rust, and Python |
| Pull request | [#73](https://github.com/b3hr4d/candid-core/pull/73) |
| Verified PR head | `b6d7c31de3a7ee7ea751d486f597545a19fd988c` |
| Merge commit | `7d29eb03e1a905de66900f2c083707885c1a3963` |
| CI evidence | [Verify run 29834439291](https://github.com/b3hr4d/candid-core/actions/runs/29834439291), including `conformance-reference` ("Independent conformance reference") |
| Result | All 11 canonical graphs, payload bytes, domain preimages, Contract IDs, and interface IDs reproduced; all 8 pull-request jobs succeeded, while 2 schedule-only jobs were skipped by design |

The recorded job counts describe the workflow as it stood for that run. The
feature-matrix and dependency-boundary jobs were added afterwards and do not
affect this record: canonicalization is base-feature behaviour, and the same
`verify_vectors.py` invocation reproduces the same 11 vectors.

This record completes ADR 0002's independent-vector gate. ADRs 0001 and 0003–0007 remain **Implemented, verification pending** until their own required-verification lists are completed and recorded. ADR 0007 ships its own independent Python reference and CI job, but this document records no run of it yet, so wiring is not evidence and its status is unchanged.
