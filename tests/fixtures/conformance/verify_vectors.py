#!/usr/bin/env python3
"""Independent reference canonicalizer for every conformance vector (issue #14).

This is a complete, standard-library-only implementation of the
``candid-core-canon-1`` canonicalization algorithm specified normatively in
``docs/canonicalization-v1.md``. It supersedes the earlier actorless-only
``verify_actorless.py``. No part of the Rust implementation is involved: for
every raw graph vector named by ``manifest.json`` — noncanonical
arrangements alongside already-canonical forms that pin idempotence — it
independently

  1. validates the fixture-format graph (shapes, u32 ranges, reference
     targets, unique field IDs, unique method names, Candid method-ID hashes,
     oneway arity, class placement, actor shape, reachability, Unicode
     scalar-sequence strings),
  2. computes the semantic partition refinement and quotient graph,
  3. deterministically re-indexes the quotient (actor first, declaration
     roots by quotient type ID, defensive remaining roots, iterative
     depth-first preorder),
  4. renders the constrained canonical JSON profile with its own writer,
  5. builds the exact Contract and interface identity payloads, applies the
     domain separator and SHA-256,

and compares every intermediate — canonical graph, canonical JSON text and
UTF-8 hex, domain-preimage hex, and the resulting IDs — against the pinned
expected fixtures. It also cross-checks the checked-in wire fixtures and the
byte-level actorless identity pins, and fails if any required scenario is
missing from the manifest.

Run from anywhere:

    python3 tests/fixtures/conformance/verify_vectors.py

Exit status 0 means every pinned value of every vector was reproduced.

Scope: this is a conformance reference, not a production parser for
untrusted input. It is deliberately bounded (see ``BOUNDS``) and fails
loudly on anything outside the fixture vocabulary instead of trying to be
robust against adversarial documents. The bounds are implementation policy
of this verifier only; they are not part of Contract identity.
"""

import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent

CONTRACT_FORMAT = "candid-core"
FORMAT_VERSION = 1
SEMANTICS_PROFILE = "candid-1"
CANONICALIZATION_PROFILE = "candid-core-canon-1"
CONTRACT_DOMAIN = "candid-core:contract:v1"
INTERFACE_DOMAIN = "candid-core:interface:v1"

# Every scenario the manifest must cover. A vector file that disappears, or a
# manifest edit that drops a scenario, fails here instead of passing silently.
REQUIRED_CASES = (
    "actorless",
    "empty_actor",
    "class",
    "basic",
    "recursive",
    "mutual_recursion",
    "hash_collision",
    "unicode",
    "duplicate_semantic_nodes",
    "arena_permutation",
    "declaration_root_order",
)

# Scenarios whose point is that several distinct raw arenas converge to one
# canonical result; they must keep more than one raw input.
CONVERGENCE_CASES = ("duplicate_semantic_nodes", "arena_permutation")

U32_MAX = 2**32 - 1

PRIMITIVES = (
    "null", "bool", "nat", "int",
    "nat8", "nat16", "nat32", "nat64",
    "int8", "int16", "int32", "int64",
    "float32", "float64", "text", "reserved", "empty", "principal",
)
PRIMITIVE_TAG = {name: tag for tag, name in enumerate(PRIMITIVES)}
MODES = ("update", "query", "composite_query", "oneway")
MODE_TAG = {name: tag for tag, name in enumerate(MODES)}
NODE_TAG = {
    "primitive": 0, "opt": 1, "vec": 2, "record": 3,
    "variant": 4, "func": 5, "service": 6, "class": 7,
}
REFINEMENT_SEPARATOR = 0xFF

# Non-normative verifier bounds; generous for fixtures, small enough that a
# malformed vector cannot make this script do unbounded work.
BOUNDS = {
    "max_type_nodes": 4096,
    "max_declarations": 4096,
    "max_collection_len": 4096,
    "max_string_bytes": 65536,
}


class VectorError(Exception):
    """A vector violated the fixture format or a pinned value."""


def fail(message):
    raise VectorError(message)


def idl_hash(name):
    """Candid's 32-bit method/field label hash over UTF-8 bytes."""
    value = 0
    for byte in name.encode("utf-8"):
        value = (value * 223 + byte) % (2**32)
    return value


