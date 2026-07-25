#!/usr/bin/env python3
"""Verify what each `candid-core` feature set puts in a consumer's dependency graph.

Issue #24 split one published package into a base Contract model plus the
`host-value`, `compiler`, and `filesystem-compiler` features. The claim that
makes that split worth anything is a claim about *dependency graphs*: a pure
Contract consumer must not be made to build a Candid source engine, and a
consumer that never touches a filesystem must not be made to build a filesystem
capability crate. This script checks that claim directly against
`cargo metadata`, which resolves exactly what Cargo would build.

Standard library only, so it runs anywhere `cargo` and `python3` do:

    python3 tests/fixtures/packaging/verify_feature_graph.py

Two deliberate scoping decisions:

* **Only normal and build dependencies are followed.** Dev-dependencies exist
  to test *this* package and never appear in a downstream consumer's graph.
  `candid_parser` is a dev-dependency precisely so the tests can compare the
  crate's internal Candid name hash against the upstream reference in every
  feature configuration — including the one where the library does not link it.
  Counting that as a leak would be wrong.
* **The graph is resolved per target triple** with `--filter-platform`, because
  `cap-std` is declared under `cfg(not(target_os = "unknown"))` as well as
  behind `filesystem-compiler`. Browser WASM therefore has no `cap-std` even
  with default features, and the expectations below say so per target rather
  than pretending the answer is target-independent.

This lives beside the (future) `.crate` archive-content checks: feature
selection bounds what a consumer must *build*, while archive policy bounds what
a consumer must *download*. They are separate gates on the same question and
both belong in this directory.
"""

import json
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
    """Package names reachable from candid-core over normal/build edges only.

    `features` is a comma-separated feature list applied on top of
    `--no-default-features`, or the sentinel `"default"` / `"all"`.
    """
    command = [
        "cargo",
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--manifest-path",
        str(MANIFEST),
        "--filter-platform",
        target,
    ]
    if features == "all":
        command.append("--all-features")
    elif features != "default":
        command.append("--no-default-features")
        if features:
            command += ["--features", features]

    metadata = json.loads(
        subprocess.run(command, check=True, capture_output=True, text=True).stdout
    )
    names = {package["id"]: package["name"] for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]

    reached, pending = set(), [root]
    while pending:
        current = pending.pop()
        if current in reached:
            continue
        reached.add(current)
        for dependency in nodes[current]["deps"]:
            kinds = {
                entry.get("kind")
                for entry in dependency.get("dep_kinds", [{"kind": None}])
            }
            # `None` is a normal dependency; "build" is a build script's. Only
            # "dev" is excluded.
            if kinds & {None, "build"}:
                pending.append(dependency["pkg"])
    return {names[identifier] for identifier in reached}


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
