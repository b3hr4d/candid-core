#!/usr/bin/env python3
"""The packaged-consumer smoke for @candid-core/schema (issue #106).

Builds the package, packs the exact artifact `npm publish` would ship,
extracts it into a scratch consumer, and proves the artifact stands up on
its own: the shipped file list is exactly what the manifest promises, the
root and every subpath export resolve, everything compiles under the pinned
strict TypeScript *without* `skipLibCheck` — so a broken declaration is a
failure here rather than a permanent mistake on npm — and a real
encode/validate round-trip executes under node. Consumption goes through the
extracted tarball only, never the repository sources.

The shipped declarations are also read as documentation, because that is what
they are: JSDoc flows into `dist/*.d.ts` at build time and becomes the editor
hover a consumer meets first. An internal issue number there means nothing to
that reader, and a published version bakes it in permanently, so the gate
below refuses one in the artifact rather than after the fact.

The shipped *prose* is held to the same standard, because it is the first
thing a consumer meets on npm: the README's TypeScript blocks are compiled
against the packed artifact, exactly as a consumer's would be, so a documented
example cannot drift away from the package it documents. A block that cannot
be compiled here is opted out by an HTML comment naming the reason, so the
exemption is visible in the README rather than implicit in this script (no
block needs one today).

The changelog is treated as an artifact claim rather than a courtesy. It ships
in the tarball, and the gate refuses one that does not document the version
being packed together with the `candid-core` version it pairs with — the
README promises exactly that, and a promise about an artifact belongs in the
artifact's gate.

The export surface is asserted exactly, in both directions. The manifest
must export precisely the four Candid-layer subpaths (`.`, `./validate`,
`./contract`, `./codec`) and declare no runtime dependency and no peer of
any kind; no shipped module may import from `@icp-sdk/core`; and a consumer of
every remaining subpath compiles and runs in a scratch tree with no
`@icp-sdk/core` installed at all. The subpaths 0.2.0 exported and this
package no longer does — `./actor`, `./transport-icp`, `./forms`,
`./labels` — must each fail to resolve, at runtime with Node's
`ERR_PACKAGE_PATH_NOT_EXPORTED` and at compile time with a missing-module
error, and so must a deep import of the internal modules the entry points
still ship and load (`dist/labels.js`, `dist/options.js`,
`dist/principal-text.js`, `dist/typetable.js`).

Principals are canonical text, branded `Principal`: the consumer below converts
an SDK-style object once through `principal()`, proves the unconverted object
is refused by `validate` and `encode` alike, and proves a decoded principal is
a plain string that survives `JSON.stringify` and `structuredClone` and
compares with `===` — all against the packed artifact.
"""

import datetime
import json
import pathlib
import re
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[3]
PACKAGE = REPO / "crates" / "candid-core-ts" / "ts"

# The export map, exactly: the Candid layer and nothing else.
EXPORTED = [".", "./validate", "./contract", "./codec"]

# Subpaths 0.2.0 exported that this package no longer does. Each must fail
# to resolve from the packed artifact; a deep path into `dist/` must fail the
# same way, because `exports` is what makes the internal modules internal.
REMOVED = ["./actor", "./transport-icp", "./forms", "./labels"]
DEEP = [
    "./dist/labels.js",
    "./dist/options.js",
    "./dist/principal-text.js",
    "./dist/typetable.js",
    "./dist/forms.js",
]

# Manifest fields that would give the package a dependency of any kind.
DEPENDENCY_FIELDS = [
    "dependencies",
    "peerDependencies",
    "peerDependenciesMeta",
    "optionalDependencies",
    "bundleDependencies",
    "bundledDependencies",
]

# A README block preceded by this marker is documented as verified elsewhere
# and is not compiled here. The marker is a full HTML comment in the README,
# so the reason travels with the exemption instead of living in this script.
UNCOMPILED_MARKER = "Not compiled by the packaged-consumer gate"


def run(args, cwd):
    subprocess.run(args, cwd=cwd, check=True)


def typescript_blocks(readme):
    """Fenced ```ts blocks, minus those the preceding comment opts out."""
    blocks = []
    lines = readme.splitlines()
    index = 0
    exempt = False
    while index < len(lines):
        line = lines[index]
        if UNCOMPILED_MARKER in line:
            exempt = True
        elif line.strip() == "```ts":
            end = index + 1
            while end < len(lines) and lines[end].strip() != "```":
                end += 1
            if end == len(lines):
                raise SystemExit(f"unterminated ```ts block at README line {index + 1}")
            if not exempt:
                blocks.append("\n".join(lines[index + 1 : end]) + "\n")
            exempt = False
            index = end
        index += 1
    return blocks