def utf8(text, what):
    """Encode, rejecting lone surrogates Python's json parser lets through."""
    if not isinstance(text, str):
        fail(f"{what} must be a string, found {type(text).__name__}")
    try:
        encoded = text.encode("utf-8")
    except UnicodeEncodeError:
        fail(f"{what} is not a valid Unicode scalar sequence (lone surrogate)")
    if len(encoded) > BOUNDS["max_string_bytes"]:
        fail(f"{what} exceeds the verifier string bound")
    return encoded


def check_u32(value, what):
    if not isinstance(value, int) or isinstance(value, bool):
        fail(f"{what} must be an integer, found {type(value).__name__}")
    if not 0 <= value <= U32_MAX:
        fail(f"{what} = {value} is outside the u32 range")
    return value


def check_keys(obj, required, what, optional=()):
    if not isinstance(obj, dict):
        fail(f"{what} must be an object")
    keys = set(obj)
    missing = set(required) - keys
    unknown = keys - set(required) - set(optional)
    if missing or unknown:
        fail(f"{what} has missing keys {sorted(missing)} / unknown keys {sorted(unknown)}")


# --------------------------------------------------------------------------
# Fixture-format graph validation
# --------------------------------------------------------------------------

def validate_node(node, index, node_count):
    what = f"types[{index}]"
    if not isinstance(node, dict) or "kind" not in node:
        fail(f"{what} must be an object with a kind")
    kind = node["kind"]

    def ref(value, member):
        check_u32(value, f"{what}.{member}")
        if value >= node_count:
            fail(f"{what}.{member} = {value} dangles outside the arena of {node_count} node(s)")
        return value

    if kind == "primitive":
        check_keys(node, ("kind", "primitive"), what)
        if node["primitive"] not in PRIMITIVE_TAG:
            fail(f"{what}.primitive {node['primitive']!r} is not a v1 primitive")
    elif kind in ("opt", "vec"):
        check_keys(node, ("kind", "inner"), what)
        ref(node["inner"], "inner")
    elif kind in ("record", "variant"):
        check_keys(node, ("kind", "fields"), what)
        fields = node["fields"]
        if not isinstance(fields, list) or len(fields) > BOUNDS["max_collection_len"]:
            fail(f"{what}.fields must be a bounded array")
        seen_ids = set()
        for position, field in enumerate(fields):
            member = f"fields[{position}]"
            check_keys(field, ("id", "type"), f"{what}.{member}")
            field_id = check_u32(field["id"], f"{what}.{member}.id")
            if field_id in seen_ids:
                fail(f"{what}.{member}.id {field_id} occurs more than once")
            seen_ids.add(field_id)
            ref(field["type"], f"{member}.type")
    elif kind == "func":
        check_keys(node, ("kind", "args", "results", "mode"), what)
        for member in ("args", "results"):
            values = node[member]
            if not isinstance(values, list) or len(values) > BOUNDS["max_collection_len"]:
                fail(f"{what}.{member} must be a bounded array")
            for position, value in enumerate(values):
                ref(value, f"{member}[{position}]")
        if node["mode"] not in MODE_TAG:
            fail(f"{what}.mode {node['mode']!r} is not a v1 method mode")
        if node["mode"] == "oneway" and node["results"]:
            fail(f"{what} is oneway but has results")
    elif kind == "service":
        check_keys(node, ("kind", "methods"), what)
        methods = node["methods"]
        if not isinstance(methods, list) or len(methods) > BOUNDS["max_collection_len"]:
            fail(f"{what}.methods must be a bounded array")
        seen_names = set()
        for position, method in enumerate(methods):
            member = f"methods[{position}]"
            check_keys(method, ("name", "id", "function"), f"{what}.{member}")
            name = method["name"]
            utf8(name, f"{what}.{member}.name")
            if not name:
                fail(f"{what}.{member}.name must not be empty")
            if name in seen_names:
                fail(f"{what}.{member}.name {name!r} occurs more than once")
            seen_names.add(name)
            method_id = check_u32(method["id"], f"{what}.{member}.id")
            if method_id != idl_hash(name):
                fail(
                    f"{what}.{member}.id {method_id} is not the Candid hash "
                    f"{idl_hash(name)} of {name!r}"
                )
            ref(method["function"], f"{member}.function")
    elif kind == "class":
        check_keys(node, ("kind", "init", "service"), what)
        init = node["init"]
        if not isinstance(init, list) or len(init) > BOUNDS["max_collection_len"]:
            fail(f"{what}.init must be a bounded array")
        for position, value in enumerate(init):
            ref(value, f"init[{position}]")
        ref(node["service"], "service")
    else:
        fail(f"{what}.kind {kind!r} is not a v1 node kind")


