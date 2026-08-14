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
be compiled here — the `@icp-sdk/core` agent adapter, whose SDK is a type-only
peer this repository's lockfile deliberately does not carry — is opted out by
an HTML comment naming the reason, so the exemption is visible in the README
rather than implicit in this script.

The changelog is treated as an artifact claim rather than a courtesy. It ships
in the tarball, and the gate refuses one that does not document the version
being packed together with the `candid-core` version it pairs with — the
README promises exactly that, and a promise about an artifact belongs in the
artifact's gate.

The `@icp-sdk/core` peer is installed as a minimal stub providing exactly
the surfaces the artifact and this gate's consumers reference: the
`Principal` class (the full consumer proves an SDK-class value is accepted
where the schemas ask for the structural `PrincipalValue` — issue #150) and
the `./agent` names the `./transport-icp` module uses (issue #154). The
peer contract is then asserted in both directions with the peer removed: a
consumer of every subpath except `./transport-icp` compiles and runs with
no `@icp-sdk/core` at all, while the transport-importing consumer fails
with the clear missing-module error rather than a silent `any`.
"""

import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[3]
PACKAGE = REPO / "crates" / "candid-core-ts" / "ts"

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
        modules = [
            "actor",
            "codec",
            "contract",
            "forms",
            "labels",
            "schema",
            "transport-icp",
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
            name: body
            for name, body in re.findall(
                r"^## (\S+)(.*?)(?=^## |\Z)", changelog, re.M | re.S
            )
        }
        if version not in entries:
            raise SystemExit(
                f"the shipped changelog documents no '## {version}' entry, so "
                f"the packed version {version} arrives on npm undocumented. "
                f"Headings found: {sorted(entries)}"
            )
        if not re.search(r"[Pp]airs with `candid-core` \S+", entries[version]):
            raise SystemExit(
                f"the changelog entry for {version} names no `candid-core` "
                "pairing, which is exactly what the README promises it does"
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

        # The type-only peer, as a minimal stub: `Principal` must stay a real
        # nominal type in the consumer, or the compile proves nothing. The
        # `./agent` subpath joined for the transport adapter (issue #154): its
        # shipped declaration imports the `Agent` type and its shipped module
        # imports the `HttpAgent` and `QueryResponseStatus` values, so both
        # halves are stubbed with exactly those names. The *real* `@icp-sdk/
        # core@6.1.0` surface is what the repository's own tsc gate and mock-
        # fetch suite compile and execute against; this stub only keeps the
        # packaged-artifact compile hermetic.
        peer = scratch / "node_modules" / "@icp-sdk" / "core"
        (peer / "principal").mkdir(parents=True)
        (peer / "agent").mkdir(parents=True)
        (peer / "package.json").write_text(
            json.dumps(
                {
                    "name": "@icp-sdk/core",
                    "version": "6.0.0",
                    "type": "module",
                    "exports": {
                        "./principal": {
                            "types": "./principal/index.d.ts",
                            "default": "./principal/index.js",
                        },
                        "./agent": {
                            "types": "./agent/index.d.ts",
                            "default": "./agent/index.js",
                        },
                    },
                }
            )
        )
        (peer / "agent" / "index.d.ts").write_text(
            "export interface Agent {\n"
            "  readonly rootKey: Uint8Array | null;\n"
            "}\n"
            "export declare class HttpAgent {\n"
            "  private readonly _isHttpAgent: true;\n"
            "  static create(options?: unknown): Promise<HttpAgent>;\n"
            "}\n"
            "export declare enum QueryResponseStatus {\n"
            '  Replied = "replied",\n'
            '  Rejected = "rejected",\n'
            "}\n"
        )
        (peer / "agent" / "index.js").write_text(
            "export class HttpAgent {\n"
            "  static create() { return Promise.resolve(new HttpAgent()); }\n"
            "}\n"
            'export const QueryResponseStatus = { Replied: "replied", Rejected: "rejected" };\n'
        )
        (peer / "principal" / "index.d.ts").write_text(
            "export declare class Principal {\n"
            "  private readonly _isPrincipal: true;\n"
            "  static fromText(text: string): Principal;\n"
            "  toText(): string;\n"
            "}\n"
        )
        # A runtime shape the validator accepts: canonical management-canister
        # text, exactly what a real Principal's toText() returns.
        (peer / "principal" / "index.js").write_text(
            "export class Principal {\n"
            '  toText() { return "aaaaa-aa"; }\n'
            "}\n"
        )

        consumer = scratch / "consumer"
        consumer.mkdir()
        (consumer / "package.json").write_text('{ "type": "module" }\n')
        (consumer / "main.ts").write_text(
            'import { c, type Infer } from "@candid-core/schema";\n'
            'import { validate } from "@candid-core/schema/validate";\n'
            'import { encode, decode } from "@candid-core/schema/codec";\n'
            'import { schemaFromContract } from "@candid-core/schema/contract";\n'
            'import { createActor, type Transport } from "@candid-core/schema/actor";\n'
            'import { httpTransport } from "@candid-core/schema/transport-icp";\n'
            'import { formModel } from "@candid-core/schema/forms";\n'
            'import { candidLabelHash } from "@candid-core/schema/labels";\n'
            'import { Principal } from "@icp-sdk/core/principal";\n'
            "\n"
            "const Account = c.record({ owner: c.principal, balance: c.nat });\n"
            "type Account = Infer<typeof Account>;\n"
            'const owner = new Principal();\n'
            "const value: Account = { owner, balance: 5n };\n"
            "const checked = validate(Account, value);\n"
            'if (!checked.ok) throw new Error("validate");\n'
            "const bytes = encode(Account, value);\n"
            'if (!bytes.ok) throw new Error("encode");\n'
            "const back = decode(Account, bytes.bytes);\n"
            'if (!back.ok) throw new Error("decode");\n'
            'if ((back.value as Account).balance !== 5n) throw new Error("round trip");\n'
            'if (formModel(Account).control !== "group") throw new Error("forms");\n'
            'if (candidLabelHash("a") !== 97) throw new Error("labels");\n'
            "void schemaFromContract;\n"
            "void createActor;\n"
            "const transportSlot: Transport | undefined = undefined;\n"
            "void transportSlot;\n"
            "const adapterSlot: (() => Transport) | undefined = httpTransport;\n"
            "void adapterSlot;\n"
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
        # `@types/node`, in the same spirit as the peer above: this gate adds
        # no supply-chain surface to compile four signatures.
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

        # The issue #150 peer contract, asserted in both directions with the
        # peer removed entirely. A consumer of everything EXCEPT
        # `./transport-icp` — principal fields included, as structural
        # `PrincipalValue`s — must compile and run with no `@icp-sdk/core`
        # installed at all: the shipped declarations no longer import the SDK
        # anywhere else, and this is what keeps that claim honest. The full
        # consumer, which imports the transport subpath (and the SDK class
        # itself), must still fail with the clear missing-module error rather
        # than a silent `any`.
        shutil.rmtree(scratch / "node_modules" / "@icp-sdk")
        peerless = scratch / "peerless"
        peerless.mkdir()
        (peerless / "package.json").write_text('{ "type": "module" }\n')
        (peerless / "main.ts").write_text(
            'import { c, type Infer, type PrincipalValue } from "@candid-core/schema";\n'
            'import { validate } from "@candid-core/schema/validate";\n'
            'import { encode, decode } from "@candid-core/schema/codec";\n'
            'import { schemaFromContract } from "@candid-core/schema/contract";\n'
            'import { createActor, type Transport } from "@candid-core/schema/actor";\n'
            'import { formModel } from "@candid-core/schema/forms";\n'
            'import { candidLabelHash } from "@candid-core/schema/labels";\n'
            "\n"
            "const Account = c.record({ owner: c.principal, balance: c.nat });\n"
            "type Account = Infer<typeof Account>;\n"
            'const owner: PrincipalValue = { toText: () => "ryjl3-tyaaa-aaaaa-aaaba-cai" };\n'
            "const value: Account = { owner, balance: 5n };\n"
            "const checked = validate(Account, value);\n"
            'if (!checked.ok) throw new Error("validate");\n'
            "const bytes = encode(Account, value);\n"
            'if (!bytes.ok) throw new Error("encode");\n'
            "const back = decode(Account, bytes.bytes);\n"
            'if (!back.ok) throw new Error("decode");\n'
            'if ((back.value as Account).balance !== 5n) throw new Error("round trip");\n'
            'if (formModel(Account).control !== "group") throw new Error("forms");\n'
            'if (candidLabelHash("a") !== 97) throw new Error("labels");\n'
            "void schemaFromContract;\n"
            "void createActor;\n"
            "const transportSlot: Transport | undefined = undefined;\n"
            "void transportSlot;\n"
            'console.log("npm package peerless smoke: ok");\n'
        )
        (peerless / "tsconfig.json").write_text(
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
                    "include": ["main.ts"],
                }
            )
        )
        run([str(tsc), "-p", str(peerless / "tsconfig.json")], peerless)
        run(["node", "--experimental-strip-types", "--no-warnings", "main.ts"], peerless)

        missing = subprocess.run(
            [str(tsc), "-p", str(consumer / "tsconfig.json")],
            cwd=consumer,
            capture_output=True,
            text=True,
        )
        if missing.returncode == 0 or "@icp-sdk/core" not in missing.stdout:
            raise SystemExit(
                "expected a missing-peer type error naming @icp-sdk/core, got:\n"
                f"{missing.stdout}{missing.stderr}"
            )
    print("npm package artifact verified: manifest file list, homepage, changelog "
          "entry and pairing, self-contained doc comments, root + 7 subpaths, "
          "strict compile without skipLibCheck, executed round-trip, "
          f"{len(blocks)} README block(s) compiled, peer contract in both "
          "directions")


if __name__ == "__main__":
    sys.exit(main())
