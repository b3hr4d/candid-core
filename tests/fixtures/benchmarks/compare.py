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
units. This script records what only it can observe reliably — toolchain, target,
host, and the lockfile digest — and stores both together.

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
import subprocess
import sys
from pathlib import Path

# Bumped when the *stored baseline* layout changes. Independent of the Rust
# manifest's own schema version, which describes what was measured.
BASELINE_SCHEMA_VERSION = 1

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
    "lockfile_digest",
    # Codegen flags change the binary without changing the toolchain or the
    # host. `RUSTFLAGS='-C target-cpu=native'` on one run and not the other
    # produces two materially different programs on one machine, which would
    # otherwise show as no drift at all and be allowed to fail a gate.
    "rustflags",
)


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


def environment(repo_root: Path) -> dict:
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
    lockfile = repo_root / "Cargo.lock"
    if not lockfile.is_file():
        die(f"{lockfile} is missing; a benchmark identity needs the locked graph")
    return {
        "rustc": verbose.splitlines()[0].strip(),
        "cargo": run("cargo", "--version"),
        "target": target,
        "host": f"{platform.system()} {platform.machine()}",
        "lockfile_digest": fnv1a64(lockfile.read_bytes()),
        "rustflags": rustflags,
    }


def load_json(path: Path, what: str) -> object:
    if not path.exists():
        die(f"{what} not found at {path}")
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        die(f"{what} at {path} could not be read: {error}")


def collect_criterion(root: Path, run_epoch: float) -> "tuple[dict, list]":
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
    class of bug in the other direction.
    """
    if not root.is_dir():
        die(f"Criterion output directory not found at {root}")
    results = {}
    stale = []
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
    return results, sorted(stale)


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


def build_record(args, repo_root: Path) -> "tuple[dict, list]":
    manifest_path = Path(args.manifest)
    manifest = load_json(manifest_path, "benchmark manifest")
    if not isinstance(manifest, dict) or "corpus_id" not in manifest:
        die("benchmark manifest is not the object `cargo bench --bench manifest` emits")
    criterion, stale = collect_criterion(
        Path(args.criterion), manifest_path.stat().st_mtime
    )
    record = {
        "baseline_schema_version": BASELINE_SCHEMA_VERSION,
        "manifest": manifest,
        "environment": environment(repo_root),
        "criterion": criterion,
        "allocations": collect_allocations(Path(args.allocations)),
    }
    return record, stale


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
        out.append("")
    out += [
        f"Corpus `{report['corpus_id']}` · features "
        f"`{', '.join(report['features'])}` · {report['rustc']}",
        "",
        "## Timing (Criterion median)",
        "",
        "| Benchmark | Baseline | Candidate | Delta | 95% CI (candidate) |",
        "| --- | ---: | ---: | ---: | --- |",
    ]
    for name, before, after, change in report["timing"]:
        if before is None:
            out.append(f"| `{name}` | — | {after['median_ns']:.0f} ns | new | |")
            continue
        if after is None:
            out.append(f"| `{name}` | {before['median_ns']:.0f} ns | — | removed | |")
            continue
        ci = (
            f"{after['lower_ns']:.0f} – {after['upper_ns']:.0f} ns"
            if after.get("lower_ns") is not None
            else ""
        )
        out.append(
            f"| `{name}` | {before['median_ns']:.0f} ns | {after['median_ns']:.0f} ns "
            f"| {change:+.1f}% | {ci} |"
        )
    # All three captured metrics, not only the count. A change that keeps the
    # number of allocations constant while growing cumulative or peak bytes is a
    # memory regression, and reporting only `allocations` would render it as a
    # reassuring 0%.
    out += [
        "",
        "## Allocations",
        "",
        "| Case | Metric | Baseline | Candidate | Delta |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for metric in ALLOCATION_METRICS:
        for name, before, after, change in report["allocations"][metric]:
            if before is None or after is None:
                out.append(
                    f"| `{name}` | {metric} | "
                    f"{before[metric] if before else '—'} | "
                    f"{after[metric] if after else '—'} | |"
                )
                continue
            out.append(
                f"| `{name}` | {metric} | {before[metric]} | {after[metric]} "
                f"| {change:+.1f}% |"
            )
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
    candidate, stale = build_record(args, repo_root)

    # Reported, never silently dropped. A leftover `new/` from an earlier run in
    # the same `target/criterion` means the candidate no longer produces that
    # benchmark — a renamed or deleted case — which is information, not noise.
    if stale:
        print(
            f"note: ignored {len(stale)} Criterion case(s) older than this run's "
            f"manifest: {', '.join(stale)}",
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

    report = {
        "corpus_id": candidate["manifest"]["corpus_id"],
        "features": candidate["manifest"]["features"],
        "rustc": candidate["environment"]["rustc"],
        "informational_only": bool(drift),
        "environment_drift": drift,
        "timing": delta_rows(baseline["criterion"], candidate["criterion"], "median_ns"),
        "allocations": {
            metric: delta_rows(
                baseline["allocations"], candidate["allocations"], metric
            )
            for metric in ALLOCATION_METRICS
        },
        "stale_cases": stale,
    }

    markdown = render_markdown(report)
    print(markdown)
    if args.markdown:
        Path(args.markdown).write_text(markdown)
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(report, indent=2, default=str) + "\n")

    if args.fail_on_regression is not None:
        if drift:
            die(
                "--fail-on-regression cannot be used with an environment-drifted "
                "comparison; the deltas are not evidence"
            )
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
