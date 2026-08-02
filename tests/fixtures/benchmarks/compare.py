#!/usr/bin/env python3
"""Capture and compare benchmark baselines.

A percentage delta between two runs is meaningless unless both measured the same
corpus, with the same feature set, on comparable machinery. This tool makes that
precondition checkable instead of assumed, and it is built to *refuse* rather
than to guess: a missing or incompatible baseline reports that no comparison was
made and exits non-zero. It never renders a delta it cannot stand behind, which
is the one property that makes a warning worth reading.

Two halves make up a run's identity. `benches/manifest.rs` emits what only the
Rust side knows — corpus fingerprints, generator sizes, feature set, metric
units. This script records what only it can observe reliably — toolchain,
target, host, and the resolved dependency graph of the bench binary — and
stores both together, along with advisory machine identity (CPU model, core
count, runner image) that is rendered for the reader but deliberately never
part of the drift check: hosted-runner hardware varies run to run by design,
and issue #39 already records that such variance must inform, not gate.

The dependency identity is a projection, not a file hash. Hashing the whole
workspace `Cargo.lock` over-identifies: the root package's own version stanza
and sibling workspace members' dev-dependency lists change its bytes without
changing the program `cargo bench` builds, so a release version bump would
permanently invalidate every baseline (issue #132). Instead the identity is
the closure of packages reachable from the root package's lockfile stanza —
whose dependency list already merges normal, build, and dev dependencies; dev
included deliberately, benches compile against them — excluding the root
package itself.

Standard library only, like every other verifier in this repository.

Usage
-----

Capture a baseline on `main`:

    cargo bench --bench manifest --locked > /tmp/manifest.json
    cargo bench --bench compilation --locked -- --noplot
    cargo bench --bench allocation --locked > /tmp/allocations.json
    python3 tests/fixtures/benchmarks/compare.py capture \\
        --manifest /tmp/manifest.json --criterion target/criterion \\
        --allocations /tmp/allocations.json --out benches/baselines/main.json

Compare a candidate against it (same three commands first, then):

    python3 tests/fixtures/benchmarks/compare.py compare \\
        --baseline benches/baselines/main.json --manifest /tmp/manifest.json \\
        --criterion target/criterion --allocations /tmp/allocations.json \\
        --markdown /tmp/report.md

Exit codes
----------

    0  compared; no gate was requested, or none was exceeded
    1  compared; a requested --fail-on-regression gate was exceeded
    2  no comparison was made (baseline missing, unreadable, or incompatible)
"""

import argparse
import json
import os
import platform
import statistics
import subprocess
import sys
import tomllib
from pathlib import Path

# Bumped when the *stored baseline* layout changes. Independent of the Rust
# manifest's own schema version, which describes what was measured.
# Version 2: `lockfile_digest` (whole-file hash) replaced by
# `bench_graph_digest` (bench dependency-graph projection), the graph itself
# stored for drift attribution, and advisory machine identity recorded.
# Schema-1 baselines refuse rather than half-compare; recapture per
# docs/benchmarks.md.
BASELINE_SCHEMA_VERSION = 2

# Manifest fields that must match exactly for a comparison to mean anything.
# A difference in any of them means the two runs measured different programs or
# different inputs, so no delta between them is interpretable.
HARD_KEYS = (
    "schema_version",
    "corpus_id",
    "generator_sizes",
    "features",
    "metric_units",
)

# Environment fields. A difference here means the same program was measured on
# different machinery: the comparison may still be informative, but it is not
# evidence, so it requires --allow-environment-drift and can never fail a gate.
# Every metric the allocation probe captures. All three are compared: a change
# that holds the allocation count constant while growing cumulative or peak
# bytes is a real memory regression, and comparing only the count would render
# it as 0%.
ALLOCATION_METRICS = ("allocations", "allocated_bytes", "peak_live_bytes")

