#!/usr/bin/env python3
"""Independent verifier for the actorless conformance vector (issue #13).

Reproduces the actorless Contract's canonical identity bytes and ID from the
checked-in wire fixture using only the Python standard library — no part of
the Rust implementation is involved. It rebuilds the identity payload from
``actorless.contract.json``, serializes it with its own JCS writer, prepends
the identity domain and zero byte, hashes with SHA-256, and compares every
intermediate against the pins in ``actorless.identity.json``.

Run from anywhere:

    python3 tests/fixtures/conformance/verify_actorless.py

Exit status 0 means every pinned value was reproduced.

Scope: this deliberately verifies the actorless vector only. It is not the
full language-independent canonicalization reference implementation (that is
tracked separately), so it implements exactly the JCS subset this vector
exercises — sorted object keys, escape-free ASCII strings, and integers well
inside the IEEE-754 exact range — and fails loudly if the vector ever leaves
that subset.
"""

import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent

# The v1 contract identity payload is the canonical Contract minus its
# `identities` (the value being derived) and `producer` (explicitly outside
# authenticated identity). An absent actor is OMITTED — never `null` — in
# both the wire Contract and this payload.
IDENTITY_PAYLOAD_PROPERTIES = (
    "format",
    "format_version",
    "semantics_profile",
    "canonicalization_profile",
    "types",
    "declarations",
)


def assert_within_jcs_subset(value):
    """Guard the subset in which json.dumps matches RFC 8785 byte-for-byte.

    Sorted keys plus compact separators reproduce JCS exactly when every
    string (key or value) serializes without escapes and identically under
    UTF-16 code-unit ordering — true for printable ASCII without quotes or
    backslashes — and every number is an integer of magnitude below 2**53.
    """
    if isinstance(value, dict):
        for key, item in value.items():
            assert_within_jcs_subset(key)
            assert_within_jcs_subset(item)
    elif isinstance(value, list):
        for item in value:
            assert_within_jcs_subset(item)
    elif isinstance(value, str):
        if not all(0x20 <= ord(ch) <= 0x7E and ch not in '"\\' for ch in value):
            raise AssertionError(
                f"string {value!r} needs JCS escaping rules this verifier does not implement"
            )
    elif isinstance(value, bool) or value is None:
        raise AssertionError(
            "booleans and nulls do not appear in this vector; extend the verifier deliberately"
        )
    elif isinstance(value, int):
        if abs(value) >= 2**53:
            raise AssertionError(f"integer {value} leaves the IEEE-754 exact range")
    else:
        raise AssertionError(f"unsupported JSON value type: {type(value).__name__}")


def jcs_bytes(payload):
    assert_within_jcs_subset(payload)
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def main():
    wire = json.loads((HERE / "actorless.contract.json").read_text())
    pins = json.loads((HERE / "actorless.identity.json").read_text())

    failures = []

    def expect(name, computed, pinned):
        if computed != pinned:
            failures.append(f"{name}\n  computed: {computed}\n  pinned:   {pinned}")

    if "actor" in wire:
        failures.append('the actorless wire fixture must omit the "actor" property entirely')

    payload = {name: wire[name] for name in IDENTITY_PAYLOAD_PROPERTIES}
    jcs = jcs_bytes(payload)
    domain = pins["domain"]
    preimage = domain.encode() + b"\x00" + jcs
    contract_id = f"{domain}:sha256:{hashlib.sha256(preimage).hexdigest()}"

    expect("canonical JCS text", jcs.decode(), pins["jcs"])
    expect("canonical JCS bytes (hex)", jcs.hex(), pins["jcs_hex"])
    expect("domain preimage bytes (hex)", preimage.hex(), pins["preimage_hex"])
    expect("contract ID", contract_id, pins["contract_id"])
    expect("wire fixture contract ID", wire["identities"]["contract"], contract_id)

    if failures:
        print("actorless vector verification FAILED:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"actorless vector verified independently: {contract_id}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
