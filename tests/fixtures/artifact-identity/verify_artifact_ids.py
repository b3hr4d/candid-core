#!/usr/bin/env python3
"""Independent reference implementation of artifact identity (issue #25).

A standard-library-only recomputation of every vector in ``manifest.json``,
with no part of the Rust implementation involved. Run from anywhere:

    python3 tests/fixtures/artifact-identity/verify_artifact_ids.py

Exit status 0 means every pinned value of every vector was reproduced.

The construction it checks is deliberately tiny, and that is the security
argument: for a frozen per-kind domain ``D`` and the exact artifact octets
``B``,

    preimage    = D encoded as UTF-8 || 0x00 || B
    artifact_id = D + ":sha256:" + lowercase_hex(SHA-256(preimage))

There is no canonicalization step, no parse, no key sorting, no length field,
and no second kind label. An artifact identity claims equality of the kind and
of the octet sequence, under the SHA-256 collision assumption, and nothing
else: not semantic equality, not structural validity, not authenticity, not
producer truth, and not signature trust. No unkeyed content ID authenticates
itself; a signature or other external mechanism is what authenticates an ID.

Four properties are checked, and each maps to something issue #25 asked for:

1. **Golden IDs.** Every vector's pinned ``artifact_id`` is recomputed from the
   file's bytes on disk, and the ``empty`` vector additionally pins the whole
   preimage as hex so the domain framing itself is anchored byte for byte.
2. **Kind separation.** The vectors listed under ``cross_kind`` are re-hashed
   under another kind and must produce different, separately pinned IDs — so
   identical bytes can never be confused across kinds.
3. **The semantic Contract identity does not move.** Every member of
   ``shared_contract_id`` embeds the identical ``contract_id`` while every
   member has a distinct ``artifact_id``. Producer rewrites, extension edits,
   sidecar edits, and re-encoding all change the artifact identity and leave
   ``contract_id`` exactly where it was — including in a raw Contract document,
   whose octets do contain ``producer`` even though no semantic identity covers
   it.
4. **Coverage.** Every name in ``required_cases`` is present, so deleting a
   vector fails here instead of silently shrinking the evidence.

Scope: this is a conformance reference for identity bytes, not a validator.
It never claims a vector is a valid artifact — the ``empty`` vector is
deliberately not one — because computing an artifact identity is not a
validity claim in the first place.

This manifest is separate from ``tests/fixtures/conformance/manifest.json`` on
purpose. That one is the closed semantic canonicalization conformance set for
ADR 0002 and is untouched by this file.
"""

import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent

# The frozen, self-describing domain tags. Duplicated here rather than read
# from the manifest so a manifest edit cannot quietly redefine a domain and
# still pass.
DOMAINS = {
    "ContractJsonV1": "candid-core:artifact:contract-json:v1",
    "ContractEnvelopeJsonV1": "candid-core:artifact:contract-envelope-json:v1",
    "CompilationJsonV1": "candid-core:artifact:compilation-json:v1",
}

failures = []


def check(condition, message):
    if not condition:
        failures.append(message)
    return condition


def artifact_id(kind: str, payload: bytes) -> str:
    """SHA-256 over ``<domain> || 0x00 || <payload>``, rendered with its domain."""
    domain = DOMAINS[kind]
    digest = hashlib.sha256(domain.encode("utf-8") + b"\x00" + payload).hexdigest()
    return f"{domain}:sha256:{digest}"


def read(relative: str) -> bytes:
    path = HERE / relative
    if not path.is_file():
        raise SystemExit(f"missing artifact fixture: {path}")
    return path.read_bytes()


def main() -> int:
    manifest = json.loads((HERE / "manifest.json").read_text(encoding="utf-8"))

    check(
        manifest.get("manifest_version") == 1,
        "manifest_version must be 1",
    )
    check(
        manifest.get("domains") == DOMAINS,
        f"manifest domains {manifest.get('domains')!r} disagree with this reference",
    )

    vectors = {}
    for vector in manifest["vectors"]:
        name = vector["name"]
        check(name not in vectors, f"{name}: duplicate vector name")
        vectors[name] = vector

        kind = vector["kind"]
        check(kind in DOMAINS, f"{name}: unknown kind {kind!r}")
        payload = read(vector["file"])

        if "preimage_hex" in vector:
            expected = DOMAINS[kind].encode("utf-8") + b"\x00" + payload
            check(
                expected.hex() == vector["preimage_hex"],
                f"{name}: preimage mismatch\n  expected {vector['preimage_hex']}\n  computed {expected.hex()}",
            )

        computed = artifact_id(kind, payload)
        check(
            computed == vector["artifact_id"],
            f"{name}: artifact_id mismatch\n  expected {vector['artifact_id']}\n  computed {computed}",
        )
        check(
            computed.startswith(DOMAINS[kind] + ":sha256:"),
            f"{name}: rendered ID must carry its own domain",
        )
        check(
            len(computed.rsplit(":", 1)[1]) == 64
            and all(c in "0123456789abcdef" for c in computed.rsplit(":", 1)[1]),
            f"{name}: digest must be 64 lowercase hex digits",
        )

    missing = [name for name in manifest["required_cases"] if name not in vectors]
    check(not missing, f"manifest is missing required cases: {missing}")

    # 2. The same bytes under another kind are a different identity.
    for entry in manifest["cross_kind"]:
        source = vectors[entry["vector"]]
        check(
            entry["kind"] != source["kind"],
            f"{entry['vector']}: cross-kind entry must name a different kind",
        )
        computed = artifact_id(entry["kind"], read(source["file"]))
        check(
            computed == entry["artifact_id"],
            f"{entry['vector']} as {entry['kind']}: mismatch\n  expected {entry['artifact_id']}\n  computed {computed}",
        )
        check(
            computed != source["artifact_id"],
            f"{entry['vector']}: identical bytes must not share an ID across kinds",
        )

    # 3. One semantic Contract identity, many artifact identities.
    group = manifest["shared_contract_id"]
    paths = group["contract_id_path"]
    check(
        set(paths) == set(DOMAINS),
        f"contract_id_path {sorted(paths)} must name every kind",
    )
    seen = {}
    for name in group["members"]:
        vector = vectors[name]
        document = json.loads(read(vector["file"]).decode("utf-8"))
        # A raw Contract document carries `identities` at its root; an envelope
        # and a compilation nest the same Contract one level down.
        embedded = document
        for step in paths[vector["kind"]]:
            embedded = embedded[step]
        check(
            embedded == group["contract_id"],
            f"{name}: embedded contract_id {embedded} is not the shared {group['contract_id']}",
        )
        check(
            vector["contract_id"] == group["contract_id"],
            f"{name}: manifest contract_id disagrees with the shared value",
        )
        check(
            vector["artifact_id"] not in seen,
            f"{name}: artifact_id collides with {seen.get(vector['artifact_id'])}",
        )
        seen[vector["artifact_id"]] = name
    check(
        len(seen) == len(group["members"]),
        "every member of the shared-contract_id group must have a distinct artifact_id",
    )
    check(
        len({vectors[name]["kind"] for name in group["members"]}) == len(DOMAINS),
        "the shared-contract_id group must span every artifact kind",
    )

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"\n{len(failures)} artifact identity check(s) failed", file=sys.stderr)
        return 1

    print(
        f"reproduced {len(vectors)} artifact identity vector(s) across "
        f"{len(DOMAINS)} kind(s), {len(manifest['cross_kind'])} cross-kind pin(s), "
        f"and {len(group['members'])} distinct artifact IDs over one shared contract_id"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