SOFT_KEYS = (
    "rustc",
    "cargo",
    "target",
    "host",
    # The bench binary's resolved dependency graph, not the raw lockfile
    # bytes: a root version bump or a sibling member's dev-dependency change
    # must not read as drift when the compiled program is the same.
    "bench_graph_digest",
    # Codegen flags change the binary without changing the toolchain or the
    # host. `RUSTFLAGS='-C target-cpu=native'` on one run and not the other
    # produces two materially different programs on one machine, which would
    # otherwise show as no drift at all and be allowed to fail a gate.
    "rustflags",
)
# `machine` (CPU model, core count, runner image) is recorded in the
# environment but deliberately absent from SOFT_KEYS: it is rendered for the
# reader as advisory context, because on hosted runners machinery variance is
# the common case and flagging it as drift would mark essentially every
# comparison while changing nothing (decision recorded on issue #132).


def die(message: str) -> "None":
    """Report that no comparison was made. Never partial output."""
    print(f"no comparison was made: {message}", file=sys.stderr)
    sys.exit(2)


def run(*command: str) -> str:
    try:
        return subprocess.run(
            command, capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        die(f"could not run {' '.join(command)}: {error}")


def fnv1a64(data: bytes) -> str:
    """Matches `benches/manifest.rs`, so both halves speak one dialect."""
    digest = 0xCBF29CE484222325
    for byte in data:
        digest ^= byte
        digest = (digest * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{digest:016x}"


def root_package_name(repo_root: Path) -> str:
    manifest = repo_root / "Cargo.toml"
    try:
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        die(f"{manifest} could not be parsed: {error}")
    name = data.get("package", {}).get("name")
    if not name:
        die(f"{manifest} has no [package] name; cannot anchor the bench graph")
    return name


def bench_graph(repo_root: Path) -> list:
    """The identity of the program `cargo bench` builds, as sorted entries.

    Each entry is `name version source checksum` (literal `none` where the
    lockfile has no such field, as for path dependencies). The set is the
    reachable closure from the root package's own `[[package]]` stanza, whose
    dependency list merges normal, build, and dev dependencies — dev included
    deliberately, because benches compile against them. The root package
    itself is excluded, so its version bump cannot invalidate a baseline; a
    sibling workspace member is excluded unless the root actually depends on
    it, so its dev-dependency churn cannot either.
    """
    lockfile = repo_root / "Cargo.lock"
    if not lockfile.is_file():
        die(f"{lockfile} is missing; a benchmark identity needs the locked graph")
    try:
        data = tomllib.loads(lockfile.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        die(f"{lockfile} could not be parsed: {error}")
    packages = data.get("package")
    if not isinstance(packages, list) or not packages:
        die(f"{lockfile} contains no [[package]] entries")

    by_name = {}
    for package in packages:
        by_name.setdefault(package.get("name"), []).append(package)

    def resolve(spec: str, dependent: str) -> dict:
        # Lockfile dependency entries are `name`, `name version`, or
        # `name version (source)`; a bare name must be unambiguous.
        parts = spec.split()
        candidates = by_name.get(parts[0], [])
        if len(parts) >= 2:
            candidates = [c for c in candidates if c.get("version") == parts[1]]
        if len(parts) >= 3:
            source = " ".join(parts[2:]).strip("()")
            candidates = [c for c in candidates if c.get("source") == source]
        if len(candidates) != 1:
            die(
                f"Cargo.lock dependency {spec!r} of {dependent!r} resolves to "
                f"{len(candidates)} packages; the locked graph is not usable "
                "as an identity"
            )
        return candidates[0]

    root = root_package_name(repo_root)
    roots = by_name.get(root, [])
    if len(roots) != 1:
        die(f"Cargo.lock does not contain exactly one {root!r} package")
    seen = {}
    queue = [(spec, root) for spec in roots[0].get("dependencies", [])]
    while queue:
        spec, dependent = queue.pop()
        package = resolve(spec, dependent)
        key = (package["name"], package["version"])
        if key in seen or package["name"] == root:
            continue
        seen[key] = " ".join(
            (
                package["name"],
                package["version"],
                package.get("source") or "none",
                package.get("checksum") or "none",
            )
        )
        queue.extend(
            (dep, package["name"]) for dep in package.get("dependencies", [])
        )
    return sorted(seen.values())


def machine_identity() -> dict:
    """Advisory only, never a drift key: best effort, `unrecorded` on failure."""
    cpu = ""
    try:
        if sys.platform.startswith("linux"):
            with open("/proc/cpuinfo", encoding="utf-8") as cpuinfo:
                for line in cpuinfo:
                    if line.lower().startswith("model name"):
                        cpu = line.split(":", 1)[1].strip()
                        break
        elif sys.platform == "darwin":
            cpu = subprocess.run(
                ["sysctl", "-n", "machdep.cpu.brand_string"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        cpu = ""
    image = (
        f"{os.environ.get('ImageOS', '')} {os.environ.get('ImageVersion', '')}".strip()
    )
    return {
        "cpu_model": cpu or "unrecorded",
        "cpu_count": os.cpu_count() or 0,
        "runner_image": image or "unrecorded",
    }


def environment(graph: list) -> dict:
    verbose = run("rustc", "-Vv")
    host = ""
    for line in verbose.splitlines():
        if line.startswith("host: "):
            host = line[len("host: ") :].strip()
    # The *effective* target, not the host triple. `cargo bench --target ...`
    # or `CARGO_BUILD_TARGET` builds a different binary on the same machine,
    # and recording only `rustc -Vv`'s host would report no drift between them.
    target = os.environ.get("CARGO_BUILD_TARGET") or host
    # `CARGO_ENCODED_RUSTFLAGS` wins over `RUSTFLAGS` when both are set, and
    # separates arguments with \x1f; normalize so the two spellings of the same
    # flags compare equal.
    encoded = os.environ.get("CARGO_ENCODED_RUSTFLAGS")
    rustflags = (
        " ".join(encoded.split("\x1f"))
        if encoded is not None
        else os.environ.get("RUSTFLAGS", "")
    ).strip()
    return {
        "rustc": verbose.splitlines()[0].strip(),
        "cargo": run("cargo", "--version"),
        "target": target,
        "host": f"{platform.system()} {platform.machine()}",
        "bench_graph_digest": fnv1a64("\n".join(graph).encode("utf-8")),
        "rustflags": rustflags,
        "machine": machine_identity(),
    }


def load_json(path: Path, what: str) -> object:
    if not path.exists():
        die(f"{what} not found at {path}")
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        die(f"{what} at {path} could not be read: {error}")


def collect_criterion(root: Path, run_epoch: float) -> "tuple[dict, list, list]":
    """Read this run's benchmark estimates from a Criterion tree.

    Criterion writes `benchmark.json` and `estimates.json` together inside each
    case's `new/` directory. Sibling `base/`, `main/`, and `change/` directories
    hold whatever a previous run or a named baseline left behind — exactly the
    stale data this tool exists to avoid comparing against by accident — so the
    glob is anchored to `new/` rather than matching `benchmark.json` anywhere.

    That is necessary but not sufficient. Criterion never *removes* a `new/`
    directory, so a benchmark the candidate renamed or deleted keeps its result
    from whatever ran last in the same `target/criterion`. A recursive scan
    would import that leftover as if the candidate had just produced it, and the
    report would show a stale unchanged case rather than a removed one.

    `run_epoch` is the mtime of the manifest file, which the documented workflow
    emits *before* running the suite. Anything older than it was not produced by
    this run. Such cases are excluded and returned separately, so they are
    reported rather than silently dropped — a silent drop would be the same
    class of bug in the other direction. A fresh case whose `estimates.json`
    carries no median is excluded and returned the same way, for the same
    reason.
    """
    if not root.is_dir():
        die(f"Criterion output directory not found at {root}")
    results = {}
    stale = []
    unusable = []
    for benchmark_json in root.rglob("new/benchmark.json"):
        estimates_path = benchmark_json.parent / "estimates.json"
        if not estimates_path.is_file():
            continue
        if estimates_path.stat().st_mtime < run_epoch:
            stale.append(json.loads(benchmark_json.read_text())["full_id"])
            continue
        meta = json.loads(benchmark_json.read_text())
        estimates = json.loads(estimates_path.read_text())
        median = estimates.get("median")
        if not median:
            unusable.append(meta["full_id"])
            continue
        interval = median.get("confidence_interval", {})
        results[meta["full_id"]] = {
            "median_ns": median["point_estimate"],
            "lower_ns": interval.get("lower_bound"),
            "upper_ns": interval.get("upper_bound"),
            "throughput_bytes": (meta.get("throughput") or {}).get("Bytes"),
        }
    if not results:
        die(
            f"no Criterion estimates newer than the manifest under {root}. "
            "Emit the manifest first, then run the suite — see docs/benchmarks.md."
        )
    return results, sorted(stale), sorted(unusable)


def collect_allocations(path: Path) -> dict:
    rows = load_json(path, "allocation probe output")
    if not isinstance(rows, list):
        die("allocation probe output is not a JSON array")
    return {
        f"{row['case']}/{row['operation']}": {
            "allocations": row["allocations"],
            "allocated_bytes": row["allocated_bytes"],
            "peak_live_bytes": row["peak_live_bytes"],
        }
        for row in rows
    }


def build_record(args, repo_root: Path) -> "tuple[dict, list, list]":
    manifest_path = Path(args.manifest)
    manifest = load_json(manifest_path, "benchmark manifest")
    if not isinstance(manifest, dict) or "corpus_id" not in manifest:
        die("benchmark manifest is not the object `cargo bench --bench manifest` emits")
    criterion, stale, unusable = collect_criterion(
        Path(args.criterion), manifest_path.stat().st_mtime
    )
    graph = bench_graph(repo_root)
    record = {
        "baseline_schema_version": BASELINE_SCHEMA_VERSION,
        "manifest": manifest,
        "environment": environment(graph),
        "bench_graph": graph,
        "criterion": criterion,
        "allocations": collect_allocations(Path(args.allocations)),
    }
    return record, stale, unusable


def check_compatibility(baseline: dict, candidate: dict, allow_drift: bool) -> list:
    """Refuse outright on a hard mismatch; describe soft drift for the report."""
    if baseline.get("baseline_schema_version") != BASELINE_SCHEMA_VERSION:
        die(
            f"stored baseline is schema {baseline.get('baseline_schema_version')}, "
            f"this tool writes {BASELINE_SCHEMA_VERSION}; recapture it"
        )
    base_manifest = baseline.get("manifest", {})
    cand_manifest = candidate["manifest"]
    for key in HARD_KEYS:
        if base_manifest.get(key) != cand_manifest.get(key):
            die(
                f"benchmark manifests differ on {key!r}: baseline "
                f"{base_manifest.get(key)!r} vs candidate {cand_manifest.get(key)!r}. "
                "These runs did not measure the same thing; recapture the baseline."
            )

    drift = [
        (key, baseline.get("environment", {}).get(key), candidate["environment"][key])
        for key in SOFT_KEYS
        if baseline.get("environment", {}).get(key) != candidate["environment"][key]
    ]
    if drift and not allow_drift:
        details = "; ".join(f"{k}: {b!r} -> {c!r}" for k, b, c in drift)
        die(
            f"environment differs from the baseline ({details}). "
            "Recapture the baseline, or pass --allow-environment-drift to render "
            "an informational report that cannot fail a gate."
        )
    return drift


def delta_rows(base: dict, cand: dict, key: str) -> list:
    rows = []
    for name in sorted(set(base) | set(cand)):
        before, after = base.get(name), cand.get(name)
        if before is None or after is None:
            rows.append((name, before, after, None))
            continue
        b, a = before[key], after[key]
        change = ((a - b) / b * 100.0) if b else None
        rows.append((name, before, after, change))
    return rows


def is_control(name: str) -> bool:
    """`official_*` cases run only upstream `candid_parser` code.

    No change in this repository can alter what they execute, so a shift they
    share with the rest of the table measures the machinery, not the
    candidate. The report leads with that shift instead of leaving the reader
    to infer it (issue #132).
    """
    return any(segment.startswith("official_") for segment in name.split("/"))


def control_summary(timing_rows: list) -> "dict | None":
    changes = [
        change
        for name, _before, _after, change in timing_rows
        if change is not None and is_control(name)
    ]
    if not changes:
        return None
    return {
        "count": len(changes),
        "median_pct": statistics.median(changes),
        "min_pct": min(changes),
        "max_pct": max(changes),
    }


def bench_graph_drift_lines(base_graph: "list | None", cand_graph: list) -> list:
    """Name what actually changed when the graph digests differ.

    Two opaque digests steer a reader nowhere (issue #132); the stored graphs
    make the drift attributable: version moves render as `name: old → new`,
    everything else as added/removed entries.
    """
    base = set(base_graph or [])
    cand = set(cand_graph or [])
    if not base or base == cand:
        return []

    def by_name(entries: set) -> dict:
        index = {}
        for entry in entries:
            name, version = entry.split()[:2]
            index.setdefault(name, []).append(version)
        return index

    removed, added = by_name(base - cand), by_name(cand - base)
    lines = []
    for name in sorted(set(removed) | set(added)):
        old, new = removed.get(name, []), added.get(name, [])
        if len(old) == 1 and len(new) == 1:
            if old[0] != new[0]:
                lines.append(f"`{name}`: {old[0]} → {new[0]}")
            else:
                lines.append(f"`{name}` {old[0]}: source or checksum changed")
            continue
        lines.extend(f"removed `{name}` {version}" for version in sorted(old))
        lines.extend(f"added `{name}` {version}" for version in sorted(new))
    return lines


def format_ns(value: "float | None") -> str:
    """Scale to the unit that keeps the number readable; raw ns stay in JSON."""
    if value is None:
        return "—"
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if abs(value) >= scale:
            return f"{value / scale:,.2f} {unit}"
    return f"{value:,.0f} ns"


def format_interval(entry: dict) -> str:
    if entry.get("lower_ns") is None or entry.get("upper_ns") is None:
        return ""
    return f"{format_ns(entry['lower_ns'])} – {format_ns(entry['upper_ns'])}"


def describe_machine(machine: "dict | None") -> str:
    if not machine:
        return "unrecorded"
    return (
        f"{machine.get('cpu_model', 'unrecorded')}, "
        f"{machine.get('cpu_count', 0)} cores, "
        f"image {machine.get('runner_image', 'unrecorded')}"
    )


def render_markdown(report: dict) -> str:
    out = ["# Benchmark comparison", ""]
    if report["informational_only"]:
        out += [
            "> **Informational only.** The environment differs from the baseline, so "
            "these deltas describe two machines as much as two revisions. They cannot "
            "fail a gate.",
            "",
        ]
        for key, before, after in report["environment_drift"]:
            out.append(f"> - `{key}`: `{before}` → `{after}`")
            if key == "bench_graph_digest":
                lines = report.get("bench_graph_drift") or []
                if lines:
                    out.extend(f">   - {line}" for line in lines)
                else:
                    out.append(
                        ">   - the baseline does not record its bench graph, so "
                        "the change cannot be attributed; recapture it"
                    )
        out.append("")
    # The controls are the reader's yardstick: they execute no code this
    # repository can change, so their shared shift is the machinery's
    # contribution and belongs above the table it explains.
    controls = report.get("control_shift")
    if controls:
        out += [
            f"> **Controls: {controls['median_pct']:+.1f}% median** across "
            f"{controls['count']} upstream-only `official_*` benchmarks (range "
            f"{controls['min_pct']:+.1f}% … {controls['max_pct']:+.1f}%). These "
            "cases execute no code from this repository, so a broad shift they "
            "share with the rest of the table measures the machinery, not the "
            "change under test — read the code deltas against it.",
            "",
        ]
    baseline_machine = describe_machine(report["machine"].get("baseline"))
    candidate_machine = describe_machine(report["machine"].get("candidate"))
    if baseline_machine == candidate_machine:
        machinery = f"Machinery (advisory): `{candidate_machine}` on both sides"
    else:
        machinery = (
            "Machinery (advisory, never a drift key): baseline "
            f"`{baseline_machine}` → candidate `{candidate_machine}` — timing "
            "deltas partly describe the machines"
        )
    out += [
        f"Corpus `{report['corpus_id']}` · features "
        f"`{', '.join(report['features'])}` · {report['rustc']}",
        "",
        machinery,
        "",
        "## Timing (Criterion median)",
        "",
        "| Benchmark | Baseline | Candidate | Delta "
        "| 95% CI (baseline) | 95% CI (candidate) |",
        "| --- | ---: | ---: | ---: | --- | --- |",
    ]
    for name, before, after, change in report["timing"]:
        label = f"`{name}` (control)" if is_control(name) else f"`{name}`"
        if before is None:
            out.append(
                f"| {label} | — | {format_ns(after['median_ns'])} | new "
                f"| | {format_interval(after)} |"
            )
            continue
        if after is None:
            out.append(
                f"| {label} | {format_ns(before['median_ns'])} | — | removed "
                f"| {format_interval(before)} | |"
            )
            continue
        delta = f"{change:+.1f}%" if change is not None else "n/a"
        out.append(
            f"| {label} | {format_ns(before['median_ns'])} "
            f"| {format_ns(after['median_ns'])} | {delta} "
            f"| {format_interval(before)} | {format_interval(after)} |"
        )
    # All three captured metrics, not only the count. A change that keeps the
    # number of allocations constant while growing cumulative or peak bytes is a
    # memory regression, and reporting only `allocations` would render it as a
    # reassuring 0%. Rows are grouped per case so one case's three metrics read
    # together.
    out += [
        "",
        "## Allocations",
        "",
        "| Case | Metric | Baseline | Candidate | Delta |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    by_metric = {
        metric: {row[0]: row for row in report["allocations"][metric]}
        for metric in ALLOCATION_METRICS
    }
    names = sorted(set().union(*(set(rows) for rows in by_metric.values())))
    for name in names:
        for metric in ALLOCATION_METRICS:
            row = by_metric[metric].get(name)
            if row is None:
                continue
            _name, before, after, change = row
            if before is None or after is None:
                out.append(
                    f"| `{name}` | {metric} | "
                    f"{f'{before[metric]:,}' if before else '—'} | "
                    f"{f'{after[metric]:,}' if after else '—'} | |"
                )
                continue
            delta = f"{change:+.1f}%" if change is not None else "n/a"
            out.append(
                f"| `{name}` | {metric} | {before[metric]:,} | {after[metric]:,} "
                f"| {delta} |"
            )
    excluded = [
        (case, "estimates predate this run's manifest — a renamed or removed case")
        for case in report.get("stale_cases", [])
    ] + [
        (case, "estimates carry no median — the run did not produce a usable result")
        for case in report.get("unusable_cases", [])
    ]
    if excluded:
        out += ["", "## Excluded from this comparison", ""]
        out.extend(f"- `{case}` — {reason}" for case, reason in excluded)
    out += [
        "",
        "Deltas are regression *signals*, not proof. A single run on a shared machine "
        "is diagnostic; repeat a suspected change and read absolute values and "
        "intervals, not only percentages.",
        "",
    ]
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    for name in ("capture", "compare"):
        p = sub.add_parser(name)
        p.add_argument("--manifest", required=True)
        p.add_argument("--criterion", default="target/criterion")
        p.add_argument("--allocations", required=True)
        if name == "capture":
            p.add_argument("--out", required=True)
            p.add_argument("--note", default="", help="why this baseline was captured")
        else:
            p.add_argument("--baseline", required=True)
            p.add_argument("--markdown")
            p.add_argument("--json", dest="json_out")
            p.add_argument("--allow-environment-drift", action="store_true")
            p.add_argument(
                "--fail-on-regression",
                type=float,
                metavar="PCT",
                help="exit 1 if any median regresses by more than PCT percent",
            )

    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[3]
    candidate, stale, unusable = build_record(args, repo_root)

    # Reported, never silently dropped. A leftover `new/` from an earlier run in
    # the same `target/criterion` means the candidate no longer produces that
    # benchmark — a renamed or deleted case — which is information, not noise.
    if stale:
        print(
            f"note: ignored {len(stale)} Criterion case(s) older than this run's "
            f"manifest: {', '.join(stale)}",
            file=sys.stderr,
        )
    if unusable:
        print(
            f"note: ignored {len(unusable)} Criterion case(s) with no median in "
            f"estimates.json: {', '.join(unusable)}",
            file=sys.stderr,
        )

    if args.command == "capture":
        candidate["note"] = args.note
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(candidate, indent=2, sort_keys=True) + "\n")
        print(f"captured baseline: {out}")
        print(f"  corpus {candidate['manifest']['corpus_id']}")
        print(f"  {len(candidate['criterion'])} timing cases, "
              f"{len(candidate['allocations'])} allocation cases")
        return 0

    baseline = load_json(Path(args.baseline), "stored baseline")
    drift = check_compatibility(baseline, candidate, args.allow_environment_drift)

    # Refuse the gate *before* rendering anything: printing a full comparison
    # and then exiting "no comparison was made" would contradict itself.
    if args.fail_on_regression is not None and drift:
        die(
            "--fail-on-regression cannot be used with an environment-drifted "
            "comparison; the deltas are not evidence"
        )

    timing = delta_rows(baseline["criterion"], candidate["criterion"], "median_ns")
    report = {
        "corpus_id": candidate["manifest"]["corpus_id"],
        "features": candidate["manifest"]["features"],
        "rustc": candidate["environment"]["rustc"],
        "informational_only": bool(drift),
        "environment_drift": drift,
        "bench_graph_drift": bench_graph_drift_lines(
            baseline.get("bench_graph"), candidate["bench_graph"]
        ),
        "machine": {
            "baseline": baseline.get("environment", {}).get("machine"),
            "candidate": candidate["environment"]["machine"],
        },
        "control_shift": control_summary(timing),
        "timing": timing,
        "allocations": {
            metric: delta_rows(
                baseline["allocations"], candidate["allocations"], metric
            )
            for metric in ALLOCATION_METRICS
        },
        "stale_cases": stale,
        "unusable_cases": unusable,
    }

    markdown = render_markdown(report)
    print(markdown)
    if args.markdown:
        Path(args.markdown).write_text(markdown)
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(report, indent=2, default=str) + "\n")

    if args.fail_on_regression is not None:
        worst = [
            (name, change)
            for name, _b, _a, change in report["timing"]
            if change is not None and change > args.fail_on_regression
        ]
        if worst:
            print("", file=sys.stderr)
            for name, change in worst:
                print(
                    f"regression: {name} {change:+.1f}% "
                    f"(threshold {args.fail_on_regression:+.1f}%)",
                    file=sys.stderr,
                )
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
