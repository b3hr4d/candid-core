# ADR 0005: Bound all untrusted work and avoid recursive execution

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

DID sources, Contract JSON, source bundles, recursive graphs, and host values will be supplied by agents and remote ecosystem components. Valid structure is not the same as safe cost. Unbounded input, graph refinement, recursive graph walks, large diagnostic collections, and import expansion can cause memory, CPU, or stack exhaustion.

## Decision

Every public parse, compile, validate, canonicalize, encode, and decode entry point accepts a `Limits` policy, directly or through a context. Defaults are safe for interactive tooling and may be raised explicitly by trusted hosts.

The policy includes at least:

- input bytes, per-source bytes, total bundle bytes, source count, and per-source-ID byte length;
- import depth and import edge count;
- source syntax nesting and checked semantic type depth;
- type nodes, graph edges, declarations, fields, methods, arguments, results, string bytes, and producer-metadata bytes;
- diagnostics count and retained diagnostic text;
- HostValue lexical JSON nesting, semantic depth, elements, text/blob bytes, and encoded message bytes;
- canonicalization/refinement work units, provenance target-resolution work units, source-bundle identity work units, and an optional cancellation/deadline.

Graph and import algorithms use explicit work queues rather than call-stack recursion. Limits are checked before allocation where possible and during work otherwise. Exhaustion fails closed with a stable `resource_limit_exceeded` diagnostic containing `resource`, `limit`, and observed or attempted value. No partially validated Contract is returned.

Default numeric values live in a versioned operational profile rather than the semantic Contract format, so hosts can choose server, desktop, or embedded profiles without changing Contract IDs. This is implemented as `LimitsProfile`, a `#[non_exhaustive]` enum whose only released profile is `InteractiveV1`; its numbers are frozen, and future tunings become new variants rather than edits. The serialized policy is a versioned portable configuration — `{"version":1,"profile":"interactive_v1","overrides":{…}}` — that names its profile and carries only explicit overrides as fixed-width `u64` values, so one document configures identical policy on every platform; overrides a platform cannot represent are rejected with a structured error, never truncated.

## Consequences

- The runtime is suitable for multi-tenant and agent-facing use.
- Very large legitimate interfaces require an explicit host decision.
- Algorithmic complexity becomes observable and benchmarkable.
- Cancellation is a host concern and does not alter deterministic results.

## Implementation

Existing conveniences use `Limits::default`, which is exactly `LimitsProfile::InteractiveV1.limits()`; context-aware entry points expose the policy. `Limits` fields are private — construction starts from a profile and overrides individual fields through `with_*` builders, so adding a limit is neither a source-breaking nor a wire-breaking change. Every zero value is a defined fail-closed policy rather than a rejected configuration: a zero byte/count/work limit rejects any input consuming that resource, `max_diagnostics = 0` retains exactly one out-of-band `resource_limit_exceeded` sentinel so an invalid input never yields an empty error collection, and an elapsed `deadline_unix_ms` (including `0`) fails every bounded operation closed. Unknown configuration versions, unknown profiles, unknown fields, and overrides exceeding the platform's `usize` are rejected with stable structured errors (`unsupported_limits_version`, `unsupported_limits_profile`, `limit_override_unrepresentable`). Contract JSON, source resolution, graph structure, canonicalization, extensions, and HostValue traversal enforce limits. Graph canonicalization uses an explicit work stack. HostValue validation descends recursively, bounded by `max_value_depth`; it is safe because no `HostValue` exceeding that depth can be obtained, not because the traversal is iterative. Limit failures carry structured resource, limit, and observed values.

The compiler revalidates every resolver result before digesting or parsing it and owns source-count, per-source-byte, and bundle-byte accounting. Resolver implementations may reject inputs earlier, but cannot bypass compiler enforcement. Inline compilation uses the same accounting and source-sidecar generation propagates validation failures without panicking.

Source token nesting is bounded before the recursive upstream parser or type
checker is invoked. Checked Candid types are depth-validated with an explicit
work stack, and Contract lowering plus provenance collection likewise use
explicit work stacks rather than recursive descent.

HostValue JSON nesting is bounded the same way and for the same reason: a
constant-stack scan of the document rejects hostile nesting before the recursive
`serde_json` decoder is invoked, so the rejection is a budget decision rather
than a stack-exhaustion abort. Lexical nesting and semantic depth stay separate
limits, mirroring `max_source_nesting` against `max_type_depth`, because one
`vec` level costs two JSON containers and one `record` level costs three — a
single limit could not report an honest observed value for both.

