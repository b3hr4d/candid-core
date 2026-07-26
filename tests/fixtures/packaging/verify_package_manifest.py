#!/usr/bin/env python3
"""Assert exactly what the published `.crate` archive contains.

`Cargo.toml` carries a positive `include` allowlist rather than a growing
`exclude` list, and this script is the half that makes the allowlist hold. It
checks three separate things, because each one fails a different way:

1. **Nothing internal ships.** Test suites and their fixtures, benchmarks and
   their corpus, the fuzz crate, CI workflows, agent/skill assets, and any local
   scratch directory in a contributor's working tree must all be absent. An
   untracked directory sitting next to `src/` is packaged by default, so absence
   has to be asserted rather than assumed.

2. **Nothing required is missing.** Every `src/**.rs`, `examples/**.rs`, and
   `docs/**.md` in the working tree must be in the archive, along with the three
   root documents and the manifest material Cargo adds itself. A source file
   that exists locally but is not packaged compiles here and fails for the first
   consumer who installs the crate.

3. **The manifest itself has not drifted.** The `include` list, the packaging
   metadata crates.io renders, and the license are compared against the values
   recorded here, so relaxing the allowlist is a deliberate edit to this file
   rather than a silent change to what ships.

Standard library only, and `cargo` is the only subprocess: packaging
verification must not become a dependency of the crate it verifies.

Usage:
    python3 tests/fixtures/packaging/verify_package_manifest.py [extra cargo args]

Any extra arguments are passed through to `cargo package --list`, which is how
a contributor with an untracked scratch directory passes `--allow-dirty`. CI
passes `--locked` and nothing else.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]

# Cargo synthesizes these regardless of `include`; they are expected, and their
# absence means something changed about how Cargo builds archives.
CARGO_GENERATED = {
    "Cargo.toml",
    "Cargo.toml.orig",
    "Cargo.lock",
    ".cargo_vcs_info.json",
}

# The root documents the allowlist names by hand.
REQUIRED_ROOT_DOCUMENTS = {
    "README.md",
    "CHANGELOG.md",
    "LICENSE",
}

# Individually named because each one has a distinct failure mode: `lib.rs` is
# the crate, `bin/candid-core.rs` is the CLI a `cargo install` smoke needs, and
# `bounded.rs` is reached only through a `#[path]` attribute in the binary, so
# no `mod` declaration in `lib.rs` would reveal its absence.
REQUIRED_SOURCE_FILES = {
    "src/lib.rs",
    "src/bin/candid-core.rs",
    "src/bounded.rs",
}

# Directories that must never appear, whatever a working tree happens to hold.
# The check is a prefix test, so a path anywhere beneath one of these fails.
FORBIDDEN_PREFIXES = (
    ".cargo/",
    ".claude/",
    ".codex/",
    ".github/",
    ".vscode/",
    "benches/",
    "candid-scope/",
    "fuzz/",
    "target/",
    "tests/",
)

# Root files that are repository infrastructure and must not ship.
FORBIDDEN_EXACT = {
    ".gitattributes",
    ".gitexclude-local",
    ".gitignore",
    "deny.toml",
    "rustfmt.toml",
}

# The allowlist as it must appear in `Cargo.toml`. Widening the published source
# set means editing both this tuple and the manifest, in the same change.
EXPECTED_INCLUDE = [
    "/src/**/*.rs",
    "/examples/**/*.rs",
    "/docs/**/*.md",
    "/README.md",
    "/CHANGELOG.md",
    "/LICENSE",
]

# Packaging metadata crates.io and docs.rs render. Checked for exact value, not
# merely for presence, so a typo in a URL is a test failure.
EXPECTED_METADATA = {
    "license": "Apache-2.0",
    "repository": "https://github.com/b3hr4d/candid-core",
    "homepage": "https://github.com/b3hr4d/candid-core",
    "documentation": "https://docs.rs/candid-core",
    "readme": "README.md",
}
EXPECTED_KEYWORDS = [
    "candid",
    "did",
    "canonicalization",
    "content-addressing",
    "internet-computer",
]
EXPECTED_CATEGORIES = ["encoding", "data-structures", "development-tools"]


class Failure(Exception):
    """A packaging assertion that did not hold."""


def run(command: list[str]) -> str:
    result = subprocess.run(
        command,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise Failure(
            f"{' '.join(command)} exited {result.returncode}\n{result.stderr.strip()}"
        )
    return result.stdout


def packaged_paths(extra_args: list[str]) -> set[str]:
    listing = run(["cargo", "package", "--list", "--quiet", *extra_args])
    return {line.strip() for line in listing.splitlines() if line.strip()}


# Published directories, and the one file extension the allowlist matches in
# each. Anything else living under one of these is either an editor dropping or
# an asset somebody forgot to add to `include`; `check_published_directories`
# tells the two apart by name.
PUBLISHED_DIRECTORIES = {"src": ".rs", "examples": ".rs", "docs": ".md"}

# Never publishable and never a mistake: OS and tooling droppings.
IGNORABLE_NAMES = {".DS_Store", "Thumbs.db", ".gitkeep"}
IGNORABLE_PARTS = {"__pycache__", ".ipynb_checkpoints"}


def working_tree_sources() -> set[str]:
    """Every file the allowlist claims to publish, as it exists on disk."""
    found: set[str] = set()
    for directory, suffix in PUBLISHED_DIRECTORIES.items():
        for path in sorted((REPO_ROOT / directory).rglob(f"*{suffix}")):
            found.add(path.relative_to(REPO_ROOT).as_posix())
    return found


def working_tree_strays() -> set[str]:
    """Files under a published directory that the allowlist does not match.

    This is the check that catches an omission rustdoc or an example needs. A
    `src/table.json` behind an `include_str!`, or a `docs/diagram.svg` an ADR
    links to, compiles and renders here and is simply absent from the archive —
    the first person to learn about it is a consumer whose build fails. Adding
    such an asset is fine; adding it *without* widening `include` is not.
    """
    strays: set[str] = set()
    for directory, suffix in PUBLISHED_DIRECTORIES.items():
        for path in sorted((REPO_ROOT / directory).rglob("*")):
            if not path.is_file() or path.suffix == suffix:
                continue
            if path.name in IGNORABLE_NAMES:
                continue
            if IGNORABLE_PARTS & set(path.parts):
                continue
            strays.add(path.relative_to(REPO_ROOT).as_posix())
    return strays


def manifest() -> dict:
    """The `[package]` table as Cargo resolved it, plus the keys `cargo metadata`
    does not expose.

    `include` and `exclude` are absent from `cargo metadata` output at every
    format version, so the allowlist itself has to be read from the manifest
    text. `tomllib` is standard library from Python 3.11, which keeps this script
    dependency-free.
    """
    metadata = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1"]))
    package = next(
        (entry for entry in metadata["packages"] if entry["name"] == "candid-core"),
        None,
    )
    if package is None:
        raise Failure("cargo metadata did not report a candid-core package")

    with (REPO_ROOT / "Cargo.toml").open("rb") as handle:
        declared = tomllib.load(handle)["package"]
    package["include"] = declared.get("include")
    package["exclude"] = declared.get("exclude")
    return package


def locked_version(lockfile: Path) -> str | None:
    """The `candid-core` version recorded in a lockfile.

    Both lockfiles carry it, and a bump that updates the manifest but not a
    lockfile is exactly the mistake a `--locked` build reports as an opaque
    "the lock file needs to be updated". Naming it here makes the failure
    readable.
    """
    with lockfile.open("rb") as handle:
        for package in tomllib.load(handle).get("package", []):
            if package.get("name") == "candid-core":
                return package.get("version")
    return None


def check_nothing_internal_ships(packaged: set[str], failures: list[str]) -> None:
    for path in sorted(packaged):
        for prefix in FORBIDDEN_PREFIXES:
            if path.startswith(prefix):
                failures.append(f"forbidden path packaged: {path} (under {prefix})")
        if path in FORBIDDEN_EXACT:
            failures.append(f"forbidden file packaged: {path}")


def check_nothing_required_is_missing(packaged: set[str], failures: list[str]) -> None:
    for path in sorted(CARGO_GENERATED | REQUIRED_ROOT_DOCUMENTS | REQUIRED_SOURCE_FILES):
        if path not in packaged:
            failures.append(f"required file missing from the archive: {path}")

    for path in sorted(working_tree_sources() - packaged):
        failures.append(f"present in the working tree but not packaged: {path}")

    for path in sorted(working_tree_strays() - packaged):
        directory = path.split("/", 1)[0]
        failures.append(
            f"{path} lives under {directory}/ but the allowlist only matches "
            f"*{PUBLISHED_DIRECTORIES[directory]} there, so it will not ship; "
            "widen `include` deliberately or move the file out"
        )

    # Category-level floors, so an `include` pattern that silently matches
    # nothing — a renamed directory, a typo in a glob — cannot pass by having
    # every individually named file still present.
    for prefix, minimum in (("src/", 20), ("examples/", 6), ("docs/", 10)):
        count = sum(1 for path in packaged if path.startswith(prefix))
        if count < minimum:
            failures.append(
                f"only {count} packaged paths under {prefix}; expected at least {minimum}"
            )


def check_no_unexpected_paths(packaged: set[str], failures: list[str]) -> None:
    """The drift guard: every packaged path must be explained by a rule above."""
    allowed_prefixes = ("src/", "examples/", "docs/")
    for path in sorted(packaged):
        if path in CARGO_GENERATED or path in REQUIRED_ROOT_DOCUMENTS:
            continue
        if path.startswith(allowed_prefixes):
            expected_suffix = ".md" if path.startswith("docs/") else ".rs"
            if not path.endswith(expected_suffix):
                failures.append(
                    f"unexpected file type packaged: {path} (expected {expected_suffix})"
                )
            continue
        failures.append(f"unexpected path packaged: {path}")


def check_manifest_has_not_drifted(package: dict, failures: list[str]) -> None:
    include = package.get("include") or []
    if include != EXPECTED_INCLUDE:
        failures.append(f"Cargo.toml include drifted:\n  found:    {include}\n  expected: {EXPECTED_INCLUDE}")

    for key, expected in EXPECTED_METADATA.items():
        found = package.get(key)
        if found != expected:
            failures.append(f"Cargo.toml {key} is {found!r}, expected {expected!r}")

    if package.get("keywords") != EXPECTED_KEYWORDS:
        failures.append(
            f"Cargo.toml keywords are {package.get('keywords')!r}, expected {EXPECTED_KEYWORDS!r}"
        )
    if package.get("categories") != EXPECTED_CATEGORIES:
        failures.append(
            f"Cargo.toml categories are {package.get('categories')!r}, expected {EXPECTED_CATEGORIES!r}"
        )

    # crates.io enforces both of these server-side, where the only way to learn
    # about a violation is a rejected publish.
    if len(EXPECTED_KEYWORDS) > 5:
        failures.append("crates.io accepts at most 5 keywords")
    for keyword in EXPECTED_KEYWORDS:
        if len(keyword) > 20:
            failures.append(f"keyword {keyword!r} exceeds the 20-character crates.io limit")
    if len(EXPECTED_CATEGORIES) > 5:
        failures.append("crates.io accepts at most 5 categories")

    if package.get("exclude"):
        failures.append(
            "Cargo.toml declares `exclude`; this package uses a positive `include` "
            "allowlist and mixing the two makes what ships hard to reason about"
        )

    if not (REPO_ROOT / "LICENSE").is_file():
        failures.append("LICENSE is declared by `license = \"Apache-2.0\"` but is not on disk")

    # One version, three files. `--locked` reports a stale lockfile as an opaque
    # resolver error; this says which file was left behind.
    version = package["version"]
    for lockfile in (REPO_ROOT / "Cargo.lock", REPO_ROOT / "fuzz" / "Cargo.lock"):
        found = locked_version(lockfile)
        if found != version:
            failures.append(
                f"{lockfile.relative_to(REPO_ROOT).as_posix()} records candid-core "
                f"{found!r}, but Cargo.toml declares {version!r}"
            )


def main() -> int:
    extra_args = sys.argv[1:]
    failures: list[str] = []

    try:
        packaged = packaged_paths(extra_args)
        package = manifest()
    except Failure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1

    check_nothing_internal_ships(packaged, failures)
    check_nothing_required_is_missing(packaged, failures)
    check_no_unexpected_paths(packaged, failures)
    check_manifest_has_not_drifted(package, failures)

    if failures:
        print(f"package manifest verification FAILED ({len(failures)} problem(s))", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print(f"package manifest verified: {len(packaged)} paths")
    for prefix in ("src/", "examples/", "docs/"):
        print(f"  {prefix:<12}{sum(1 for path in packaged if path.startswith(prefix))}")
    print(f"  {'root':<12}{len(CARGO_GENERATED | REQUIRED_ROOT_DOCUMENTS)}")
    print(f"  version     {package['version']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
