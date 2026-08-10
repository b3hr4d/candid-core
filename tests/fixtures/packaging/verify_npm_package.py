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

The type-only `@icp-sdk/core` peer is installed as a minimal stub providing
exactly the `Principal` surface the declarations reference. That keeps the
gate honest in both directions: `Principal` stays a real nominal type (so a
wrong value is a compile error, not silently `any`), and the missing-peer
case is asserted separately as a clear, actionable error.
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


def run(args, cwd):
    subprocess.run(args, cwd=cwd, check=True)


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
        modules = ["actor", "codec", "contract", "forms", "labels", "schema", "validate"]
        expected = sorted(
            ["LICENSE", "README.md", "package.json"]
            + [f"dist/{name}.js" for name in modules]
            + [f"dist/{name}.d.ts" for name in modules]
        )
        if shipped != expected:
            raise SystemExit(
                f"the packed file list is not the manifest's promise:\n"
                f"  shipped:  {shipped}\n  expected: {expected}"
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
        # nominal type in the consumer, or the compile proves nothing.
        peer = scratch / "node_modules" / "@icp-sdk" / "core"
        (peer / "principal").mkdir(parents=True)
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
                        }
                    },
                }
            )
        )
        (peer / "principal" / "index.d.ts").write_text(
            "export declare class Principal {\n"
            "  private readonly _isPrincipal: true;\n"
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

        # The peer really is required for types: without it the failure must
        # be the clear missing-module error the README documents, not a
        # silent `any`.
        shutil.rmtree(scratch / "node_modules" / "@icp-sdk")
        missing = subprocess.run(
            [str(tsc), "-p", str(consumer / "tsconfig.json")],
            cwd=consumer,
            capture_output=True,
            text=True,
        )
        if missing.returncode == 0 or "@icp-sdk/core/principal" not in missing.stdout:
            raise SystemExit(
                "expected a missing-peer type error naming @icp-sdk/core/principal, got:\n"
                f"{missing.stdout}{missing.stderr}"
            )
    print("npm package artifact verified: manifest file list, self-contained doc "
          "comments, root + 6 subpaths, strict compile without skipLibCheck, "
          "executed round-trip, peer requirement")


if __name__ == "__main__":
    sys.exit(main())