def node_children(node):
    """Raw edge order (validation reachability), before any sorting."""
    kind = node["kind"]
    if kind == "primitive":
        return []
    if kind in ("opt", "vec"):
        return [node["inner"]]
    if kind in ("record", "variant"):
        return [field["type"] for field in node["fields"]]
    if kind == "func":
        return list(node["args"]) + list(node["results"])
    if kind == "service":
        return [method["function"] for method in node["methods"]]
    return list(node["init"]) + [node["service"]]


def validate_graph(types, declarations, actor):
    node_count = len(types)
    if node_count > BOUNDS["max_type_nodes"]:
        fail("the arena exceeds the verifier node bound")
    if len(declarations) > BOUNDS["max_declarations"]:
        fail("declarations exceed the verifier bound")

    for index, node in enumerate(types):
        validate_node(node, index, node_count)

    seen_names = set()
    for index, declaration in enumerate(declarations):
        what = f"declarations[{index}]"
        check_keys(declaration, ("name", "type"), what)
        name = declaration["name"]
        utf8(name, f"{what}.name")
        if not name:
            fail(f"{what}.name must not be empty")
        if name in seen_names:
            fail(f"{what}.name {name!r} occurs more than once")
        seen_names.add(name)
        check_u32(declaration["type"], f"{what}.type")
        if declaration["type"] >= node_count:
            fail(f"{what}.type dangles outside the arena")
        if types[declaration["type"]]["kind"] == "class":
            fail(f"{what}.type must not target a class node")

    roots = [declaration["type"] for declaration in declarations]
    if actor is not None:
        check_keys(actor, ("kind",), "actor", optional=("service", "class"))
        if actor["kind"] == "service":
            check_keys(actor, ("kind", "service"), "actor")
            target = check_u32(actor["service"], "actor.service")
            if target >= node_count or types[target]["kind"] != "service":
                fail("actor.service must reference a service node")
        elif actor["kind"] == "class":
            check_keys(actor, ("kind", "class"), "actor")
            target = check_u32(actor["class"], "actor.class")
            if target >= node_count or types[target]["kind"] != "class":
                fail("actor.class must reference a class node")
        else:
            fail(f"actor.kind {actor['kind']!r} is not a v1 actor kind")
        roots.append(target)

    actor_class = actor["class"] if actor is not None and actor["kind"] == "class" else None
    for index, node in enumerate(types):
        if node["kind"] == "class" and index != actor_class:
            fail(f"types[{index}] is a class node outside the actor root")
        if node["kind"] == "class":
            if types[node["service"]]["kind"] != "service":
                fail(f"types[{index}].service must reference a service node")
        if node["kind"] == "service":
            for position, method in enumerate(node["methods"]):
                if types[method["function"]]["kind"] != "func":
                    fail(f"types[{index}].methods[{position}].function must reference a func node")
        for child in node_children(node):
            if types[child]["kind"] == "class":
                fail(f"types[{index}] references a class through a type edge")

    if node_count and not roots:
        fail("a non-empty arena requires an actor or at least one declaration root")
    reached = [False] * node_count
    work = list(roots)
    while work:
        reference = work.pop()
        if reached[reference]:
            continue
        reached[reference] = True
        work.extend(node_children(types[reference]))
    for index, was_reached in enumerate(reached):
        if not was_reached:
            fail(f"types[{index}] is unreachable from every actor/declaration root")


# --------------------------------------------------------------------------
# Signatures and partition refinement
# --------------------------------------------------------------------------

def be32(value):
    if value > U32_MAX:
        fail(f"value {value} does not fit the u32 protocol encoding")
    return value.to_bytes(4, "big")


def encoded_string(name):
    encoded = name.encode("utf-8")
    return be32(len(encoded)) + encoded


def sorted_fields(fields):
    return sorted(fields, key=lambda field: (field["id"], field["type"]))


def sorted_methods(methods):
    return sorted(
        methods,
        key=lambda method: (method["id"], method["name"].encode("utf-8"), method["function"]),
    )


