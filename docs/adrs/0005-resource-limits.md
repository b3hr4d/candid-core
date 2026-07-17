# ADR 0005: Bound all untrusted work and avoid recursive execution

- Status: Implemented, verification pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

DID sources, Contract JSON, source bundles, recursive graphs, and host values will be supplied by agents and remote ecosystem components. Valid structure is not the same as safe cost. Unbounded input, graph refinement, recursive graph walks, large diagnostic collections, and import expansion can cause memory, CPU, or stack exhaustion.

## Decision

Every public parse, compile, validate, canonicalize, encode, and decode entry point accepts a `Limits` policy, directly or through a context. Defaults are safe for interactive tooling and may be raised explicitly by trusted hosts.

The policy includes at least:

- input bytes, per-source bytes, total bundle bytes, and source count;
- import depth and import edge count;
- source syntax nesting and checked semantic type depth;
- type nodes, graph edges, declarations, fields, methods, arguments, results, and string bytes;
- diagnostics count and retained diagnostic text;
- HostValue depth, elements, text/blob bytes, and encoded message bytes;
- canonicalization/refinement work units and an optional cancellation/deadline.

Graph and import algorithms use explicit work queues rather than call-stack recursion. Limits are checked before allocation where possible and during work otherwise. Exhaustion fails closed with a stable `resource_limit_exceeded` diagnostic containing `resource`, `limit`, and observed or attempted value. No partially validated Contract is returned.

Default numeric values live in a versioned operational profile rather than the semantic Contract format, so hosts can choose server, desktop, or embedded profiles without changing Contract IDs.

## Consequences

- The runtime is suitable for multi-tenant and agent-facing use.
- Very large legitimate interfaces require an explicit host decision.
- Algorithmic complexity becomes observable and benchmarkable.
- Cancellation is a host concern and does not alter deterministic results.

## Implementation

Existing conveniences use `Limits::default`; context-aware entry points expose the policy. Contract JSON, source resolution, graph structure, canonicalization, extensions, and HostValue traversal enforce limits. Graph canonicalization and value validation use explicit work stacks. Limit failures carry structured resource, limit, and observed values.

The compiler revalidates every resolver result before digesting or parsing it and owns source-count, per-source-byte, and bundle-byte accounting. Resolver implementations may reject inputs earlier, but cannot bypass compiler enforcement. Inline compilation uses the same accounting and source-sidecar generation propagates validation failures without panicking.

Source token nesting is bounded before the recursive upstream parser or type
checker is invoked. Checked Candid types are depth-validated with an explicit
work stack, and Contract lowering plus provenance collection likewise use
explicit work stacks rather than recursive descent.

Each context-aware public operation creates one internal consumable budget.
Loading, preflight checks, lowering, Contract validation, canonicalization, and
provenance validation share that instance instead of resetting allowances at
stage boundaries. Retained resources use high-water accounting so validating
the same artifact in a later stage does not count it twice, while work units are
consumed cumulatively.

`RuntimeContext` snapshots the configured Unix deadline into a monotonic local
deadline when work begins and carries a cloneable `CancellationToken` for
cooperative cancellation. Traversals and stage boundaries checkpoint both.
Synchronous upstream parser/type-checker calls and third-party resolvers cannot
be preempted safely; the runtime checks immediately before and after them, and
custom resolvers may override `load_with_context` to checkpoint during their
own long-running work.

## Required verification

- Boundary tests at and one unit over every limit.
- Fuzzing for parser, validator, canonicalizer, source resolver, and codec.
- Deep acyclic and cyclic graphs proving no stack overflow.
- Benchmarks and regression thresholds for adversarial partition refinement.
