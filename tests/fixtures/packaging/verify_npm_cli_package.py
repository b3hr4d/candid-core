#!/usr/bin/env python3
"""What `@candid-core/cli` actually ships, verified against the tarball.

`npm pack` silently drops a `files` entry naming a path that does not exist,
and this package's `wasm/` outputs are build products excluded from git. So a
publish whose build step was skipped, reordered, or failed open produces a
tarball with **no WebAssembly in it** — 6 entries instead of 8, exit 0, no
warning — that 404s on its own core at first import. npm versions are
permanent, so that cannot be corrected in place.

The sibling gate (`verify_npm_package.py`) proves the same class of claim for
`@candid-core/schema`. This is a separate script rather than a parameter on
that one: the two packages ship different things (a bin and a wasm artifact
here, eight subpath exports there) and share no assertion beyond "pack it and
look", so one script serving both would be a switch statement wearing a
function's clothes.

Run from anywhere; paths are resolved from this file.
"""

import datetime
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[3]
PACKAGE = REPO / "crates" / "candid-core-wasm" / "npm"
SCHEMA = REPO / "crates" / "candid-core-ts" / "ts"

# Big enough that a truncated or placeholder artifact fails, far enough below
# the real size (~1.6 MB unoptimised) that an optimisation pass does not.
MIN_WASM_BYTES = 200_000

SERVICE = 'type Account = record { owner : principal; tokens : nat };\nservice : { balance : (Account) -> (nat) query; }\n'


def run(args, cwd, **kw):
    return subprocess.run(args, cwd=cwd, check=True, text=True, **kw)


def pack(directory, into):
    out = run(
        ["npm", "pack", "--silent", "--pack-destination", str(into)],
        cwd=directory,
        stdout=subprocess.PIPE,
    )
    return into / out.stdout.strip().splitlines()[-1]


