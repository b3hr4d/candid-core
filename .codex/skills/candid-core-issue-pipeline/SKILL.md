---
name: candid-core-issue-pipeline
description: Triage, prioritize, implement, publish, review, merge, close, and release Candid Core work through a safe local workflow. Use for one issue or PR, repository-wide backlog ordering, program umbrellas and their slices, security remediation, review threads, release preparation and execution, or delegated-agent results in b3hr4d/candid-core.
---

# Candid Core Issue Pipeline

Use this workflow for `b3hr4d/candid-core`. Read the repository `CLAUDE.md` and every file it references before acting; it carries the verification battery, the merge boundary, and the recorded footguns, and it overrides this skill where they conflict.

## Select modes and authority

- **Portfolio**: Inspect all relevant open issues and PRs, build a dependency-aware priority queue, and recommend milestones. Do not edit code or GitHub state.
- **Triage**: Inspect one issue and produce a verified design checkpoint. Do not edit.
- **Implement**: Own one issue locally, create its branch, implement it, and verify it. Do not push, create a PR, merge, or close unless the user also authorizes those modes.
- **Publish**: Commit the reviewed implementation, push its issue branch, and create or update its PR.
- **Review**: Inspect a branch or PR without editing. Treat every unresolved or outdated review thread as potentially valid until compared with final code.
- **Remediate**: Address verified actionable review findings on the existing issue branch, then re-run proportional checks.
- **Merge**: Merge only with explicit user authorization in the current request and after independently verifying every merge gate.
- **Close**: Perform post-merge issue housekeeping only after independently confirming every acceptance criterion on refreshed `main`.
- **Release**: Prepare and execute a version release strictly through `docs/releasing.md`. Every externally visible mutation — tag, registry publish, GitHub release — needs its own explicit authorization; one approval never carries to the next step, and registry publications are permanent.

Treat named modes as binding. Infer only the minimum modes clearly requested. A request to implement does not by itself authorize publication or merge; a request to complete an issue end-to-end authorizes the necessary sequence through close unless the user limits it.

## Verify ownership and state

1. Read current GitHub and checkout state instead of trusting summaries, labels, issue suggestions, or cloud-task reports. Inspect the complete issue and discussion, linked and related PRs, checks, reviews, review threads, branch, worktree, and `main` freshness.
2. Reproduce or confirm the issue evidence against current `main`. Treat the reported cause and suggested implementation as hypotheses until the code and tests support them.
3. Permit exactly one implementation owner per issue. Do not implement the same issue concurrently in this session and any delegated or background agent.
4. Use delegated agents only for bounded research, a candidate patch, or read-only review. Inspect their final diff and actual test output locally before adoption. Adopt one candidate as sole ownership or replace it; never combine uncontrolled concurrent implementations.
5. Keep one issue per branch and PR. Coordinate shared architectural decisions across dependent issues without combining their implementation scopes.
6. Preserve unrelated user changes. Never use destructive Git commands unless the user explicitly requests them.

## Portfolio mode

For a repository-wide review:

1. Refresh remote state and enumerate every open issue and PR, including bodies, discussions, labels, milestones, linked work, and recently merged prerequisites.
2. Classify each issue by observed impact:
   - **P0**: process abort, filesystem or sandbox escape, validation bypass, or comparable critical trust-boundary failure.
   - **P1**: resource exhaustion, unbounded untrusted input, unauthenticated identity/provenance, or a foundation required to fix P0 safely.
   - **P2**: compatibility, deterministic output, diagnostics, portability, or public API correctness.
   - **P3**: packaging, developer tooling, performance infrastructure, or new ecosystem capability.
3. Record `blocked by`, `blocks`, `overlaps`, and `independent` relationships. Distinguish a direct fix from a foundational prerequisite and from post-merge housekeeping.
4. Order by verified severity, exploitability and blast radius, prerequisite depth, compatibility risk, and implementation readiness—not issue number, age, or label alone.
5. Propose small milestones and a single highest-priority unblocked issue. Preserve one issue per PR even when several issues form one program.
6. When a program outgrows one issue, split it: keep the original as an umbrella carrying the charter and a dependency-ordered index, and file one issue per slice. Write each slice self-contained — restating the decisions, file paths, constraints, and acceptance criteria it depends on — so a fresh session needs no conversation history to execute it.
7. Flag issues whose acceptance criteria conflict, duplicate merged work, require a material product decision, or cannot yet be verified.

Report the complete ordered queue, dependency rationale, milestone grouping, and recommended execution method. Do not mutate issue labels, milestones, bodies, or state in Portfolio mode.

## Triage and design checkpoint

Before editing, read the complete issue and discussion, relevant source, tests, docs, ADRs, related merged work, and dependencies. Then state:

