# Benchmark corpus

These fixtures are part of the repository's Apache-2.0-licensed test material.
They were written specifically for `candid-core`; they are not copied from a
production canister or presented as an official Candid workload.

| Case | Purpose |
| --- | --- |
| `ledger.did` | Realistic single-file service with nested records, variants, callbacks, vectors, options, and update/query methods. |
| `imports/root.did` | End-to-end file benchmark entry point. |
| `imports/common.did` | Shared account, transfer, and result types. |
| `imports/archive.did` | Recursive archive result and callback types. |

The benchmark harness also reuses the small checked-in conformance fixtures and
creates deterministic synthetic cases for record width, service method count,
and recursive graph depth. Generator sizes and fixture contents are part of the
benchmark definition: changing them invalidates direct comparison with earlier
results and should be called out in the pull request.