def main():
    run(["npm", "run", "build"], PACKAGE)
    with tempfile.TemporaryDirectory() as scratch_name:
        scratch = pathlib.Path(scratch_name)
        pack = subprocess.run(
            ["npm", "pack", "--json", "--pack-destination", str(scratch)],
            cwd=PACKAGE,
            check=True,
            capture_output=True,
            text=True,
        )
        tarball = scratch / json.loads(pack.stdout)[0]["filename"]
        extracted = scratch / "node_modules" / "@candid-core" / "schema"
        extracted.parent.mkdir(parents=True)
        run(["tar", "-xzf", str(tarball), "-C", str(scratch)], scratch)
        (scratch / "package").rename(extracted)

        # The shipped file list is exactly the manifest's promise — a stray
        # test file or a missing declaration fails here.
        shipped = sorted(
            str(path.relative_to(extracted))
            for path in extracted.rglob("*")
            if path.is_file()
        )
        # `labels`, `options`, `principal-text` and `typetable` ship without
        # an export: the codec, the Contract loader, `validate` and (for
        # `principal-text`) the root entry's `principal()` and `isPrincipal()`
        # import them relatively, so they are part of those modules' runtime,
        # not entry points. `forms` is internal and is not built at all.
        modules = [
            "codec",
            "contract",
            "labels",
            "options",
            "principal-text",
            "schema",
            "typetable",
            "validate",
        ]
        expected = sorted(
            ["CHANGELOG.md", "LICENSE", "README.md", "package.json"]
            + [f"dist/{name}.js" for name in modules]
            + [f"dist/{name}.d.ts" for name in modules]
        )
        if shipped != expected:
            raise SystemExit(
                f"the packed file list is not the manifest's promise:\n"
                f"  shipped:  {shipped}\n  expected: {expected}"
            )

        manifest = json.loads((extracted / "package.json").read_text())

        # The export surface and the dependency surface, stated exactly. A
        # removed subpath left in the map, or a peer re-added, fails here
        # before anything is compiled.
        exported = list(manifest.get("exports", {}))
        if exported != EXPORTED:
            raise SystemExit(
                f"package.json exports {exported}; the package exports exactly "
                f"{EXPORTED}"
            )
        declared = [field for field in DEPENDENCY_FIELDS if field in manifest]
        if declared:
            raise SystemExit(
                f"package.json declares {declared}; the package has no runtime "
                "dependency and no peer of any kind"
            )
        # Documentation may name the SDK (the principal docs explain how its
        # values interoperate); no shipped module or declaration may import
        # from it, statically, dynamically, or by `require`.
        sdk_import = re.compile(
            r"""(?:\bfrom|\bimport|\brequire)\s*\(?\s*["']@icp-sdk/"""
        )
        sdk = [
            f"{path.relative_to(extracted)}:{number}: {line.strip()}"
            for path in sorted(extracted.glob("dist/*"))
            for number, line in enumerate(path.read_text().splitlines(), 1)
            if sdk_import.search(line)
        ]
        if sdk:
            raise SystemExit(
                "shipped modules import @icp-sdk/core, which this package "
                "neither depends on nor peers:\n  " + "\n  ".join(sdk)
            )

        # npm derives the homepage link from `repository` when the field is
        # absent, which lands a consumer on the repository root README. That
        # is a different package's material, so the field is required here.
        if not manifest.get("homepage"):
            raise SystemExit(
                "package.json has no homepage; npm would derive one from "
                "`repository` and send consumers to the repository root README"
            )

        # The README tells a reader that the changelog records, for every
        # release, the candid-core version it pairs with. That is a claim
        # about this artifact, so the artifact's gate is where it is checked.
        #
        # Headings are parsed rather than substring-matched: `"## 0.1.2" in
        # text` is also true of `## 0.1.20`, so a heading mistyped to a
        # prefix-sharing version would pass this gate and then hand the *other*
        # release's entry to the pairing check below — the gate failing exactly
        # where it is needed.
        version = manifest["version"]
        changelog = (extracted / "CHANGELOG.md").read_text()
        entries = {
            name: (status, body)
            for name, status, body in re.findall(
                r"^## (\S+)([^\n]*)\n(.*?)(?=^## |\Z)", changelog, re.M | re.S
            )
        }
        if version not in entries:
            raise SystemExit(
                f"the shipped changelog documents no '## {version}' entry, so "
                f"the packed version {version} arrives on npm undocumented. "
                f"Headings found: {sorted(entries)}"
            )
        status, body = entries[version]

        # The entry being released must not describe itself as unpublished.
        # A tarball is immutable, so "prepared, not yet published" would sit on
        # npm forever telling whoever installed it that it does not exist. The
        # crate's archive has to say that — it is digested before publication —
        # but `npm-release.yml` takes no digest input, so this one can and must
        # carry the real date.
        heading_date = re.fullmatch(r"\s*—\s*(\d{4}-\d{2}-\d{2})\s*", status)
        if not heading_date:
            raise SystemExit(
                f"the changelog heading for the released version must read "
                f"'## {version} — YYYY-MM-DD'; it reads "
                f"'## {version}{status}'. A shipped tarball cannot describe "
                "itself as unpublished."
            )
        try:
            datetime.date.fromisoformat(heading_date.group(1))
        except ValueError:
            raise SystemExit(
                f"the changelog heading for {version} carries "
                f"'{heading_date.group(1)}', which is not a real date"
            ) from None

        pairing = re.search(r"[Pp]airs with `candid-core` (\S+)", body)
        if not pairing:
            raise SystemExit(
                f"the changelog entry for {version} names no `candid-core` "
                "pairing, which is exactly what the README promises it does"
            )

        # Every `cargo install candid-core --version X` the README tells a
        # reader to run must name the version this release pairs with. The
        # on-ramp once shipped pointing at a crate release that predated the
        # `--envelope` flag the very next line invokes, and nothing caught it:
        # only ```ts blocks are compiled, so ```sh is unverified prose.
        paired = pairing.group(1).rstrip(".")
        readme_text = (extracted / "README.md").read_text()
        pinned = set(
            re.findall(
                r"cargo install candid-core --version (\S+)", readme_text
            )
        )
        if pinned - {paired}:
            raise SystemExit(
                f"the shipped README tells the reader to install candid-core "
                f"{sorted(pinned)}, but this release pairs with {paired}. The "
                "on-ramp must install the version it was verified against."
            )

        # A declaration line count stated in the released entry has to match
        # the declarations actually being packed. This is not a guard against
        # some later release moving the file — a published entry describes its
        # own version forever — but against the entry being authored early and
        # the artifact drifting underneath it before the dispatch, which is
        # exactly how the 0.1.1-era figure went stale. So it applies only at
        # release: while the changelog still carries an `## Unreleased`
        # section, the tree is development past the packed version, whose
        # published entry measured the artifact it shipped, not this one.
        # Release prep renames that section, which re-arms the check.
        developing = "Unreleased" in entries
        for stated_from, stated_to in ([] if developing else re.findall(
            r"`schema\.d\.ts` grows from (\d+) lines to (\d+)", body
        )):
            actual = len(
                (extracted / "dist" / "schema.d.ts").read_text().splitlines()
            )
            if int(stated_to) != actual:
                raise SystemExit(
                    f"the changelog entry for {version} says `schema.d.ts` "
                    f"grows from {stated_from} lines to {stated_to}, but the "
                    f"packed declarations are {actual} lines. A measured number "
                    "in a shipped changelog must match the artifact it ships in."
                )

        # Shipped doc comments must stand on their own. The pattern is broad
        # on purpose — `#` followed by a digit, anywhere in a declaration
        # file: one historical citation was line-wrapped ("issue\n * #104"),
        # which a narrower "issue #N" pattern reads straight past, and no
        # legitimate shipped text has that shape today.
        cited = [
            f"dist/{path.name}:{number}: {line.strip()}"
            for path in sorted(extracted.glob("dist/*.d.ts"))
            for number, line in enumerate(path.read_text().splitlines(), 1)
            if re.search(r"#\d", line)
        ]
        if cited:
            raise SystemExit(
                "shipped declarations cite internal issue numbers; a consumer's "
                "editor hover cannot follow them, so rewrite each as a "
                "self-contained explanation:\n  " + "\n  ".join(cited)
            )

        # A consumer of every exported subpath, in a scratch tree that has no
        # `@icp-sdk/core` at all — nothing installs one, and the assertion
        # below makes that a checked fact rather than an assumption. The
        # nominal `SdkStylePrincipal` class stands in for an SDK `Principal`:
        # a class with a private member and `toText()` converts through
        # `principal()` into the branded canonical text the schemas ask for,
        # while the unconverted instance is refused — encode is strict (the
        # repository's own suite runs the same claims against the real pinned
        # SDK). A decoded principal is then checked to be plain data.
        if (scratch / "node_modules" / "@icp-sdk").exists():
            raise SystemExit("the scratch consumer tree must not contain @icp-sdk")
        consumer = scratch / "consumer"
        consumer.mkdir()
        (consumer / "package.json").write_text('{ "type": "module" }\n')
        (consumer / "main.ts").write_text(
            'import { c, isPrincipal, principal, type Infer, type Principal } from "@candid-core/schema";\n'
            'import { validate, unwrapResult } from "@candid-core/schema/validate";\n'
            'import { encode, decode } from "@candid-core/schema/codec";\n'
            'import { schemaFromContract } from "@candid-core/schema/contract";\n'
            "\n"
            "class SdkStylePrincipal {\n"
            "  private readonly _isPrincipal = true;\n"
            '  toText(): string { return "aaaaa-aa"; }\n'
            "}\n"
            "\n"
            "const Account = c.record({ owner: c.principal, balance: c.nat });\n"
            "type Account = Infer<typeof Account>;\n"
            'const text: Principal = principal("ryjl3-tyaaa-aaaaa-aaaba-cai");\n'
            "const sdk = new SdkStylePrincipal();\n"
            "for (const owner of [text, principal(sdk)]) {\n"
            "  const value: Account = { owner, balance: 5n };\n"
            "  const checked = validate(Account, value);\n"
            '  if (!checked.ok) throw new Error("validate");\n'
            "  const bytes = encode(Account, value);\n"
            '  if (!bytes.ok) throw new Error("encode");\n'
            "  const back = decode(Account, bytes.bytes);\n"
            '  if (!back.ok) throw new Error("decode");\n'
            "  const decoded = back.value as Account;\n"
            '  if (decoded.balance !== 5n) throw new Error("round trip");\n'
            "  // A decoded principal is the canonical text: plain, comparable data.\n"
            '  if (decoded.owner !== owner || !isPrincipal(decoded.owner)) throw new Error("principal text");\n'
            '  if (JSON.stringify(decoded.owner) !== JSON.stringify(owner)) throw new Error("json");\n'
            '  if (structuredClone(decoded).owner !== owner) throw new Error("clone");\n'
            "}\n"
            "// Encode is strict: an unconverted object with toText() is refused,\n"
            "// exactly as validate refuses it.\n"
            "const raw = { owner: sdk, balance: 5n } as unknown as Account;\n"
            'if (validate(Account, raw).ok || encode(Account, raw).ok) throw new Error("strict");\n'
            "let refused = false;\n"
            'try { principal("AAAAA-AA"); } catch (error) { refused = error instanceof TypeError; }\n'
            'if (!refused) throw new Error("principal() must refuse non-canonical text");\n'
            "const Reply = c.variant({ ok: c.nat, err: c.text });\n"
            'const outcome = unwrapResult(Reply, { tag: "ok", value: 1n });\n'
            'if (!outcome.ok || outcome.value !== 1n) throw new Error("unwrap");\n'
            "const loaded = schemaFromContract({});\n"
            'if (loaded.ok) throw new Error("an empty document must be refused");\n'
            'console.log("npm package smoke: ok");\n'
        )
        (consumer / "tsconfig.json").write_text(
            json.dumps(
                {
                    "compilerOptions": {
                        "strict": True,
                        "noEmit": True,
                        "module": "nodenext",
                        "moduleResolution": "nodenext",
                        "target": "es2022",
                        # Deliberately NOT skipLibCheck: the shipped
                        # declarations must compile as they stand.
                        "skipLibCheck": False,
                    },
                    "include": ["main.ts"],
                }
            )
        )
        # The pinned compiler from the package's own lockfile.
        tsc = PACKAGE / "node_modules" / ".bin" / "tsc"
        run([str(tsc), "-p", str(consumer / "tsconfig.json")], consumer)
        # Execute the emitted behavior under plain node, from the artifact.
        run(["node", "--experimental-strip-types", "--no-warnings", "main.ts"], consumer)

        # The shipped README, compiled against the shipped package. Each block
        # is its own file, so an example that silently leans on an earlier
        # block's bindings fails here — which is the point: a reader copies one
        # block, not the file. `node:fs` is stubbed rather than pulled from
        # `@types/node`: this gate adds no supply-chain surface to compile one
        # signature.
        readme = scratch / "readme"
        readme.mkdir()
        (readme / "package.json").write_text('{ "type": "module" }\n')
        (readme / "node-stub.d.ts").write_text(
            'declare module "node:fs" {\n'
            "  export function readFileSync(path: URL | string, encoding: \"utf8\"): string;\n"
            "}\n"
        )
        blocks = typescript_blocks((extracted / "README.md").read_text())
        if not blocks:
            raise SystemExit("no compilable ```ts blocks found in the shipped README")
        for number, block in enumerate(blocks, 1):
            (readme / f"block{number}.ts").write_text(block)
        (readme / "tsconfig.json").write_text(
            json.dumps(
                {
                    "compilerOptions": {
                        "strict": True,
                        "noEmit": True,
                        "module": "nodenext",
                        "moduleResolution": "nodenext",
                        "target": "es2022",
                        "skipLibCheck": False,
                    },
                    "include": ["*.ts"],
                }
            )
        )
        readme_check = subprocess.run(
            [str(tsc), "-p", str(readme / "tsconfig.json")],
            cwd=readme,
            capture_output=True,
            text=True,
        )
        if readme_check.returncode != 0:
            numbered = "\n".join(
                f"  block{number}.ts:\n"
                + "\n".join(f"    {line}" for line in block.rstrip().splitlines())
                for number, block in enumerate(blocks, 1)
            )
            raise SystemExit(
                "the shipped README's TypeScript does not compile against the "
                "packed artifact:\n"
                f"{readme_check.stdout}{readme_check.stderr}\n"
                f"blocks, in README order:\n{numbered}"
            )

        # The removed subpaths, and deep paths into `dist/`, must not resolve.
        # At runtime Node refuses them with ERR_PACKAGE_PATH_NOT_EXPORTED —
        # checked per specifier in a fresh process each, so one resolution
        # cannot mask another; `./package.json` is in the deep list because
        # the map does not export it either.
        refused = []
        for subpath in REMOVED + DEEP:
            specifier = "@candid-core/schema" + subpath[1:]
            probe = subprocess.run(
                [
                    "node",
                    "--input-type=module",
                    "-e",
                    f"await import({json.dumps(specifier)});",
                ],
                cwd=consumer,
                capture_output=True,
                text=True,
            )
            output = probe.stdout + probe.stderr
            if probe.returncode == 0 or "ERR_PACKAGE_PATH_NOT_EXPORTED" not in output:
                raise SystemExit(
                    f"importing {specifier} from the packed artifact must fail "
                    f"with ERR_PACKAGE_PATH_NOT_EXPORTED; got exit "
                    f"{probe.returncode}:\n{output}"
                )
            refused.append(specifier)

        # And at compile time: each removed subpath is its own file, so every
        # one must be named in the compiler's missing-module diagnostics.
        removed_types = scratch / "removed"
        removed_types.mkdir()
        (removed_types / "package.json").write_text('{ "type": "module" }\n')
        for index, subpath in enumerate(REMOVED):
            specifier = "@candid-core/schema" + subpath[1:]
            (removed_types / f"removed{index}.ts").write_text(
                f"import * as removed from {json.dumps(specifier)};\n"
                "void removed;\n"
            )
        (removed_types / "tsconfig.json").write_text(
            json.dumps(
                {
                    "compilerOptions": {
                        "strict": True,
                        "noEmit": True,
                        "module": "nodenext",
                        "moduleResolution": "nodenext",
                        "target": "es2022",
                        "skipLibCheck": False,
                    },
                    "include": ["*.ts"],
                }
            )
        )
        compiled = subprocess.run(
            [str(tsc), "-p", str(removed_types / "tsconfig.json")],
            cwd=removed_types,
            capture_output=True,
            text=True,
        )
        # Matched against both streams: the pinned tsc writes diagnostics to
        # stdout today, but the assertion should not hinge on which stream a
        # future compiler picks.
        diagnostics = compiled.stdout + compiled.stderr
        unrefused = [
            subpath
            for index, subpath in enumerate(REMOVED)
            if not re.search(
                rf"removed{index}\.ts\(\d+,\d+\): error TS2307: .*"
                + re.escape("@candid-core/schema" + subpath[1:]),
                diagnostics,
            )
        ]
        if compiled.returncode == 0 or unrefused:
            raise SystemExit(
                f"removed subpaths {unrefused} still type-check against the "
                f"packed artifact; expected TS2307 for each:\n{diagnostics}"
            )
    print("npm package artifact verified: manifest file list, exact export map "
          f"{EXPORTED}, no dependency or peer, no @icp-sdk/core import, "
          "homepage, changelog entry and pairing, self-contained doc comments, "
          "strict compile without skipLibCheck with no @icp-sdk/core installed, "
          f"executed round-trip, {len(blocks)} README block(s) compiled, "
          f"{len(REMOVED)} removed subpaths refused at compile time, "
          f"{len(refused)} specifiers refused at runtime "
          "(ERR_PACKAGE_PATH_NOT_EXPORTED)")


if __name__ == "__main__":
    sys.exit(main())
