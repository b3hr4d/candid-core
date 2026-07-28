#!/usr/bin/env python3
"""Verify what each `candid-core` feature set puts in a consumer's dependency graph.

Issue #24 split one published package into a base Contract model plus the
`host-value`, `compiler`, and `filesystem-compiler` features. The claim that
makes that split worth anything is a claim about *dependency graphs*: a pure
Contract consumer must not be made to build a Candid source engine, and a
consumer that never touches a filesystem must not be made to build a filesystem
capability crate. This script checks that claim with `cargo tree`, resolving
what Cargo would build for a consumer of the selected feature set.

Standard library only, so it runs anywhere `cargo` and `python3` do:

    python3 tests/fixtures/packaging/verify_feature_graph.py

Three deliberate scoping decisions:

* **Dev-dependencies are excluded from resolution itself** (`--edges no-dev`),
  not merely from a walk over a resolved graph. Dev-dependencies exist to test
  this repository and never appear in a downstream consumer's graph — and that
  now includes *sibling workspace members'* dev-dependencies: the generator
  crate's golden tests dev-depend on `candid-core` with `compiler` on, and a
  whole-workspace resolve (what `cargo metadata` computes) unifies that feature
  into `candid-core` before any edge filtering can help. This script previously
  used `cargo metadata` with a normal/build edge walk, which was sound while
  the package was alone in its workspace and silently stopped being sound when
  it gained a sibling.
* **`candid_parser` as a dev-dependency is not a leak.** The tests compare the
  crate's internal Candid name hash against the upstream reference in every
  feature configuration — including the one where the library does not link it.
* **The graph is resolved per target triple** with `--target`, because
  `cap-std` is declared under `cfg(not(target_os = "unknown"))` as well as
  behind `filesystem-compiler`. Browser WASM therefore has no `cap-std` even
  with default features, and the expectations below say so per target rather
  than pretending the answer is target-independent.

This lives beside the (future) `.crate` archive-content checks: feature
selection bounds what a consumer must *build*, while archive policy bounds what
a consumer must *download*. They are separate gates on the same question and
both belong in this directory.
"""

import subprocess
import sys
from pathlib import Path

MANIFEST = Path(__file__).resolve().parents[3] / "Cargo.toml"

# The Candid source engine and the filesystem capability crate, by package name.
CANDID = "candid"
CANDID_PARSER = "candid_parser"
CAP_STD = "cap-std"
IC_PRINCIPAL = "ic_principal"

# Packages that can only arrive through `candid_parser`. Naming the engine
# crates alone would pass even if a future refactor depended on the parser's
# generator stack directly, so the absence claim names the stack too.
PARSER_STACK = ("lalrpop-util", "codespan-reporting", "handlebars", "logos", "leb128")

WASM = "wasm32-unknown-unknown"


def host_triple() -> str:
    output = subprocess.run(
        ["rustc", "-vV"], check=True, capture_output=True, text=True
    ).stdout
    for line in output.splitlines():
        if line.startswith("host: "):
            return line[len("host: ") :].strip()
    raise SystemExit("cannot determine the host target triple from `rustc -vV`")


