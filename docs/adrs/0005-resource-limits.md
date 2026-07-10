# ADR 0005: Bound all untrusted work and avoid recursive execution

- Status: Accepted; implementation pending
- Date: 2026-07-10
- Owners: Contract runtime maintainers

## Context

DID sources, Contract JSON, source bundles, recursive graphs, and host values
will be supplied by agents and remote ecosystem components. Valid structure is
not the same as safe cost. Unbounded input, graph refinement, recursive graph
walks, large diagnostic collections, and import expansion can cause memory,
CPU, or stack exhaustion.

## Decision

Every public parse, compile, validate, canonicalize, encode, and decode entry
point accepts a `Limits` policy, directly or through a context. Defaults are
safe for interactive tooling and may be raised explicitly by trusted hosts.

The policy includes at least:

- input bytes, per-source bytes, total bundle bytes, and source count;
- import depth and import edge count;
- type nodes, graph edges, declarations, fields, methods, arguments, results,
  and string bytes;
- diagnostics count and retained diagnostic text;
- HostValue depth, elements, text/blob bytes, and encoded message bytes;
- canonicalization/refinement work units and an optional cancellation/deadline.

Graph and import algorithms use explicit work queues rather than call-stack
recursion. Limits are checked before allocation where possible and during work
otherwise. Exhaustion fails closed with a stable `resource_limit_exceeded`
diagnostic containing `resource`, `limit`, and observed or attempted value. No
partially validated Contract is returned.

Default numeric values live in a versioned operational profile rather than the
semantic Contract format, so hosts can choose server, desktop, or embedded
profiles without changing Contract IDs.

## Consequences

- The runtime is suitable for multi-tenant and agent-facing use.
- Very large legitimate interfaces require an explicit host decision.
- Algorithmic complexity becomes observable and benchmarkable.
- Cancellation is a host concern and does not alter deterministic results.

## Migration

Existing convenience functions continue to use documented defaults. New
`*_with_context` entry points expose limits and cancellation. Recursive
canonical traversal is replaced before external JSON is treated as untrusted.

## Required verification

- Boundary tests at and one unit over every limit.
- Fuzzing for parser, validator, canonicalizer, source resolver, and codec.
- Deep acyclic and cyclic graphs proving no stack overflow.
- Benchmarks and regression thresholds for adversarial partition refinement.

