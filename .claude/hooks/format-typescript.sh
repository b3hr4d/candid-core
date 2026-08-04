#!/bin/bash
# Format a TypeScript file the moment an agent finishes writing it.
#
# Wired as a PostToolUse hook on Edit/Write/MultiEdit (.claude/settings.json).
# The point is that agent-authored TypeScript lands already formatted, so a
# human editing the same file next never inherits a reformat: the diff a
# maintainer reads is the change, not the change plus a whitespace sweep.
#
# Deliberately narrow. It formats one file, only when that file is TypeScript
# Prettier is allowed to touch, using the repo-root .prettierrc.json — the same
# binary, config, and ignore list as `npm run format`, the pre-commit hook, and
# CI, so no two surfaces can disagree about what "formatted" means.
#
# Saving a file is not one of these surfaces, and must never become one.
set -uo pipefail

root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -n "${root}" ] || exit 0
prettier="${root}/crates/candid-core-ts/ts/node_modules/.bin/prettier"

# A fresh clone has no node_modules. An agent must still be able to edit files
# there, so a missing formatter is a no-op rather than a broken toolchain.
[ -x "${prettier}" ] || exit 0

# The hook contract delivers the tool call on stdin as JSON. jq is not assumed:
# python3 is already this repository's scripting dependency (the packaging and
# conformance verifiers are standard-library Python).
file="$(python3 -c '
import json, sys
try:
    payload = json.load(sys.stdin)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)
value = (payload.get("tool_input") or {}).get("file_path") or ""
sys.stdout.write(value if isinstance(value, str) else "")
' 2>/dev/null)"

case "${file}" in
  *.ts | *.tsx | *.mts | *.cts) ;;
  *) exit 0 ;;
esac
[ -f "${file}" ] || exit 0

# Ask Prettier itself whether the path is in scope rather than re-deriving the
# ignore rules here. A second copy of that logic is a second thing to drift:
# this is what keeps generated goldens out (`"ignored": true`).
ignored="$(cd "${root}" && "${prettier}" --file-info "${file}" 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin).get("ignored", True))' 2>/dev/null)"
[ "${ignored}" = "False" ] || exit 0

if ! error="$(cd "${root}" && "${prettier}" --write --log-level warn "${file}" 2>&1)"; then
  # The file no longer parses. Exit 2 puts stderr in front of the agent, which
  # is the right moment to fix it — a syntax error found now is cheaper than
  # the same error found by tsc three edits later.
  printf 'Prettier could not parse %s after this edit:\n%s\n' "${file}" "${error}" >&2
  exit 2
fi

exit 0
