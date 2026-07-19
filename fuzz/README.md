# Fuzz targets

Each target drives one trust boundary with untrusted bytes. `fuzz/` is a
separate cargo project, so it has its own `Cargo.lock`, which is tracked:
dependencies are pinned exactly, matching the crate under test.

## Corpus layout

Three directories per target, with different jobs:

| Directory | Tracked | Purpose |
|---|---|---|
| `seeds/<target>/` | yes | Valid, structurally interesting inputs that let the fuzzer start deep in the code instead of in the syntax-error path. |
| `regressions/<target>/` | yes | Minimized reproducers for crashes that have been fixed. Replayed on every pull request so they stay fixed. |
| `corpus/<target>/` | no | libFuzzer's working corpus, grown during a run. |
| `artifacts/` | no | Crash, slow-unit, and OOM reproducers. CI uploads these; they are not committed. |

libFuzzer writes only to the **first** corpus directory passed, so always pass
the gitignored `corpus/<target>` first and the tracked directories after it.
That is what keeps seeds and regressions read-only.

Seeding is not cosmetic. Measured on `contract_json`, 30 seconds, same build:

| Corpus | Coverage | Features |
|---|---|---|
| cold | 1036 | 2540 |
| seeded | **4331** | **9428** |

Random bytes almost never form a parseable Contract, so an unseeded run spends
its whole budget being rejected by `serde_json`.

## Running

Install `cargo-fuzz`, then run a target with a bounded time budget:

```sh
cargo +nightly fuzz run contract_json \
  fuzz/corpus/contract_json fuzz/seeds/contract_json fuzz/regressions/contract_json \
  -- -max_total_time=60
```

To run everything the way the scheduled job does:

```sh
for target in $(cargo +nightly fuzz list); do
  mkdir -p "fuzz/corpus/${target}" "fuzz/seeds/${target}" "fuzz/regressions/${target}"
  cargo +nightly fuzz run "${target}" \
    "fuzz/corpus/${target}" "fuzz/seeds/${target}" "fuzz/regressions/${target}" \
    -- -max_total_time=60
done
```

Replay only, no mutation — this is the pull-request gate, and it is
deterministic:

```sh
cargo +nightly fuzz run --dev <target> \
  fuzz/corpus/<target> fuzz/seeds/<target> fuzz/regressions/<target> -- -runs=0
```

## Triaging a failure

1. Download the `fuzz-artifacts-<sha>` artifact from the failed run, or
   reproduce locally. The reproducer is a file under `artifacts/<target>/`.
2. Minimize it: `cargo +nightly fuzz tmin <target> <artifact>`.
3. Open an issue with the minimized bytes and the panic site. Prefer a
   reproducer that does not need the fuzzer — most crashes reduce to a plain
   `#[test]` against the public API.
4. Fix the defect, add a deterministic regression test in `tests/`, and commit
   the minimized input to `fuzz/regressions/<target>/` so the pull-request
   replay pins it.

Do not commit an input to `regressions/` before its defect is fixed; the
replay gate would then fail on every pull request.

## Adding a target

Add `fuzz_targets/<name>.rs`, register a matching `[[bin]]` in `Cargo.toml`,
and add at least one seed under `seeds/<name>/`. `cargo fuzz list` drives both
CI jobs, so no workflow change is needed.

## Regenerating seeds

Seeds are derived from the crate's own compiler rather than copied from
`tests/fixtures/`, so the repository does not carry two sets of the same
artifact that can drift apart. If a serialized shape changes, regenerate them
by compiling a few representative DID sources and writing out the Contract,
Compilation, and envelope renders. Seeds only need to be *reachable* input,
not current vectors — a stale seed weakens coverage but breaks nothing.