def local_signature(node):
    kind = node["kind"]
    if kind == "primitive":
        return bytes([NODE_TAG[kind], PRIMITIVE_TAG[node["primitive"]]])
    if kind in ("opt", "vec"):
        return bytes([NODE_TAG[kind]])
    if kind in ("record", "variant"):
        fields = sorted_fields(node["fields"])
        output = bytes([NODE_TAG[kind]]) + be32(len(fields))
        for field in fields:
            output += be32(field["id"])
        return output
    if kind == "func":
        return (
            bytes([NODE_TAG[kind], MODE_TAG[node["mode"]]])
            + be32(len(node["args"]))
            + be32(len(node["results"]))
        )
    if kind == "service":
        methods = sorted_methods(node["methods"])
        output = bytes([NODE_TAG[kind]]) + be32(len(methods))
        for method in methods:
            output += be32(method["id"]) + encoded_string(method["name"])
        return output
    return bytes([NODE_TAG["class"]]) + be32(len(node["init"]))


def sorted_children(node):
    """Child references in canonical order (record/variant fields and service
    methods sorted; func and class positional)."""
    kind = node["kind"]
    if kind == "primitive":
        return []
    if kind in ("opt", "vec"):
        return [node["inner"]]
    if kind in ("record", "variant"):
        return [field["type"] for field in sorted_fields(node["fields"])]
    if kind == "func":
        return list(node["args"]) + list(node["results"])
    if kind == "service":
        return [method["function"] for method in sorted_methods(node["methods"])]
    return list(node["init"]) + [node["service"]]


def refined_signature(node, own_class, classes):
    output = local_signature(node) + bytes([REFINEMENT_SEPARATOR]) + be32(own_class)
    for child in sorted_children(node):
        output += be32(classes[child])
    return output


def assign_partition_ids(signatures):
    ids = {signature: index for index, signature in enumerate(sorted(set(signatures)))}
    return [ids[signature] for signature in signatures]


def partition_was_split(previous, next_classes):
    first = {}
    for old, new in zip(previous, next_classes):
        if old in first:
            if first[old] != new:
                return True
        else:
            first[old] = new
    return False


def semantic_classes(types):
    classes = assign_partition_ids([local_signature(node) for node in types])
    # Refinement only ever splits classes (each refined signature embeds its
    # own class), so at most len(types) rounds can strictly refine; the +2
    # margin makes an incorrect non-termination loud instead of infinite.
    for _ in range(len(types) + 2):
        refined = assign_partition_ids(
            [
                refined_signature(node, classes[index], classes)
                for index, node in enumerate(types)
            ]
        )
        if not partition_was_split(classes, refined):
            return refined
        classes = refined
    fail("partition refinement did not stabilize")


def remap_node(node, remap):
    kind = node["kind"]
    if kind == "primitive":
        return {"kind": kind, "primitive": node["primitive"]}
    if kind in ("opt", "vec"):
        return {"kind": kind, "inner": remap(node["inner"])}
    if kind in ("record", "variant"):
        return {
            "kind": kind,
            "fields": [
                {"id": field["id"], "type": remap(field["type"])}
                for field in sorted_fields(node["fields"])
            ],
        }
    if kind == "func":
        return {
            "kind": kind,
            "args": [remap(reference) for reference in node["args"]],
            "results": [remap(reference) for reference in node["results"]],
            "mode": node["mode"],
        }
    if kind == "service":
        return {
            "kind": kind,
            "methods": [
                {
                    "name": method["name"],
                    "id": method["id"],
                    "function": remap(method["function"]),
                }
                for method in sorted_methods(node["methods"])
            ],
        }
    return {
        "kind": kind,
        "init": [remap(reference) for reference in node["init"]],
        "service": remap(node["service"]),
    }


def remap_actor(actor, remap):
    if actor is None:
        return None
    if actor["kind"] == "service":
        return {"kind": "service", "service": remap(actor["service"])}
    return {"kind": "class", "class": remap(actor["class"])}


def actor_type_ref(actor):
    return actor["service"] if actor["kind"] == "service" else actor["class"]


