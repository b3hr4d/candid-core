# Agent entrypoint

Read [`CLAUDE.md`](CLAUDE.md) — it is the single source for how this
repository works, regardless of which assistant discovered this file. Then
read `.codex/skills/candid-core-issue-pipeline/SKILL.md` for the working modes
(triage, implement, publish, review, merge, close, release).

This file exists because different assistants auto-discover different
entrypoints (`AGENTS.md` here, `CLAUDE.md` there); the content lives once, in
`CLAUDE.md`, and this shim only converges the entrypoints. Do not add
instructions here.
