//! TypeScript code generation over the `candid-core` Contract graph.
//!
//! This crate is the separate-crate half of the issue #38 decision: code
//! generation consumes the published Contract model and never influences it.
//! Depending on `candid-core` with `default-features = false` is the boundary
//! made structural — a generator needs no Candid parser, no filesystem
//! capability, and no host-value ABI, and this crate cannot reach for them
//! without that dependency changing shape in review.
//!
//! # Scope, and what is deliberately not claimed
//!
//! - The generator's input is a validated [`candid_core::Contract`]; its output
//!   is TypeScript source text. Nothing here parses Candid.
//! - `@icp-sdk/bindgen` is a differential *oracle* for type-level agreement,
//!   never the specification. Byte-identical output is a non-goal.
//! - No performance claims. Benchmarks arrive only after the generator exists
//!   and go through the baseline machinery from issue #39.
//! - The Contract format is not stable v1, and this crate tracks it as a
//!   pre-1.0 consumer: pinned to an exact `candid-core` version, expecting
//!   breaks.
//!
//! The crate currently contains no generator. It is the scaffold that pins the
//! dependency boundary and the workspace layout, landed alone so that the first
//! generator slice reviews as generator code and nothing else.

// The scaffold's one real assertion: the base surface links and is usable
// without any feature. This fails to compile if the dependency above ever
// stops being feature-free.
#[cfg(test)]
mod tests {
    #[test]
    fn base_surface_links_without_features() {
        let limits = candid_core::Limits::default();
        assert!(limits.max_input_bytes() > 0);
    }
}