def quotient_graph(types, declarations, actor):
    classes = semantic_classes(types)
    class_count = max(classes, default=-1) + 1
    representatives = [None] * class_count
    for index, node_class in enumerate(classes):
        if representatives[node_class] is None:
            representatives[node_class] = index

    remap = lambda reference: classes[reference]
    quotient_types = [remap_node(types[representative], remap) for representative in representatives]
    quotient_declarations = [
        {"name": declaration["name"], "type": remap(declaration["type"])}
        for declaration in declarations
    ]
    return quotient_types, quotient_declarations, remap_actor(actor, remap)


def canonical_reindex(types, declarations, actor):
    old_to_new = [None] * len(types)
    new_to_old = []

    def visit(root):
        stack = [root]
        while stack:
            reference = stack.pop()
            if old_to_new[reference] is not None:
                continue
            old_to_new[reference] = len(new_to_old)
            new_to_old.append(reference)
            stack.extend(reversed(sorted_children(types[reference])))

    if actor is not None:
        visit(actor_type_ref(actor))
    for root in sorted({declaration["type"] for declaration in declarations}):
        visit(root)
    # Defensive: a valid graph has no remaining roots, but the algorithm is
    # specified for any quotient arena.
    for root in range(len(types)):
        if old_to_new[root] is None:
            visit(root)

    remap = lambda reference: old_to_new[reference]
    canonical_types = [remap_node(types[old], remap) for old in new_to_old]
    canonical_declarations = sorted(
        (
            {"name": declaration["name"], "type": remap(declaration["type"])}
            for declaration in declarations
        ),
        key=lambda declaration: (declaration["name"].encode("utf-8"), declaration["type"]),
    )
    return canonical_types, canonical_declarations, remap_actor(actor, remap)


def canonicalize(types, declarations, actor):
    validate_graph(types, declarations, actor)
    return canonical_reindex(*quotient_graph(types, declarations, actor))


# --------------------------------------------------------------------------
# Constrained canonical JSON writer (candid-core-canon-1 profile)
# --------------------------------------------------------------------------

TWO_CHAR_ESCAPES = {
    0x08: b"\\b", 0x09: b"\\t", 0x0A: b"\\n", 0x0C: b"\\f", 0x0D: b"\\r",
    0x22: b'\\"', 0x5C: b"\\\\",
}


def write_json_string(text, output):
    output.append(b'"')
    for character in text:
        point = ord(character)
        if point in TWO_CHAR_ESCAPES:
            output.append(TWO_CHAR_ESCAPES[point])
        elif point < 0x20:
            output.append(b"\\u%04x" % point)
        else:
            output.append(utf8(character, "canonical JSON string content"))
    output.append(b'"')


def write_json_value(value, output):
    if isinstance(value, bool) or value is None or isinstance(value, float):
        fail("booleans, nulls, and floats are outside the identity-payload vocabulary")
    elif isinstance(value, int):
        check_u32(value, "canonical JSON number")
        output.append(str(value).encode("ascii"))
    elif isinstance(value, str):
        write_json_string(value, output)
    elif isinstance(value, list):
        output.append(b"[")
        for index, item in enumerate(value):
            if index:
                output.append(b",")
            write_json_value(item, output)
        output.append(b"]")
    elif isinstance(value, dict):
        output.append(b"{")
        entries = sorted(value.items(), key=lambda entry: utf8(entry[0], "object key"))
        for index, (key, item) in enumerate(entries):
            if index:
                output.append(b",")
            if not all(0x20 <= ord(ch) <= 0x7E for ch in key):
                # The profile's object keys are all fixed ASCII schema names;
                # sorting by UTF-8 bytes and by RFC 8785's UTF-16 code units
                # is identical there, so a non-ASCII key would leave the
                # vocabulary this writer is specified for.
                fail(f"object key {key!r} leaves the fixed ASCII schema-key vocabulary")
            write_json_string(key, output)
            output.append(b":")
            write_json_value(item, output)
        output.append(b"}")
    else:
        fail(f"unsupported canonical JSON value type {type(value).__name__}")


def canonical_json_bytes(value):
    output = []
    write_json_value(value, output)
    return b"".join(output)


# --------------------------------------------------------------------------
# Identity payloads and domain hashing
# --------------------------------------------------------------------------

def contract_payload(types, declarations, actor):
    payload = {
        "format": CONTRACT_FORMAT,
        "format_version": FORMAT_VERSION,
        "semantics_profile": SEMANTICS_PROFILE,
        "canonicalization_profile": CANONICALIZATION_PROFILE,
        "types": types,
        "declarations": declarations,
    }
    if actor is not None:
        # An absent actor is omitted entirely; "actor": null is not a v1
        # spelling of absence, on the wire or in the identity preimage.
        payload["actor"] = actor
    return payload


