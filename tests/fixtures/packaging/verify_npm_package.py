#!/usr/bin/env python3
"""The packaged-consumer smoke for @candid-core/schema (issue #106).

Builds the package, packs the exact artifact `npm publish` would ship,
extracts it into a scratch consumer, and proves the artifact stands alone:
the root and every subpath export resolve, compile under the pinned strict
TypeScript, and execute a real encode/validate round-trip under node.
Consumption goes through the extracted tarball only — never the repository
sources — so a manifest that ships the wrong files fails here, not on npm.
"""

import json
import pathlib
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
            "\n"
            "const Account = c.record({ owner: c.principal, balance: c.nat });\n"
            "type Account = Infer<typeof Account>;\n"
            'const value: Account = { owner: { toText: () => "aaaaa-aa" }, balance: 5n };\n'
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
                        # The optional type-only @icp-sdk peer is absent in
                        # the consumer; its specifier only appears inside the
                        # package's declaration files.
                        "skipLibCheck": True,
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
    print("npm package artifact verified: root + 6 subpaths, compiled and executed")


if __name__ == "__main__":
    sys.exit(main())