def graph(features: str, target: str) -> set:
    """Package names in candid-core's consumer graph for one feature set.

    `features` is a comma-separated feature list applied on top of
    `--no-default-features`, or the sentinel `"default"` / `"all"`.

    `--edges no-dev` keeps normal, build, and proc-macro dependencies and —
    unlike an edge walk over a `cargo metadata` resolve — removes
    dev-dependencies from feature unification too, so a sibling workspace
    member's test-only requirements cannot activate this package's optional
    dependencies. `--format {p}` prints one package spec per line; the leading
    token is the package name.
    """
    command = [
        "cargo",
        "tree",
        "--locked",
        "--manifest-path",
        str(MANIFEST),
        "--package",
        "candid-core",
        "--edges",
        "no-dev",
        "--target",
        target,
        "--prefix",
        "none",
        "--format",
        "{p}",
    ]
    if features == "all":
        command.append("--all-features")
    elif features != "default":
        command.append("--no-default-features")
        if features:
            command += ["--features", features]

    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        # A raw traceback here would hide cargo's own diagnostic — which is the
        # actionable part, e.g. `--locked` refusing a stale lockfile.
        sys.stderr.write(result.stderr)
        raise SystemExit(f"`{' '.join(command)}` exited {result.returncode}")
    output = result.stdout
    packages = set()
    for line in output.splitlines():
        line = line.strip()
        if line:
            packages.add(line.split(" ", 1)[0])
    if "candid-core" not in packages:
        raise SystemExit(
            "cargo tree output does not contain candid-core itself; "
            "refusing to treat an empty or mis-scoped graph as evidence"
        )
    return packages


def check(label, features, target, required=(), forbidden=()):
    packages = graph(features, target)
    failures = []
    for name in required:
        if name not in packages:
            failures.append(f"expected {name} in the graph")
    for name in forbidden:
        if name in packages:
            failures.append(f"{name} must not be in the graph")
    status = "ok" if not failures else "FAIL"
    print(f"[{status}] {label} ({len(packages)} packages, {target})")
    for failure in failures:
        print(f"       {failure}")
    return failures


def main() -> int:
    host = host_triple()
    engine = (CANDID, CANDID_PARSER, *PARSER_STACK)
    failures = []

    # A) A pure Contract consumer builds no Candid source engine, no filesystem
    #    capability crate, and no principal codec.
    failures += check(
        "base (default-features = false)",
        "",
        host,
        required=("serde", "serde_json", "sha2", "hex"),
        forbidden=(*engine, CAP_STD, IC_PRINCIPAL),
    )

    # C) HostValue is isolated: it adds the principal codec and nothing else.
    failures += check(
        "host-value only",
        "host-value",
        host,
        required=(IC_PRINCIPAL,),
        forbidden=(*engine, CAP_STD),
    )

    # B) The source compiler is separable from the filesystem.
    failures += check(
        "compiler only",
        "compiler",
        host,
        required=(CANDID, CANDID_PARSER),
        forbidden=(CAP_STD,),
    )
    failures += check(
        "compiler only, browser WASM",
        "compiler",
        WASM,
        required=(CANDID, CANDID_PARSER),
        forbidden=(CAP_STD,),
    )

    # B) The native filesystem stack arrives only with filesystem-compiler, and
    #    only on a target that has a filesystem.
    failures += check(
        "filesystem-compiler",
        "filesystem-compiler",
        host,
        required=(CANDID, CANDID_PARSER, CAP_STD),
    )
    failures += check(
        "default features",
        "default",
        host,
        required=(CANDID, CANDID_PARSER, CAP_STD, IC_PRINCIPAL),
    )
    failures += check(
        "all features",
        "all",
        host,
        required=(CANDID, CANDID_PARSER, CAP_STD, IC_PRINCIPAL),
    )
    # `cap-std` is declared under `cfg(not(target_os = "unknown"))` as well as
    # behind the feature, so browser WASM never receives it — not even with
    # every feature on. That is what keeps `cargo check --target
    # wasm32-unknown-unknown` green with default features.
    failures += check(
        "default features, browser WASM",
        "default",
        WASM,
        required=(CANDID, CANDID_PARSER, IC_PRINCIPAL),
        forbidden=(CAP_STD,),
    )
    failures += check(
        "base, browser WASM",
        "",
        WASM,
        forbidden=(*engine, CAP_STD, IC_PRINCIPAL),
    )

    if failures:
        print(f"\n{len(failures)} dependency-boundary expectation(s) failed")
        return 1
    print("\nevery dependency-boundary expectation holds")
    return 0


if __name__ == "__main__":
    sys.exit(main())