def actor_reachable_prefix(types, actor):
    reached = [False] * len(types)
    work = [actor_type_ref(actor)]
    while work:
        # Only the reachable *set* matters here, so pop from the end (O(1))
        # rather than the front; traversal order is irrelevant.
        reference = work.pop()
        if reached[reference]:
            continue
        reached[reference] = True
        work.extend(sorted_children(types[reference]))
    prefix_len = 0
    while prefix_len < len(types) and reached[prefix_len]:
        prefix_len += 1
    if any(reached[prefix_len:]):
        fail("actor-reachable nodes must form a canonical arena prefix")
    return types[:prefix_len]


def interface_payload(types, actor):
    return {
        "semantics_profile": SEMANTICS_PROFILE,
        "canonicalization_profile": CANONICALIZATION_PROFILE,
        "types": actor_reachable_prefix(types, actor),
        "actor": actor,
    }


def domain_identity(domain, payload):
    canonical = canonical_json_bytes(payload)
    preimage = domain.encode("utf-8") + b"\x00" + canonical
    digest = hashlib.sha256(preimage).hexdigest()
    return {
        "domain": domain,
        "jcs": canonical.decode("utf-8"),
        "jcs_hex": canonical.hex(),
        "preimage_hex": preimage.hex(),
        "id": f"{domain}:sha256:{digest}",
    }


def identities(types, declarations, actor):
    contract = domain_identity(CONTRACT_DOMAIN, contract_payload(types, declarations, actor))
    interface = None
    if actor is not None:
        interface = domain_identity(INTERFACE_DOMAIN, interface_payload(types, actor))
    return contract, interface


# --------------------------------------------------------------------------
# Vector verification
# --------------------------------------------------------------------------

def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def load_raw_graph(path):
    raw = load_json(path)
    check_keys(raw, ("types",), str(path.name), optional=("declarations", "actor"))
    if not isinstance(raw["types"], list):
        fail(f"{path.name}: types must be an array")
    declarations = raw.get("declarations", [])
    if not isinstance(declarations, list):
        fail(f"{path.name}: declarations must be an array")
    actor = raw.get("actor")
    if "actor" in raw and actor is None:
        fail(f'{path.name}: "actor": null is not a v1 spelling of an absent actor')
    return raw["types"], declarations, actor


class CaseReport:
    def __init__(self, name):
        self.name = name
        self.failures = []

    def expect(self, what, computed, pinned):
        if computed != pinned:
            self.failures.append(
                f"{self.name}: {what}\n    computed: {computed!r}\n    pinned:   {pinned!r}"
            )


def verify_identity_pins(report, label, computed, pinned):
    if pinned is None:
        report.failures.append(f"{report.name}: missing pinned {label} identity")
        return
    for member in ("domain", "jcs", "jcs_hex", "preimage_hex", "id"):
        report.expect(f"{label} {member}", computed[member], pinned.get(member))


