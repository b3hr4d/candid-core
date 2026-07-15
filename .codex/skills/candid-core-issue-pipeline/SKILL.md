---
name: candid-core-issue-pipeline
description: Triage, prioritize, implement, publish, review, merge, and close Candid Core GitHub security, design-remediation, maintenance, and ecosystem issues through a safe local workflow. Use for one issue or PR, repository-wide backlog ordering, dependency and severity analysis, security remediation, review threads, release gates, or Codex cloud task results in b3hr4d/candid-core.
---

# Candid Core Issue Pipeline

Use this workflow for `b3hr4d/candid-core`. Read the repository `AGENTS.md` and every file it references before acting; those instructions override this skill.

## Select modes and authority

- **Portfolio**: Inspect all relevant open issues and PRs, build a dependency-aware priority queue, and recommend milestones. Do not edit code or GitHub state.
- **Triage**: Inspect one issue and produce a verified design checkpoint. Do not edit.
- **Implement**: Own one issue locally, create its branch, implement it, and verify it. Do not push, create a PR, merge, or close unless the user also authorizes those modes.
- **Publish**: Commit the reviewed implementation, push its issue branch, and create or update its PR.
- **Review**: Inspect a branch or PR without editing. Treat every unresolved or outdated review thread as potentially valid until compared with final code.
- **Remediate**: Address verified actionable review findings on the existing issue branch, then re-run proportional checks.
- **Merge**: Merge only with explicit user authorization in the current request and after independently verifying every merge gate.
- **Close**: Perform post-merge issue housekeeping only after independently confirming every acceptance criterion on refreshed `main`.

Treat named modes as binding. Infer only the minimum modes clearly requested. A request to implement does not by itself authorize publication or merge; a request to complete an issue end-to-end authorizes the necessary sequence through close unless the user limits it.

## Verify ownership and state

1. Read current GitHub and checkout state instead of trusting summaries, labels, issue suggestions, or cloud-task reports. Inspect the complete issue and discussion, linked and related PRs, checks, reviews, review threads, branch, worktree, and `main` freshness.
2. Reproduce or confirm the issue evidence against current `main`. Treat the reported cause and suggested implementation as hypotheses until the code and tests support them.
3. Permit exactly one implementation owner per issue. Do not implement concurrently in local and cloud tasks.
4. Use cloud tasks only for bounded research, a candidate patch, or read-only review. Inspect their final diff and actual test output locally before adoption. Adopt one candidate as sole ownership or replace it; never combine uncontrolled concurrent implementations.
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
6. Flag issues whose acceptance criteria conflict, duplicate merged work, require a material product decision, or cannot yet be verified.

Report the complete ordered queue, dependency rationale, milestone grouping, and recommended execution method. Do not mutate issue labels, milestones, bodies, or state in Portfolio mode.

## Triage and design checkpoint

Before editing, read the complete issue and discussion, relevant source, tests, docs, ADRs, related merged work, and dependencies. Then state:

1. The reproduced or code-confirmed failure and why it matters.
2. An acceptance-criteria traceability table mapping every criterion to the intended code, test, documentation, or explicit blocker.
3. The smallest robust design and deliberate non-goals.
4. Compatibility, stable-output/error, resource-bound, security, platform, and serialization risks.
5. Exact files and adversarial/regression tests to add.

Stop for direction before a material API, serialized-format, identity-domain, dependency, portability-policy, or issue-scope expansion. Do not silently resolve conflicting acceptance criteria.

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

## Completion report

Report actual state, not intended state: selected modes, issue and branch, commit/PR/merge URLs, acceptance-criteria disposition, checks passed, checks failed or unavailable, review-thread disposition, issue comment/closure state, limitations, and the highest-priority unblocked next issue. Never create an empty PR or choose the next issue by number alone.
