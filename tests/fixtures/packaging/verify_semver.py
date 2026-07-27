#!/usr/bin/env python3
"""Report semver-incompatible changes against the published baseline.

This crate is pre-1.0 and says so in `CHANGELOG.md`: any release may change the
public API, the serialized shapes, the canonical bytes, and every identity
computed over them. A breaking change is therefore *permitted*, and a gate that
hard-failed on one would fight intended work. Nor is a warning nobody must act
on worth much — that is how drift gets through.

So the rule here is neither "no breaks" nor "breaks are free". It is **no
undocumented breaks**: `cargo semver-checks` reports what changed, and a
reported break must be acknowledged in the `## Unreleased` section of
`CHANGELOG.md` with a line containing `**BREAKING**`. Acknowledged, it passes;
unacknowledged, it fails and names what to write.

Two surfaces are checked, because they are two published APIs: the base model a
`default-features = false` consumer sees, and the full surface. An item that
moves from one to the other is breaking for the base consumer even when the full
surface is unchanged.

`--release-type patch` is not cosmetic. Without it, a branch whose version has
not been bumped compares `X -> X`, which `cargo semver-checks` reads as
"no change; assume major" and answers by running *zero* checks — a gate that
passes because it asked nothing. Forcing the patch question ("is this change
patch-compatible?") makes every breaking change visible regardless of whether a
version bump has happened yet, which is what a pull-request gate needs.

Usage:
    verify_semver.py [--baseline-version X.Y.Z] [--offline-baseline X.Y.Z]

Exit codes:
    0  no breaking change, or every break acknowledged in the changelog
    1  breaking changes are not acknowledged in CHANGELOG.md
    2  the tool could not run, or no published baseline exists
"""

import argparse
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

CRATE = "candid-core"
# `cargo semver-checks` exits 100 when a version bump is required. Any other
# non-zero exit is a tool or environment failure and must not be read as "no
# breaking changes".
SEMVER_REQUIRED_EXIT = 100

SURFACES = (
    ("base", ["--only-explicit-features"]),
    ("all-features", ["--all-features"]),
)


def fail(message: str, code: int = 2) -> None:
    print(f"verify_semver: {message}", file=sys.stderr)
    sys.exit(code)


def published_baseline(repo_root: Path) -> str:
    """The version a consumer would be upgrading from.

    Prefer the latest stable release; fall back to the newest version overall,
    which is what applies while the crate has only ever shipped prereleases.
    """
    url = f"https://crates.io/api/v1/crates/{CRATE}"
    request = urllib.request.Request(
        url, headers={"User-Agent": f"{CRATE} semver gate"}
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.load(response)
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        fail(f"could not read published versions from crates.io: {error}")
    crate = payload.get("crate", {})
    version = crate.get("max_stable_version") or crate.get("newest_version")
    if not version:
        fail(
            f"{CRATE} has no published version to compare against; "
            "semver comparison begins after the first release"
        )
    return version


def unreleased_section(changelog: Path) -> str:
    """The text between `## Unreleased` and the next `## ` heading."""
    if not changelog.is_file():
        fail(f"{changelog} is missing")
    text = changelog.read_text()
    match = re.search(
        r"^##\s+Unreleased\s*$(.*?)(?=^##\s+)", text, re.MULTILINE | re.DOTALL
    )
    return match.group(1) if match else ""


def run_surface(name: str, flags: list, baseline: str, repo_root: Path) -> bool:
    """True when this surface is semver-clean; False when a break was reported."""
    command = [
        "cargo",
        "semver-checks",
        "check-release",
        "--baseline-version",
        baseline,
        "--release-type",
        "patch",
        *flags,
    ]
    print(f"\n--- {name} surface vs {CRATE} {baseline} ---")
    print(f"$ {' '.join(command)}")
    result = subprocess.run(command, cwd=repo_root)
    if result.returncode == 0:
        return True
    if result.returncode == SEMVER_REQUIRED_EXIT:
        return False
    fail(
        f"cargo semver-checks exited {result.returncode} on the {name} surface. "
        "That is a tool or environment failure, not a clean result."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--baseline-version",
        help="compare against this exact published version instead of resolving one",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[3]
    baseline = args.baseline_version or published_baseline(repo_root)
    print(f"baseline: {CRATE} {baseline}")

    broken = [
        name
        for name, flags in SURFACES
        if not run_surface(name, list(flags), baseline, repo_root)
    ]

    print()
    if not broken:
        print(f"no semver-incompatible change against {CRATE} {baseline}")
        return 0

    surfaces = ", ".join(broken)
    acknowledgement = unreleased_section(repo_root / "CHANGELOG.md")
    if "**BREAKING**" in acknowledgement:
        print(
            f"breaking changes reported on: {surfaces}\n"
            "acknowledged in the Unreleased section of CHANGELOG.md — allowed, "
            "because this crate is pre-1.0 and documents that any release may "
            "break."
        )
        return 0

    print(
        f"breaking changes reported on: {surfaces}\n"
        "\n"
        "This is permitted before 1.0, but it must be written down. Add a line "
        "containing **BREAKING** to the `## Unreleased` section of CHANGELOG.md "
        "describing what changed and what a consumer must do, then re-run.\n"
        "\n"
        "If the change was not intended, revert it instead — that is the case "
        "this gate exists to catch.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