Each context-aware public operation creates one internal consumable budget.
Loading, preflight checks, lowering, Contract validation, canonicalization, and
provenance validation share that instance instead of resetting allowances at
stage boundaries. Retained resources use high-water accounting so validating
the same artifact in a later stage does not count it twice, while work units are
consumed cumulatively.

Canonicalization work is proportional to the graph operations it performs.
It charges nodes and edges visited, signature and string bytes produced,
comparison bounds for sorted collections, graph reindexing and rewriting, and
the canonical bytes serialized and hashed for identities. Callers that both
validate and consume the canonical result reuse the same canonicalization pass.

Provenance validation resolves every field-label and method occurrence against
its target aggregate or service. Because a single aggregate may legally hold up
to `max_fields` entries and the label count is independently bounded by the same
limit, an unindexed per-label scan would be quadratic in `max_fields` and, with
duplicate occurrences permitted by design, would re-pay a full scan per
duplicate. Each referenced container's field-ID set (and each service's
method-name set) is therefore built once and consulted by membership test.
That index construction and every lookup are charged against a dedicated
`provenance_work` counter, kept separate from `max_canonicalization_work` so
that rederiving a large graph and then indexing its provenance sidecar cannot
jointly exhaust one counter. HostValue variant-tag resolution is charged
identically to record field-ID matching, so a hostile variant table amplified
across many values is bounded by `max_canonicalization_work` rather than
scanning uncharged.

Source-bundle identity serialization and hashing is charged to a dedicated
`source_identity_work` counter (`max_source_identity_work`). Exactly two
identity passes remain: the compiler emitting `candid-core:source-bundle:v1`
for a lowered bundle, and presented-sidecar validation verifying the presented
ID — whose rederivation then repeats the compiler's pass on the same budget, so
one validation charges two passes and one compilation charges one. Each pass
charges one unit per serialized payload byte during an allocation-free
streaming counting pass, then reserves two further units per canonical byte
plus the domain tag before anything is materialized. The counter is separate
from `max_canonicalization_work` because the serialized bundle scales with
`max_bundle_bytes`; metering it on the graph counter would force one default to
reject inputs the other must accept. The default is derived from the byte
limits: JSON string escaping expands a byte to at most six, so the costliest
default-valid compile pass is ~213M units and the two validation passes
together are ~341M, and the 400M default accepts every bundle that is valid
under the default bundle/source/import byte and count limits. The counting
pass observes cancellation, deadlines, and exhaustion between serializer
chunks; the final serialize-and-hash pass is one uninterruptible block — the
unavoidable granularity — bounded because loading or the sidecar preflight has
already enforced the bundle byte limits. Canonicality and the earlier
malformed/byte/count checks run before the charge, so pre-existing errors keep
their precedence; an identity mismatch is only observable after hashing, so an
exhausted identity budget reports `source_identity_work` where a mismatch
would otherwise have been detected.

Producer metadata is untrusted, caller-supplied provenance. Its aggregate bytes
are bounded (`max_producer_bytes`), but it is deliberately excluded from the
authenticated `candid-core:contract:v1` and `candid-core:interface:v1` identity
payloads: binding it in would change every existing identity, and a signature
over a Contract identity is not a signature over its producer claims. Producer
metadata remains part of the canonical serialized bytes, so it is preserved
losslessly on the wire while never influencing an identity hash.

`RuntimeContext` snapshots the configured Unix deadline into a monotonic local
deadline when work begins and carries a cloneable `CancellationToken` for
cooperative cancellation. Traversals and stage boundaries checkpoint both.
If a platform cannot represent the remaining duration as a monotonic deadline,
the operation fails closed rather than silently discarding an explicit limit.
Synchronous upstream parser/type-checker calls and third-party resolvers cannot
be preempted safely; the runtime checks immediately before and after them, and
custom resolvers may override `load_with_context` to checkpoint during their
own long-running work.

## Required verification

- Boundary tests at and one unit over every limit.
- Fuzzing for parser, validator, canonicalizer, source resolver, and codec.
- Deep acyclic and cyclic graphs proving no stack overflow.
- Benchmarks and regression thresholds for adversarial partition refinement.