def main():
    manifest = json.loads((PACKAGE / "package.json").read_text())

    if not (PACKAGE / "wasm" / "candid_core_wasm_bg.wasm").exists():
        raise SystemExit(
            "the wasm artifact is missing: run `npm run build` in "
            f"{PACKAGE} before this gate. Packing without it produces a "
            "tarball with no WebAssembly and no error, which is the whole "
            "failure this script exists to refuse."
        )

    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        tarball = pack(PACKAGE, tmp)
        extracted = tmp / "extract"
        extracted.mkdir()
        run(["tar", "xzf", str(tarball), "-C", str(extracted)], cwd=tmp)
        root = extracted / "package"

        # 1. The shipped file list is exactly what the manifest promises.
        shipped = sorted(
            str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()
        )
        expected = set()
        for entry in manifest["files"]:
            target = PACKAGE / entry
            if target.is_dir():
                expected.update(
                    str(p.relative_to(PACKAGE)) for p in target.rglob("*") if p.is_file()
                )
            elif target.is_file():
                expected.add(entry)
            else:
                raise SystemExit(
                    f"package.json `files` names {entry!r}, which does not "
                    "exist. npm would drop it silently."
                )
        expected.add("package.json")
        if set(shipped) != expected:
            missing = sorted(expected - set(shipped))
            extra = sorted(set(shipped) - expected)
            raise SystemExit(
                f"the packed file list does not match the manifest.\n"
                f"  missing: {missing}\n  unexpected: {extra}"
            )

        # 2. The WebAssembly is present and is a real artifact.
        wasm = root / "wasm" / "candid_core_wasm_bg.wasm"
        size = wasm.stat().st_size
        if size < MIN_WASM_BYTES:
            raise SystemExit(
                f"the packed wasm is {size} bytes, under the {MIN_WASM_BYTES} "
                "floor: a truncated or placeholder artifact."
            )
        if wasm.read_bytes()[:4] != b"\0asm":
            raise SystemExit("the packed wasm does not carry the WebAssembly magic")

        # 3. The entry being released must not describe itself as unpublished.
        #    A tarball is immutable: "prepared, not yet published" would sit on
        #    npm forever telling whoever installed it that it does not exist.
        changelog = (root / "CHANGELOG.md").read_text()
        entries = {
            name: (status, entry_body)
            for name, status, entry_body in re.findall(
                r"^## (\S+)([^\n]*)\n(.*?)(?=^## |\Z)", changelog, re.M | re.S
            )
        }
        version = manifest["version"]
        if version not in entries:
            raise SystemExit(
                f"the shipped changelog documents no '## {version}' entry, so "
                f"the packed version arrives on npm undocumented. Headings "
                f"found: {sorted(entries)}"
            )
        status, entry_body = entries[version]
        heading_date = re.fullmatch(r"\s*—\s*(\d{4}-\d{2}-\d{2})\s*", status)
        if not heading_date:
            raise SystemExit(
                f"the changelog heading for the released version must read "
                f"'## {version} — YYYY-MM-DD'; it reads '## {version}{status}'. "
                "A shipped tarball cannot describe itself as unpublished."
            )
        try:
            datetime.date.fromisoformat(heading_date.group(1))
        except ValueError:
            raise SystemExit(
                f"the changelog heading for {version} carries "
                f"'{heading_date.group(1)}', which is not a real date"
            ) from None

        # This package pairs by *revision*, not version: it embeds a candid-core
        # build from one commit. So the wording is "Embeds", where the sibling
        # gate looks for "Pairs with".
        if not re.search(r"Embeds `candid-core` \S+", entry_body):
            raise SystemExit(
                f"the changelog entry for {version} names no embedded "
                "`candid-core` revision, which is the pairing this package "
                "promises in place of a version pairing"
            )

        # 4. Nothing in the tarball's prose cites an internal issue number.
        #    A consumer cannot follow one, and unlike the sibling package
        #    nothing here strips comments on the way out.
        for path in sorted(root.rglob("*")):
            if path.is_file() and path.suffix in {".js", ".ts", ".md", ".mts"}:
                hit = re.search(r"#\d", path.read_text(errors="ignore"))
                if hit:
                    raise SystemExit(
                        f"{path.relative_to(root)} ships an internal issue "
                        f"number ({hit.group(0)}…), which a consumer cannot follow"
                    )

        # 5. A real consumer: install the tarball beside the schema package it
        #    declares, run the CLI end to end, and compile against it with no
        #    DOM lib — the shape most Node consumers of a CLI actually use.
        # The peer's `dist/` is a build product excluded from git, so packing
        # it unbuilt yields a tarball whose subpaths do not resolve. `npm ci`
        # alone does not produce it — this is what CI has and a warm working
        # tree does not, which is exactly the difference a gate must not depend
        # on.
        run(["npm", "run", "build"], SCHEMA, stdout=subprocess.DEVNULL)
        schema_tarball = pack(SCHEMA, tmp)
        consumer = tmp / "consumer"
        consumer.mkdir()
        (consumer / "package.json").write_text(
            json.dumps({"name": "cli-consumer", "private": True, "type": "module"})
        )
        (consumer / "service.did").write_text(SERVICE)
        run(
            ["npm", "install", "--silent", "--no-audit", "--no-fund",
             str(tarball), str(schema_tarball)],
            cwd=consumer,
        )

        run(
            ["node", "node_modules/@candid-core/cli/bin/cli.js",
             "gen", "./service.did", "-o", "./out"],
            cwd=consumer,
            stdout=subprocess.DEVNULL,
        )
        produced = sorted(p.name for p in (consumer / "out").iterdir())
        if not any(n.endswith(".ts") for n in produced):
            raise SystemExit(f"the CLI emitted no module: {produced}")
        if not any(n.endswith(".envelope.json") for n in produced):
            raise SystemExit(f"the CLI emitted no envelope: {produced}")

        envelope = json.loads(
            next((consumer / "out").glob("*.envelope.json")).read_text()
        )
        if "contract" not in envelope or "extensions" not in envelope:
            raise SystemExit(
                f"the emitted envelope is not one: keys {sorted(envelope)}"
            )

        # 6. The declared peer must be able to load what the CLI emitted, and
        #    the package's own declarations must compile without the DOM.
        # No host globals here on purpose (no `console`, no `process`): with
        # `types: []` and no DOM lib those are undeclared, and using one would
        # make this probe fail for a reason that has nothing to do with the
        # package's own declarations.
        (consumer / "check.ts").write_text(
            'import { didToContract, didToModule } from "@candid-core/cli";\n'
            'import { schemaFromContract } from "@candid-core/schema/contract";\n'
            "\n"
            "export async function main(): Promise<string[]> {\n"
            "  const notes: string[] = [];\n"
            '  const result = await didToContract("service : { ping : () -> (); }");\n'
            '  if ("contract" in result) {\n'
            "    const built = schemaFromContract(result);\n"
            '    notes.push(built.ok ? "built" : "refused");\n'
            "  } else {\n"
            "    for (const issue of result.diagnostics) {\n"
            '      notes.push(`${issue.code}: ${issue.message} ${issue.phase ?? ""}`);\n'
            "    }\n"
            "  }\n"
            '  const emitted = await didToModule({ source: "service : { ping : () -> (); }" });\n'
            '  notes.push(emitted.ok ? String(emitted.module.length) : "failed");\n'
            "  return notes;\n"
            "}\n"
        )
        (consumer / "tsconfig.json").write_text(
            json.dumps(
                {
                    "compilerOptions": {
                        "strict": True,
                        "target": "es2022",
                        # No DOM, and no ambient @types: a .d.ts naming
                        # BufferSource, URL or Response fails right here.
                        "lib": ["es2022"],
                        "types": [],
                        "module": "nodenext",
                        "moduleResolution": "nodenext",
                        "noEmit": True,
                        "skipLibCheck": False,
                    },
                    "files": ["check.ts"],
                }
            )
        )
        tsc = SCHEMA / "node_modules" / ".bin" / "tsc"
        if not tsc.exists():
            raise SystemExit(
                f"the pinned TypeScript is missing: run `npm ci` in {SCHEMA}"
            )
        run([str(tsc), "-p", "tsconfig.json"], cwd=consumer)

        # 7. The module the CLI actually emitted compiles against the peer too.
        emitted = next(p for p in (consumer / "out").iterdir() if p.suffix == ".ts")
        shutil.copy(emitted, consumer / "emitted.ts")
        (consumer / "tsconfig.emitted.json").write_text(
            json.dumps(
                {
                    "extends": "./tsconfig.json",
                    "files": ["emitted.ts"],
                }
            )
        )
        run([str(tsc), "-p", "tsconfig.emitted.json"], cwd=consumer)

    print(
        "npm cli package verified: manifest file list, wasm present "
        f"({size} bytes), self-contained prose, end-to-end gen, envelope "
        "shape, DOM-less strict compile, emitted module compiles against "
        f"the declared peer {manifest['peerDependencies']['@candid-core/schema']}"
    )


if __name__ == "__main__":
    sys.exit(main())
