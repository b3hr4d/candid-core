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
- The TypeScript runtime is fuzzed differentially against the Rust reference ([issue #196]). `crates/candid-core-ts/tests/differential/` generates seeded random Candid environments (as `.did` source, read by `candid_parser` for the reference and by candid-core's compiler for the Contract the runtime loads; field and arm names include quoted, non-ASCII and empty names, names whose label ids collide with each other or with a number, and sibling declarations that respell a field; an environment with an `opt`-only cycle such as `T = opt T` is redrawn, because reading a non-`opt` wire value there unwinds without end on both sides and neither judges it; candid-core's compiler refuses such a source since [issue #234], so the header counts it as a compiler refusal, and a regression vector on such a type supplies the Contract it runs on, which the Contract loaders still accept) plus deep environments (recursive shapes and declaration chains whose values nest up to just below the generation bound, half the runtime's depth bound; the committed corpus cycles through every shape and a chain past 128 constructors), and three kinds of cases, recording the reference's verdict for each: **decode** (a random value at the wire types encoded by the `candid` crate, optionally mutated — byte edits, a LEB128 group rewritten into its non-minimal form, or a structurally invalid type table: a duplicate or unsorted record field id, variant id or service method name — then decoded by `IDLArgs::from_bytes_with_types` at related expected types, under a decoding quota sized from a scan of the message and the type-table budget `max_type_len` set to the runtime's `maxTypeTableEntries`, 100,000), **validate** (a JavaScript domain value converted to a HostValue under a documented mapping and judged by `validate_host_value` under its default limits, whose `max_value_depth` the runtime's `maxDepth` mirrors; a value the HostValue JSON decoder's own nesting cap or budgets refuse is rebuilt through the HostValue constructors, with their depth and element budgets out of the way, and judged), and **contract** (a compiled Contract document edited by replayable JSON operations, structural ones among them, and judged by `Contract::from_json`, identities restamped). `ts/tests/differential/compare.ts` runs the same inputs through `decodeArgs` (schemas from `schemaFromContract`), `validate` and `schemaFromContract` and gives every case one status: it agrees, it diverges with an exact symptom (both verdicts and each refusal's class or code, a value mismatch with a digest of the runtime's values), the reference never judged it (`inconclusive`: its quota, its stack guard or another budget of its own refused; checked before a skip, so a campaign counts every such case), or the loader omits its declaration (`skip`). Accept/accept compares values under one mapping (`bigint` as decimal, every `number` by its binary64 bits, `Uint8Array` as hex; a value list longer than 64 KiB of JSON, a width vector's, is recorded and compared as a digest of its canonical JSON); for decode, reject/reject compares error classes (the reference's from its behaviour — header, malformed, coercion — and the runtime's from its first issue code), never messages or paths; for validate and contract, two refusals agree whatever their codes except a limit, which agrees only with the reference's refusal on the same budget. The reference budgets configured like the runtime's are verdicts, compared exactly: decode's type-table size (`max_type_len` 100,000, checked on the claimed count like `maxTypeTableEntries`), and validate's `max_value_depth` (256, the default, which both sides count alike since #231: the root at 0, a variant's `null` payload one level below it, a `rec` hop not at all) and `max_value_elements` (1,000,000, the runtime's documented `maxElements`, which also charges each examined record key). No rule attributes a divergence to an intended difference or to the reference. The budgets the two sides cannot share are kept out of the generated cases instead: the `candid` crate has no depth or element budget (only its stack and a cost quota), so a decode case whose nesting at the expected types (past 127 levels, half the runtime's bound: set while the runtime also charged a `rec` hop a level, before #231, and kept so the generated cases did not change), value count, claimed length or claimed type-table size reaches the generation bounds is redrawn before it is judged and counted in the corpus header (`case_redraws`), and validate values nest at most 125 levels; the runtime's input-size and numeric budgets (`maxBytes` 10,485,760, `maxNumericBytes` 1,048,576), which the `candid` crate has no counterpart for either, are far above any generated message. The boundaries themselves are pinned only by exact regression vectors, measured against the runtime: a decoded node at Candid level L is charged depth L, however many `rec` thunks its schema reaches it through, a blob's bytes one level below the blob, and a skipped or absorbed wire value one level per wire level; the compiler's `max_type_depth` (256) bounds every node of a declaration chain's type, so a declaration chain reaches the bound only through a blob byte, and the step past it is pinned on recursive shapes.

  CI runs a committed corpus with a fixed count and no time budget (#39): `cargo test -p candid-core-ts --features compiler` regenerates `tests/goldens/differential/corpus.jsonl` from the committed seeds (24 environments × 40 decode, 12 validate and 8 contract cases, and 6 deep environments × 4 decode and 2 validate cases) plus the minimized regression vectors in `tests/fixtures/differential/regressions.json`, requires it byte for byte, requires the reference to have judged every case in it, and requires every structural type-table edit to occur in a generated decode case; a boundary vector of a million bytes or elements is written as a run (`(<hex>*<count>)` in hex, `["r", count, item]` in a validate value); `npm test` replays it (`ts/tests/differential.test.ts`) and accepts a divergence only when the reviewed list `tests/goldens/differential/divergences.json` names its case id with the issue that explains it and exactly the symptom observed. Any other divergence fails, so does a listed case that no longer diverges so (a fix updates the list; a fault that removes an intended difference, such as a decoder that accepts non-minimal LEB128, is caught), and so does an `inconclusive` case. Every listed case cites an issue that is described in the list and says which side is wrong (the reference, the runtime), that the runtime differs by a decision settled on that issue, or that it awaits the owner's call. Every structural edit must also occur alone (a case with no other edit) on both the decode and the contract target, so its own check decides the verdict. A regression vector stays in the corpus for good (#62): it is listed or, when its entry says why (`agrees`), agrees with the reference — a boundary vector (each of the runtime's budgets, listed in the corpus header's `runtime_budgets` and checked there against the runtime's defaults, exactly at its bound and past it, one step past wherever the shape allows, on the paths named here and on no others: `maxDepth`, 256, on the decoded opt and variant chains, on the decoded variant, opt, named-record and vec chains reaching 256 through a blob byte (no declaration chain nests past it), on coercion-inserted `opt`s (an expected `opt` reading a wire `vec`), on the `null` decode supplies for an `opt`, `null` or `reserved` record field the wire omits, after the wire's last field or between two it carries (charged at the field's own level, as `validate` and the reference's `validate_host_value` charge the same domain value, which validate vectors pin at the same two levels), on the skip of an extra field, of an absorbed value, of a value an expected `reserved` argument or field absorbs, of an extra argument (nested vecs, opts or variant arms) and of a record nested in a skipped value, and on validate's vec, variant, record, tuple and opt chains (the deepest both default budgets accept and the first they refuse, the latter on a recursive shape where a declaration chain cannot reach it); `maxElements`, 1,000,000 charges, for a skipped vec (of `null`, of a variant and of a non-empty record), a decoded vec of `null`, a blob and a decoded vec of mixed elements (a record, a variant, an `opt` on the wire and one inside the variant, a tuple, text), a decoded vec of records each omitting an `opt` field, after the wire's last field or before one it carries, with omitted trailing arguments at the bound (one element per synthesized `null`), so that an over-charge of any of those node kinds is refused below the bound, and for validate's vec value (at the bound and past it) and its record and mixed values (at the runtime's bound, which both sides accept: the runtime also charges each examined record key, so the reference counts fewer elements there and nothing past the runtime's bound would agree); `maxBytes`, 10,485,760, by a message of exactly that size holding one text, the longest text the corpus decodes; `maxNumericBytes`, 1,048,576 LEB128 groups, for an unbounded `nat` and `int` on the skip path, which reads them with the same functions as the decoded path (a decoded megabyte `nat` is not compared: the reference's decimal conversion is quadratic in its length); the type-table budget, 100,000 entries, configured alike on both sides; each structural Contract edit alone; the longest principal; a minimal two-byte `nat`; a refused `bool` byte or UTF-8 text; a missing record field; a value of the wrong kind or out of range at each primitive type), or a fixed bug's vector kept so the bug stays fixed. Not compared: messages and paths; the reasons behind two non-limit refusals outside decode; inputs past the bounds above, except the boundary vectors.

  A campaign is a time-boxed run of the same driver over other seeds; nothing in it is timed or asserted:

  ```sh
  # one batch: 100 environments from seed 1000000 (200 decode, 60 validate, 30 contract cases each)
  # and 10 deep environments (20 decode, 5 validate cases each)
  DIFF_SEED=1000000 DIFF_ENVS=100 DIFF_OUT=/tmp/batch.jsonl \
    cargo test -p candid-core-ts --features compiler --locked --test differential -- --ignored --exact differential_campaign
  cd crates/candid-core-ts/ts && DIFF_CORPUS=/tmp/batch.jsonl DIFF_REPORT=/tmp/report.json \
    node --import ./tests/register.ts tests/differential/campaign.ts
  ```

  Loop over consecutive seed ranges until the time box ends. The summary groups every divergence by its exact symptom, with the mutations its cases carried, and counts the cases the reference did not judge by budget; nothing in a campaign is accepted, and each symptom is triaged by hand (minimized, and checked against the issue it belongs to or filed as a new one). `differential_rejudge` (ignored; `DIFF_IN`, `DIFF_OUT`) rebuilds each environment of an existing batch from its `did` and recomputes every reference verdict with the current tree. Minimize each new divergence (`differential_verdicts` answers the reference's verdict for hand-reduced `{did, expected, hex}` or `{did, wire, expected, textual}` requests) and add it to `regressions.json`, then regenerate both goldens (`UPDATE_GOLDENS=1` for the cargo test, then for `node --test --import ./tests/register.ts tests/differential.test.ts`, which rewrites the list: a case keeps its entry while its symptom is unchanged, and any other divergence is written with issue 0, which the next run refuses until it is reviewed and given its issue by hand), then run both again without it.

- `@candid-core/cli`'s compatibility check (`checkCompatible`, [issue #247]) is held to two references. `crates/candid-core-wasm/tests/compatibility.rs` compares it, per written method, with the `candid` crate's own subtype check (`subtype_with_config` at the exact pinned version, both sides' declarations renamed into one environment): its verdict must equal the reference's under the spec's rules (`OptReport::Silence`), and "no error and no warning" must equal the reference's verdict with the special opt rule refused (`OptReport::Error`). It runs on 38 hand-written cases, which also pin every diagnostic's code, method and path, and on a seeded campaign of 600 random interface pairs (an interface and a random edit of it, in both directions, with `func` and `service` reference types among the values); a disagreement fails unless it is listed with its reason, and a listed one that no longer disagrees fails too. The listed disagreements are two instances of the reference's unsound coinductive memo ([issue #227], item 3), in each of which the reference accepts and this check refuses. The committed campaign is bounded and the memo recurs past it: an on-demand campaign (`COMPAT_CAMPAIGN_CASES=15000 cargo test --test compatibility -- --ignored`) requires every disagreement it meets to be in the memo's direction (the reference accepts under the spec's rules and refuses under strict opt reporting; this check refuses). The same test writes `crates/candid-core-ts/tests/goldens/compat/agreement.json` (the hand cases and 150 of the random ones, with their Contracts and per-method verdicts), and `ts/tests/compat-agreement.test.ts` requires the runtime decoder's subtype relation, which it runs on a `func` or `service` reference, to give every verdict, by decoding a live-typed service reference at the written service and at one-method services. Five service-level vectors in `tests/fixtures/differential/regressions.json` (`service_compat_*`) carry the same cases into the differential corpus; the unsound-memo one is listed as a reference-side divergence. The check's `special_opt_rule` warnings are held to a walk that walks every pair again instead of re-reporting a proven pair's warnings (the same code with that reuse turned off): on every hand case and every case of the committed campaign, and of the on-demand one, the two responses must be equal, so the warnings are exactly those at the paths along which no pair of types repeats. `tests/projection.rs` holds the projection (`projectDid`) to its source: projecting every method of each generator fixture, with one extra method that reaches every declaration, keeps the Contract identity and generates a byte-identical module, docs and argument names included; every conformance fixture with a method projects onto all of them with the same methods and modes. The projection's cost is held by counts, never by time: the crate's hidden `project_did_with` reports how many entries a projection sorted into its indexes, how many lookups by name it made and how many names they compared, and the tests pin the service's methods and the declarations each sorted once, and one lookup per distinct requested name, per step from the actor to its service and per declaration reached, each comparing at most ⌊log₂ n⌋ + 2 names among n, so an index built again, a lookup that scans, or a name found without one, fails them; an unknown-method failure must list the service's methods once, with its messages bounded at 4 MiB of text; and a projection that would pass the compiler's 1 MiB bound on one source must stop printing at that bound.

[issue #196]: https://github.com/b3hr4d/candid-core/issues/196
[issue #234]: https://github.com/b3hr4d/candid-core/issues/234
[issue #227]: https://github.com/b3hr4d/candid-core/issues/227
[issue #247]: https://github.com/b3hr4d/candid-core/issues/247
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
dependencies of the crate they verify, and the Cargo that produces the archive
is pinned for a third reason. All three versions live in exactly one place —
[`tests/fixtures/packaging/release-tools.env`](../tests/fixtures/packaging/release-tools.env)
— which both the workflow and the local scripts read, so a local run and a CI
run cannot disagree about what was executed.

| Tool | Pinned version | Why it is pinned this way |
| --- | --- | --- |
| Release toolchain | `1.94.1` | The Cargo that packages and publishes. `cargo package` is byte-stable within one Cargo version and not across versions: this tree at one commit, packaged by 1.91.1 and by 1.94.1, unpacks identically and produces different `.crate` digests (`493e06…` versus `abe811…`). `cargo publish` re-packages rather than uploading a supplied archive, so a rolling `stable` would let the recorded digest describe an archive nobody published. |
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

### Semver compatibility

`cargo semver-checks` was absent from the gates that produced `0.1.0-beta.1`,
because it compares against a published baseline and at that point nothing had
ever been published under this name. That baseline now exists, and
`tests/fixtures/packaging/verify_semver.py` is the gate:

```sh
. tests/fixtures/packaging/release-tools.env
RUSTUP_TOOLCHAIN="${SEMVER_TOOLCHAIN}" python3 tests/fixtures/packaging/verify_semver.py
```

The toolchain is pinned beside the tool in `release-tools.env`, because
`cargo semver-checks` reads rustdoc JSON and each release of it understands a
fixed set of format versions: on a newer stable than the pinned one it stops
with "unsupported rustdoc format" and the gate exits 2, a tool failure, not a
verdict.

It is evidence that no breaking change reached the published surfaces
**unnoticed** — not that none occurred. The distinction is deliberate. This
crate is pre-1.0 and its changelog reserves the right to change the public API,
the serialized shapes, the canonical bytes, and every identity computed over
them; a gate that hard-failed on a break would fight intended work. A warning
nobody must act on would be ignored. So a reported break must be acknowledged in
the `## Unreleased` section of `CHANGELOG.md` by a **list item beginning with a
bolded `BREAKING` marker**: acknowledged it passes, unacknowledged it fails.

The marker must start a list item, not merely appear in the section. The
changelog entry that documents this gate necessarily contains the marker while
explaining it, so a substring match would be satisfied by its own documentation
and would pass every break forever — the anchoring is what stops the gate
defeating itself.

Both published surfaces are checked, because an item that moves from the base to
behind a feature is breaking for a `default-features = false` consumer even
though the full surface is unchanged.

The script forces `--release-type patch`. Without it a branch whose version has
not been bumped compares `X -> X`, which `cargo semver-checks` reads as "assume
major" and answers by running zero checks — a gate that passes because it asked
nothing. Forcing the patch question makes every breaking change visible whether
or not a version bump has happened yet.

This complements the inventory above rather than replacing it: the snapshots
record what the surface *is*, and this classifies what changed.

### The Release workflow

`.github/workflows/release.yml` performs the three mutations in
[releasing.md §7](releasing.md#7-authorized-mutation). It is evidence for two
things that the manual procedure could only ask an operator to promise.

- **The published bytes are the measured bytes.** Its `guard` job repackages the
  input commit with `RELEASE_TOOLCHAIN` and refuses to proceed unless the digest
  equals the one the `Release candidate` run recorded; its `confirm` job then
  reads crates.io's own checksum back and compares it again after publication.
  Previously both comparisons were a human reading two hex strings.
- **Each mutation was separately authorized.** The tag, publish, and release jobs
  run in three GitHub Environments with required reviewers, so the approval
  record is the environment's, not a sentence in a chat log.

What it is *not* evidence for: it promotes no ADR, adds no gate that `Verify`
and `Release candidate` do not already enforce, and makes no claim about whether
a release *should* happen. A human still chooses the commit, reads the step 6
evidence, and approves each environment.

The workflow triggers on `workflow_dispatch` only and declares
`permissions: {}` at workflow level, elevating per job — `contents: write` for
tag and release, `id-token: write` for publish, nothing for `guard` and
`confirm`. It holds no crates.io token; publication uses Trusted Publishing.
`Verify` and `Release candidate`, the two workflows reachable from a pull
request, are unchanged and remain unable to publish.

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
| Release toolchain | `pending` — must be the `RELEASE_TOOLCHAIN` in `release-tools.env`, in CI and at publish time |
| `.crate` file name | `pending` |
| `.crate` bytes | `pending` |
| `.crate` SHA-256 | `pending` |
| crates.io recorded checksum | not performed; exists only after publication — see [releasing.md step 7](releasing.md#7-authorized-mutation) |
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
the exact file list and the Cargo version that produced it, as a build artifact.
It identifies an archive only together with that commit and that Cargo; the
digest crates.io records after publication is the one a consumer's `Cargo.lock`
carries, and step 7 compares the two.

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
