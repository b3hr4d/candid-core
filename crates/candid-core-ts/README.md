# candid-core-ts

TypeScript code generation over the [`candid-core`](../../README.md) Contract
graph. This is the separate-crate half of the
[issue #38](https://github.com/b3hr4d/candid-core/issues/38) decision: code
generation consumes the published Contract model and never influences it —
`candid-core`'s public API, packaged archive, canonical bytes, and identities
are unaffected by anything in this directory.

**Unpublishable by construction, for now.** `publish = false` stands until the
registry name is decided deliberately; crate names on crates.io are as
permanent as versions, and `candid-core-ts` is a working name, not that
decision.

**Base surface only.** The dependency on `candid-core` declares
`default-features = false`: a generator needs no Candid parser, no filesystem
capability, and no host-value ABI. If a change here ever needs a feature, that
is a design boundary being crossed and belongs in review, not in a lockfile
diff.

**Not covered by the root CI jobs.** The root is a non-virtual workspace, so
`cargo test`/`cargo clippy` at the repository root select the root package
only. This crate gains its own CI wiring together with the first generator
slice — a job that tests an empty scaffold would be evidence of nothing.

Scope and non-goals are recorded on issue #38 and restated in `src/lib.rs`:
`@icp-sdk/bindgen` is a differential oracle rather than the specification, and
no performance claims are made before the generator exists and issue #39's
baseline machinery measures it.
