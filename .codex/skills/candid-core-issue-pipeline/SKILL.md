---
name: candid-core-issue-pipeline
description: Handle Candid Core GitHub security, design-remediation, and maintenance issues through a safe local issue-to-merge workflow. Use when triaging, implementing, reviewing, publishing, merging, or closing a Candid Core issue or pull request, especially for requests that mention an issue number, PR number, security remediation, review threads, or Codex cloud task results.
---

# Candid Core Issue Pipeline

Use this workflow for `b3hr4d/candid-core`. Read the repository `AGENTS.md` and every file it references before acting; those instructions override this skill. Prefix every terminal command with `rtk`.

## Select a mode

- **Triage**: Inspect only. Explain the present bug, acceptance criteria, affected code, risks, and smallest robust design. Do not edit.
- **Implement**: Own one issue locally from a fresh `main` branch through merge and issue closure.
- **Review**: Inspect a completed branch or PR without editing it. Treat every unresolved or outdated review thread as potentially valid until compared with final code.
- **Close**: Perform post-merge issue housekeeping only after independently confirming every acceptance criterion.

Treat a named mode as binding. Infer `implement` only when the user clearly asks for a change.

## Ownership and state

1. Verify GitHub and the local checkout instead of trusting summaries. Inspect issue state, related PRs, checks, reviews, review threads, branch, worktree status, and `main` freshness.
2. Permit exactly one implementation owner for an issue. Do not implement concurrently in local and cloud tasks.
3. Let cloud tasks perform bounded research, a candidate patch, or read-only review. Before using a cloud result, inspect its final diff and actual tests locally. Do not trust task summaries.
4. If a cloud task produced a candidate implementation, either adopt it as the sole owner or replace it locally; do not combine uncontrolled concurrent edits. Archive superseded tasks when allowed.
5. Keep one issue per branch and PR. Preserve unrelated user changes. Never use destructive Git commands unless the user explicitly requests them.

## Triage and design checkpoint

Before editing, refresh `main`, read the complete issue and discussion, suggested implementation, relevant source, tests, docs, ADRs, and related merged PRs. Then state:

1. The observed failure and why it matters.
2. The smallest robust design and deliberate non-goals.
3. Compatibility, stable-output/error, resource-bound, and security risks.
4. Exact files and adversarial/regression tests to add.

Do not make a material API, serialized-format, dependency, or scope expansion without flagging it to the user first.

## Implement and verify

1. Create an issue-specific branch from refreshed `main`.
2. Make the narrowest change that satisfies the acceptance criteria. Keep contract-directed validation separate from syntax or local canonicalization where applicable.
3. Add focused regression and adversarial tests proportional to risk. Assert stable codes, paths, resource metadata, and precedence when those are part of the public behavior.
4. Run repository-required formatting, focused tests, complete debug tests, Clippy with warnings denied, release tests, and the advertised MSRV when installed. Use locked/offline flags when the project supports them. Report unavailable checks honestly; do not claim them as passed.
5. Review the final diff skeptically for validation bypasses, changed error precedence, resource exhaustion, unstable serialization or diagnostics, compatibility regressions, hidden allocations, and unrelated changes. Run a whitespace/error check when available.

## Publish, review, and merge

1. **PR-first by default:** after implementation review and required checks pass, push only the issue branch and create a PR. Never push an implementation commit directly to `main`, and never treat pushing a branch as permission to update `main`.
2. A direct-to-`main` push is permitted only when the user explicitly authorizes that exception in the current turn. Before it, verify that `origin/main` is fresh and the commit is a fast-forward; report the exception in the completion report.
3. Make the PR body state scope, deliberate API impact, exact verification, and any limitation.
4. Inspect all PR reviews, review comments, and unresolved threads before merging. Compare even outdated comments with the final code; fix every valid actionable issue in a narrow follow-up commit.
5. Merge only when the PR is clean, verified, conflict-free, and its acceptance criteria are satisfied. Do not merge merely because checks are green.
6. After merge, refresh `main`, verify the merge commit and issue state, then comment on the issue with the merged PR link, concise fix, performed checks, and remaining limitation. Close the issue only then.

## Completion report

Report actual state, not intended state: branch/PR/merge URL, checks that passed, checks unavailable or failed, review-thread disposition, issue comment/closure state, and the next numeric issue. Never create an empty PR just to advance the workflow.