1. The reproduced or code-confirmed failure and why it matters.
2. An acceptance-criteria traceability table mapping every criterion to the intended code, test, documentation, or explicit blocker.
3. The smallest robust design and deliberate non-goals.
4. Compatibility, stable-output/error, resource-bound, security, platform, and serialization risks.
5. Exact files and adversarial/regression tests to add.

Stop for direction before a material API, serialized-format, identity-domain, dependency, portability-policy, or issue-scope expansion. Present options with a recommendation rather than an open question. Do not silently resolve conflicting acceptance criteria.

Record every settled material decision on the issue itself — the choice, the rejected alternatives, and the reason — so it survives the session. A decision recorded on an issue is settled: apply it, and reopen it only by naming it to the user, never by silently diverging.

## Implement and verify

1. Refresh `main` without discarding work, then create an issue-specific branch from it.
2. Make the narrowest change satisfying the traced acceptance criteria. Keep contract-directed validation separate from syntax or local canonicalization where applicable.
3. Add focused regression and adversarial tests proportional to risk. Test exact boundaries and one-step-over cases. Assert stable codes, paths, resource metadata, precedence, canonical bytes, and platform behavior when public.
4. Re-check every acceptance criterion against the actual diff; do not treat passing tests as sufficient evidence.
5. Run repository-required formatting, focused tests, complete debug tests, Clippy with warnings denied, release tests, the advertised MSRV when installed, and relevant platform/WASM/fuzz/benchmark gates. Use locked/offline flags when supported. Report unavailable checks honestly.
6. Review the final diff skeptically for validation bypasses, changed error precedence, resource exhaustion, unstable serialization or identity, compatibility regressions, hidden allocations, target-specific failures, and unrelated changes. Run whitespace/error checks when available.

## Publish and review

1. Confirm the diff contains only the intended issue scope before committing.
2. Push only the issue branch and create or update a PR; never push an implementation commit directly to `main` by default.
3. Permit a direct-to-`main` exception only when the user explicitly authorizes it in the current request. Verify a fresh `origin/main` and fast-forward relationship first, and report the exception.
4. Make the PR body state the issue and acceptance-criteria mapping, scope and non-goals, deliberate API/format impact, exact verification, security considerations, and limitations.
5. Inspect all reviews, inline comments, and unresolved threads. Compare even outdated comments with final code. Fix every valid actionable finding in a narrow follow-up commit; explain why non-actionable findings do not apply.

## Merge and close gates

Merge only when all are true:

- the user authorized merge;
- every acceptance criterion is satisfied or the user explicitly approved a documented scope change;
- required checks pass and unavailable optional checks are disclosed;
- the PR is conflict-free and based on sufficiently fresh `main`;
- no valid actionable review thread remains unresolved;
- the final diff has been reviewed for security, resource, compatibility, and unrelated changes;
- dependency ordering is still valid and no newer repository state supersedes the solution.

After merge, refresh `main`, verify the merge commit and deployed repository state where relevant, re-run or inspect the evidence needed for closure, and verify automatic issue closure. Comment with the merged PR, concise fix, exact checks, review-thread disposition, and remaining limitations. Close manually only after this verification.

Then check what standing evidence the merge invalidated — a Cargo.lock change that alters the bench binary's resolved dependency graph stales the reviewed benchmark baseline (the root package's own version bump and sibling members' dev-dependency changes do not; see docs/benchmarks.md), any commit stales release digests — and perform or schedule the documented follow-through rather than leaving it implicit.

Keep issue state honest in both directions: if automation closed an issue whose work is incomplete (a closing-keyword accident, a partial slice), reopen it immediately with a comment stating what remains; if reality has moved past an issue's text, rescope and retitle it rather than leaving a description that no longer describes the work.

## Evidence and etiquette

1. Prove a gate by making it fail: inject a representative fault, watch the gate refuse, revert, and cite both directions. A check demonstrated only on its green path is wiring, not evidence.
2. Golden files are reviewed decisions. Regenerate them only deliberately, review the diff as an API change, and keep any generated-versus-declared equality gate (such as an invariant type annotation the compiler must prove) intact rather than loosening it to make a diff pass.
3. Resolve a review thread only after the fix is pushed, with a reply stating what changed and how it was verified. Explain non-actionable findings instead of resolving them silently; if a finding is real but out of scope, file the issue and link it before resolving.
4. Never place a closing keyword next to an issue number that must stay open anywhere GitHub parses it — "does not close #N" still closes #N on merge. Reference with a bare number; use closing keywords only to close.
5. Route long or quote-bearing text (commit messages, PR bodies, thread replies) through files rather than inline shell arguments, and re-read what was actually written before publishing it.

## Completion report

Report actual state, not intended state: selected modes, issue and branch, commit/PR/merge URLs, acceptance-criteria disposition, checks passed, checks failed or unavailable, review-thread disposition, issue comment/closure state, limitations, and the highest-priority unblocked next issue. Never create an empty PR or choose the next issue by number alone.