def verify_case(case, base):
    report = CaseReport(case["name"])
    if not case["inputs"]:
        raise VectorError("case has no raw inputs")
    expected = load_json(base / case["expected"])
    check_keys(
        expected,
        ("canonical", "contract_identity"),
        case["expected"],
        optional=("interface_identity",),
    )
    canonical_pin = expected["canonical"]
    check_keys(canonical_pin, ("types", "declarations"), "canonical", optional=("actor",))

    raw_documents = [load_json(base / input_name) for input_name in case["inputs"]]
    for index, left in enumerate(raw_documents):
        for right_name, right in zip(case["inputs"][index + 1 :], raw_documents[index + 1 :]):
            if left == right:
                report.failures.append(
                    f"{report.name}: raw inputs {case['inputs'][index]} and {right_name} "
                    "are byte-copies; multi-input cases must keep genuinely distinct arenas"
                )

    results = []
    for input_name in case["inputs"]:
        types, declarations, actor = load_raw_graph(base / input_name)
        canonical = canonicalize(types, declarations, actor)
        contract, interface = identities(*canonical)
        results.append((input_name, canonical, contract, interface))

    first = results[0]
    for input_name, canonical, contract, interface in results[1:]:
        if (canonical, contract, interface) != (first[1], first[2], first[3]):
            report.failures.append(
                f"{report.name}: raw input {input_name} does not converge with {first[0]}"
            )

    _, (types, declarations, actor), contract, interface = first
    report.expect("canonical types", types, canonical_pin["types"])
    report.expect("canonical declarations", declarations, canonical_pin["declarations"])
    report.expect("canonical actor", actor, canonical_pin.get("actor"))
    verify_identity_pins(report, "contract", contract, expected["contract_identity"])
    if actor is None:
        if interface is not None or "interface_identity" in expected:
            report.failures.append(f"{report.name}: actorless vectors must pin no interface identity")
    else:
        verify_identity_pins(report, "interface", interface, expected.get("interface_identity"))

    if "wire" in case:
        wire = load_json(base / case["wire"])
        report.expect("wire format", wire.get("format"), CONTRACT_FORMAT)
        report.expect("wire format_version", wire.get("format_version"), FORMAT_VERSION)
        report.expect("wire semantics_profile", wire.get("semantics_profile"), SEMANTICS_PROFILE)
        report.expect(
            "wire canonicalization_profile",
            wire.get("canonicalization_profile"),
            CANONICALIZATION_PROFILE,
        )
        report.expect("wire canonical types", wire.get("types"), types)
        report.expect("wire canonical declarations", wire.get("declarations"), declarations)
        report.expect("wire canonical actor", wire.get("actor"), actor)
        if actor is None and "actor" in wire:
            report.failures.append(
                f"{report.name}: an actorless wire fixture must omit the actor property"
            )
        wire_identities = wire.get("identities", {})
        report.expect("wire contract ID", wire_identities.get("contract"), contract["id"])
        expected_interface = None if interface is None else interface["id"]
        report.expect("wire interface ID", wire_identities.get("interface"), expected_interface)

    if "identity_pins" in case:
        pins = load_json(base / case["identity_pins"])
        for member, computed in (
            ("domain", contract["domain"]),
            ("jcs", contract["jcs"]),
            ("jcs_hex", contract["jcs_hex"]),
            ("preimage_hex", contract["preimage_hex"]),
            ("contract_id", contract["id"]),
        ):
            report.expect(f"identity pin {member}", computed, pins.get(member))

    return report, contract["id"]


def main():
    manifest = load_json(HERE / "manifest.json")
    check_keys(manifest, ("format", "version", "required_cases", "cases"), "manifest.json")

    failures = []
    if manifest["format"] != "candid-core-conformance-manifest" or manifest["version"] != 1:
        failures.append("manifest.json does not declare the v1 conformance-manifest format")
    if tuple(manifest["required_cases"]) != REQUIRED_CASES:
        failures.append(
            "manifest.json required_cases drifted from the required scenario set:\n"
            f"    manifest: {manifest['required_cases']}\n"
            f"    required: {list(REQUIRED_CASES)}"
        )

    case_names = []
    for case in manifest["cases"]:
        check_keys(
            case,
            ("name", "description", "inputs", "expected"),
            "manifest case",
            optional=("did", "wire", "identity_pins"),
        )
        case_names.append(case["name"])
    if len(set(case_names)) != len(case_names):
        failures.append("manifest.json case names must be unique")
    missing = [name for name in REQUIRED_CASES if name not in case_names]
    if missing:
        failures.append(f"manifest.json is missing required cases: {missing}")
    for case in manifest["cases"]:
        if not case["inputs"]:
            failures.append(f"case {case['name']} has no raw inputs")
        if case["name"] in CONVERGENCE_CASES and len(case["inputs"]) < 2:
            failures.append(
                f"case {case['name']} must keep multiple noncanonical raw inputs"
            )

    verified = []
    for case in manifest["cases"]:
        try:
            report, contract_id = verify_case(case, HERE)
        except VectorError as error:
            failures.append(f"{case['name']}: {error}")
        else:
            failures.extend(report.failures)
            if not report.failures:
                verified.append((case["name"], contract_id))

    if failures:
        print("conformance vector verification FAILED:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    for name, contract_id in verified:
        print(f"{name}: independently reproduced {contract_id}")
    print(f"all {len(verified)} conformance vectors verified independently")
    return 0


if __name__ == "__main__":
    sys.exit(main())
