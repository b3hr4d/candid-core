# Fuzz targets

Install `cargo-fuzz`, then run each trust-boundary target with a bounded time budget:

```sh
cargo +nightly fuzz run source_parsing -- -max_total_time=60
cargo +nightly fuzz run contract_json -- -max_total_time=60
cargo +nightly fuzz run canonicalization -- -max_total_time=60
cargo +nightly fuzz run resolver_ids -- -max_total_time=60
cargo +nightly fuzz run provenance -- -max_total_time=60
cargo +nightly fuzz run host_value -- -max_total_time=60
```

The scheduled CI job runs the same targets. Minimized crash inputs belong in the relevant target corpus and require a deterministic regression test.
