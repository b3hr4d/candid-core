# Canonicalization v1: `candid-core-canon-1`

This document is the **normative, language-independent specification** of the
`candid-core-canon-1` canonicalization profile declared by ADR
[0002](adrs/0002-versioning-and-canonical-bytes.md). Together with the golden
fixtures under `tests/fixtures/conformance/`, it defines every byte of the
`candid-core:contract:v1` and `candid-core:interface:v1` identity preimages.
The Rust crate is a reference implementation of this specification, not the
specification itself.

The key words MUST, MUST NOT, SHOULD, and MAY are to be interpreted as in
RFC 2119. The algorithm is specified in executable style: a conforming
implementation follows the numbered steps literally and reproduces every
conformance vector byte for byte. This profile is frozen: any observable
change to the bytes defined here requires a new canonicalization profile
name, never an edit to this one.

Contents:

1. [Input and preconditions](#1-input-and-preconditions)
2. [Notation and primitive encodings](#2-notation-and-primitive-encodings)
3. [Tag tables](#3-tag-tables)
4. [Ordering keys](#4-ordering-keys)
5. [Node signatures](#5-node-signatures)
6. [Semantic partition refinement](#6-semantic-partition-refinement)
7. [Quotient graph construction](#7-quotient-graph-construction)
8. [Deterministic re-indexing](#8-deterministic-re-indexing)
9. [Identity payloads](#9-identity-payloads)
10. [Constrained canonical JSON](#10-constrained-canonical-json)
11. [Domain-separated hashing](#11-domain-separated-hashing)
12. [Resource policy is not identity](#12-resource-policy-is-not-identity)
13. [Conformance](#13-conformance)

## 1. Input and preconditions

The input is one Contract graph in the v1 vocabulary of the
[Contract graph specification](contract-graph.md):

- `format`: exactly `"candid-core"`.
- `format_version`: exactly `1`.
- `semantics_profile`: exactly `"candid-1"`.
- `canonicalization_profile`: exactly `"candid-core-canon-1"`.
- `types`: an arena of type nodes. A type reference (`TypeRef`) is an
  unsigned 32-bit integer (`u32`) index into this arena.
- `declarations`: an array of `{ name, type }` named roots.
- `actor`: absent, or `{ "kind": "service", "service": TypeRef }`, or
  `{ "kind": "class", "class": TypeRef }`. Absence is the only spelling of
  "no actor"; an explicit `"actor": null` MUST be rejected at decode.

Canonicalization is specified only for **valid** input. An implementation
MUST establish the following preconditions (rejecting the Contract
otherwise) before running the algorithm; the checklist matches the
[Contract graph validation rules](contract-graph.md#validation-checklist):

1. Every `TypeRef` (node edges, declaration targets, actor target) is in
   range for the arena, and constrained references target the required node
   kind: a service method's `function` targets a `func` node, a class's
   `service` targets a `service` node, a service actor targets a `service`
   node, a class actor targets a `class` node.
2. Field IDs and method IDs are `u32` values. Within one record or variant,
   field IDs are unique. Within one service, method names are non-empty and
   unique, and every method's `id` equals Candid's `idl_hash` of its UTF-8
   name bytes
   (`h := 0; for byte in utf8(name): h := (h * 223 + byte) mod 2^32`).
   Distinct method names MAY share one 32-bit hash.
3. A `func` node with mode `oneway` has no results.
4. `class` nodes appear only as the top-level class actor root: never as a
   declaration target and never through a type edge.
5. Declaration names are non-empty and unique.
6. Every node is reachable from an actor or declaration root, and a
   non-empty arena has at least one such root.
7. Every string (declaration names, method names) is a well-formed sequence
   of Unicode scalar values. Lone surrogates and other invalid Unicode MUST
   be rejected. Strings are **never normalized**: canonically equivalent but
   byte-different spellings (for example NFC `é` = `C3 A9` versus NFD
   `e`+U+0301 = `65 CC 81`) are distinct values and stay exactly as
   presented, at every stage of this algorithm.

The output is the canonical Contract: a re-indexed arena, re-ordered
collections, and the two identity strings. Canonicalization is idempotent —
running the algorithm on its own output reproduces it — and invariant under
input-arena permutation, duplicate semantically equivalent nodes, and
declaration/collection listing order. `identities` and `producer` are
carried alongside the graph but are **inputs to nothing** in this document
except §9's exclusion rules.

## 2. Notation and primitive encodings

- `u32be(n)`: the value `n` as exactly 4 bytes, big-endian. Every count,
  length, field ID, method ID, class index, and type reference in a
  signature is encoded this way.
- `utf8(s)`: the UTF-8 encoding of string `s`.
- `string(s)`: `u32be(byte-length of utf8(s)) ++ utf8(s)`. The length is the
  **UTF-8 byte length**, not a character, scalar, or UTF-16 code-unit count
  (`string("é") = 00 00 00 02 C3 A9`).
- `++`: byte-string concatenation.

Any value that does not fit in a `u32` — an arena of 2³² or more nodes, a
collection of 2³² or more entries, a string of 2³² or more bytes — is
**outside the protocol**: an implementation MUST fail the operation rather
than truncate, wrap, or substitute a host-native (`usize`-style) encoding.
Valid Contracts decoded from the v1 wire vocabulary cannot reach these
limits, but the failure behavior is normative so that no implementation
silently produces different bytes at the boundary.

## 3. Tag tables

These single-byte tags are protocol constants of this profile.

**Node tags** (8 kinds):

| kind | tag |
| --- | --- |
| `primitive` | 0 |
| `opt` | 1 |
| `vec` | 2 |
| `record` | 3 |
| `variant` | 4 |
| `func` | 5 |
| `service` | 6 |
| `class` | 7 |

**Primitive tags** (18 primitives):

| primitive | tag | primitive | tag | primitive | tag |
| --- | --- | --- | --- | --- | --- |
| `null` | 0 | `nat32` | 6 | `float32` | 12 |
| `bool` | 1 | `nat64` | 7 | `float64` | 13 |
| `nat` | 2 | `int8` | 8 | `text` | 14 |
| `int` | 3 | `int16` | 9 | `reserved` | 15 |
| `nat8` | 4 | `int32` | 10 | `empty` | 16 |
| `nat16` | 5 | `int64` | 11 | `principal` | 17 |

**Method-mode tags** (4 modes):

| mode | tag |
| --- | --- |
| `update` | 0 |
| `query` | 1 |
| `composite_query` | 2 |
| `oneway` | 3 |

## 4. Ordering keys

All comparisons in this profile are total orders built from unsigned
lexicographic byte comparison and unsigned integer comparison.

- **String order**: strings compare as their UTF-8 byte sequences, unsigned
  byte by byte, shorter-prefix first. For well-formed strings this is
  identical to Unicode scalar (code point) order. It is **not** UTF-16
  code-unit order: U+FF61 (`EF BD A1`) sorts before U+10000 (`F0 90 80
  80`) here, while a UTF-16 comparison would put the surrogate pair
  `D800 DC00` first. Dynamic Unicode is ordered only by this rule (§4
  field/method/declaration keys); canonical-JSON object keys are a disjoint,
  fixed ASCII set (§10).
- **Field order** (record and variant fields): by `id` ascending; ties
  broken by the field's type reference ascending. Valid input has unique
  field IDs per aggregate, so the tie-breaker is unreachable there; it is
  specified so the sort is total on any input an implementation processes.
- **Method order** (service methods): by `id` ascending, then by name in
  string order, then by the method's function reference ascending. Method
  IDs MAY collide (distinct names, one `idl_hash` value); the name breaks
  that tie. Valid input has unique names per service, so the function-
  reference tie-breaker is unreachable there.
- **Declaration output order**: by name in string order, then by the
  remapped type reference ascending. Valid input has unique names, so the
  reference tie-breaker is unreachable there.
- **Reference order**: plain unsigned integer order on `TypeRef` values.

Whenever a later section says "sorted fields", "sorted methods", or "sorted
declarations", it means these keys. Sorting MUST be applied wherever stated
— signatures, child enumeration, node rewriting, and output — never once
globally and cached across reference remappings, because the tie-breakers
reference the *current* graph's references at each stage.

**Sorted children** of a node, used by refinement (§6), traversal (§8), and
reachability (§9.2):

| kind | children, in order |
| --- | --- |
| `primitive` | none |
| `opt`, `vec` | `inner` |
| `record`, `variant` | each field's `type`, fields in field order |
| `func` | every `args` entry in listed order, then every `results` entry in listed order |
| `service` | each method's `function`, methods in method order |
| `class` | every `init` entry in listed order, then `service` |

`func` argument/result lists and `class` init lists are positional: their
listed order is semantic and is never sorted.

## 5. Node signatures

The **local signature** `L(n)` of a node `n` is:

| kind | bytes |
| --- | --- |
| `primitive p` | `00 ++ primitive-tag(p)` |
| `opt` | `01` |
| `vec` | `02` |
| `record f…` | `03 ++ u32be(count(f)) ++ u32be(id)` for each field in field order |
| `variant f…` | `04 ++ u32be(count(f)) ++ u32be(id)` for each field in field order |
| `func` | `05 ++ mode-tag ++ u32be(count(args)) ++ u32be(count(results))` |
| `service m…` | `06 ++ u32be(count(m)) ++ (u32be(id) ++ string(name))` for each method in method order |
| `class` | `07 ++ u32be(count(init))` |

Local signatures deliberately contain no type references: they describe a
node's own shape only. Child structure enters through refinement.

The **refined signature** `R(n)` of node `n` under a class assignment
`class[·]` is:

```text
R(n) = L(n) ++ FF ++ u32be(class[n]) ++ u32be(class[c])  for each c in sorted-children(n)
```

The single byte `FF` is the refinement separator. Including the node's own
current class means refinement can only ever split classes, never merge
them (§6). Child classes follow in sorted-children order.

## 6. Semantic partition refinement

Candid type definitions are equi-recursive: aliases and duplicated
structurally identical definitions do not create new semantic wire types.
This step partitions the arena into classes of bisimilar nodes — the
greatest labelled bisimulation of the finite graph — with deterministic
class numbering.

**Class assignment.** Given one signature byte-string per node, classes are
assigned as follows: collect the *distinct* signatures, sort them in
lexicographic (unsigned bytewise) order, and number them `0, 1, 2, …` in
that order. Each node's class is its signature's number. Class IDs are
therefore dense, start at zero, and depend only on the multiset of
signature byte-strings — never on arena positions.

**Algorithm.**

1. `class ← assign(L(n) for every node n)` — initial classes from local
   signatures.
2. Loop:
   1. `next ← assign(R(n) under class, for every node n)`.
   2. If `next` **splits** `class` — that is, if any two nodes with equal
      `class` values have different `next` values — set `class ← next` and
      repeat.
   3. Otherwise stop. **The result is `next`, the labels computed in this
      final round** — not the previous `class` labels. The two partition the
      nodes identically, but the final numbering is the one induced by the
      last round's refined-signature byte order, and that numbering is
      normative: it becomes the quotient graph's reference space (§7).
3. Because each refined signature embeds the node's own current class,
   distinct classes can never receive equal refined signatures, so classes
   only split and never merge; the loop reaches a fixpoint after at most
   `node-count` iterations. An implementation MAY bound the loop at
   `node-count + 1` rounds and MUST treat exceeding that bound as an
   internal error, never as an answer.

An empty arena yields zero classes.

## 7. Quotient graph construction

The quotient graph has exactly one node per refinement class, indexed by
class ID.

1. **Representative.** For each class, the representative is the member
   with the smallest input arena index. For valid input the choice is
   provably irrelevant: bisimilar nodes have identical local signatures,
   and their sorted collections agree position by position on every ordering
   key with children in the same classes, so step 2 produces identical
   bytes from any member. The smallest index is nevertheless normative so
   that even out-of-contract inputs produce deterministic results.
2. **Quotient node.** The quotient node for class `k` is representative
   `r`'s content with every reference `t` replaced by `class[t]`, and with
   record/variant fields and service methods re-sorted by their ordering
   keys (§4) as part of the rewrite. The sort happens **before** the
   replacement: the reference tie-breakers compare the representative's
   original input-arena references, not the substituted class IDs. (`func`
   and `class` reference lists stay positional.)
3. **Roots.** Every declaration target `t` becomes `class[t]`; the actor
   target likewise. Declaration names and the actor kind are unchanged.

After this step, semantically duplicate input nodes have collapsed into one
node each, and every reference is a class ID.

## 8. Deterministic re-indexing

The quotient's class-ID reference space is deterministic but not yet the
canonical arena order. This step renumbers nodes by traversal discovery
order.

**Root sequence.** Traversal starts from, in order:

1. The actor's target class, if an actor is present — always first.
2. The declaration target classes, **sorted as plain integers (quotient
   class IDs), ascending, with duplicates removed** — explicitly *not* in
   declaration-name order. Declaration *names* order the output
   declarations array below; they play no role in node traversal. The two
   orders differ observably: a Contract whose lexicographically first
   declaration name targets a higher class ID traverses the lower class ID
   first. The `declaration_root_order` conformance vector pins this
   distinction — its declaration targets are outside the actor-reachable
   set, and traversing them in name order produces a different canonical
   arena and different IDs than its pinned values.
3. Defensively, every remaining class ID in ascending order. Valid input
   has no remaining classes (every node is reachable from the roots above),
   but the algorithm is specified totally so that all implementations agree
   even on graphs that bypass validation.

**Traversal.** For each root in sequence, run an **iterative depth-first
preorder** walk over quotient references:

```text
visit(root):
  stack ← [root]
  while stack is non-empty:
    q ← pop(stack)                       # last-in, first-out
    if q is already numbered: continue
    number[q] ← next canonical index     # 0, 1, 2, … across all roots
    push sorted-children(q) onto stack in REVERSE order
```

Pushing the sorted children reversed means the *first* child in sorted-
children order is popped — and therefore numbered — next: preorder,
children in sorted order, cycles cut at already-numbered nodes. Recursive
implementation is equivalent but NOT required; the explicit stack form is
the reference formulation precisely so deep graphs need not recurse.

**Rewrite.** The canonical arena lists nodes by canonical index. Each node
is the quotient node's content with every reference `q` replaced by
`number[q]`, and with record/variant fields and service methods re-sorted
by their §4 keys during the rewrite (the field/method tie-breakers compare
*quotient* references at this point, exactly as written; for valid input
the primary keys already decide).

**Outputs.**

- `types`: the rewritten canonical arena.
- `declarations`: every input declaration `{ name, number[class[target]] }`,
  sorted by the declaration output order (§4) — name in string order, then
  remapped reference.
- `actor`: unchanged in kind, target replaced by its canonical index.

The canonical Contract carries the same `format`, `format_version`,
`semantics_profile`, and `canonicalization_profile` values it was given
(§1 fixed them already) and the same `producer` verbatim.

## 9. Identity payloads

Both identities hash a JSON payload rendered by §10. The payload member
sets are exact: nothing may be added, renamed, or reordered semantically
(the writer orders keys itself), and absent members are omitted entirely.

### 9.1 Contract identity payload

Exactly these members, from the canonical Contract:

| member | value |
| --- | --- |
| `format` | `"candid-core"` |
| `format_version` | `1` |
| `semantics_profile` | `"candid-1"` |
| `canonicalization_profile` | `"candid-core-canon-1"` |
| `types` | the complete canonical arena |
| `declarations` | the canonical declarations array |
| `actor` | the canonical actor object — **omitted entirely when the Contract has no actor**; `"actor": null` never appears in a payload |

`identities` (the values being derived) and `producer` are **excluded**.
Producer metadata is untrusted provenance deliberately outside
authenticated identity: two Contracts differing only in `producer` share
both IDs even though their wire JSON differs. Binding it would change every
existing `contract_id`; this exclusion is load-bearing compatibility.

### 9.2 Interface identity payload

Present exactly when the Contract has an actor. Exactly these members:

| member | value |
| --- | --- |
| `semantics_profile` | `"candid-1"` |
| `canonicalization_profile` | `"candid-core-canon-1"` |
| `types` | the **actor-reachable prefix** of the canonical arena |
| `actor` | the canonical actor object |

`format`, `format_version`, and `declarations` are excluded: the interface
identity claims wire-interface equality only, independent of declaration
names and declaration-only types.

**Actor-reachable prefix.** Because §8 traverses the actor first, every
actor-reachable node occupies a contiguous prefix `types[0 .. k)` of the
canonical arena, where `k` is the number of nodes reachable from the actor
target over sorted-children edges (including the target itself). An
implementation MUST NOT emit an interface payload that omits an
actor-reachable node or includes an actor-unreachable one. An
implementation whose §8 traversal establishes the prefix invariant by
construction MAY rely on that construction (the Rust reference additionally
asserts it in debug builds); one that computes the prefix independently
MUST verify that no reachable node lies outside it and treat a violation as
an internal error, never as an acceptable output. The prefix is a *strict*
subset whenever declaration-only types exist; the `declaration_root_order`
vector pins this truncation, so an implementation that hashes the full
arena for the interface identity cannot reproduce its pinned values.

An actorless Contract has no interface identity at all.

## 10. Constrained canonical JSON

Identity payloads are rendered with the following writer. Within this
profile's payload vocabulary the output is byte-identical to RFC 8785
(JCS); the deviations from full JCS are stated at the end of this section
and are deliberately unobservable here.

**Grammar.** A payload value is one of: an object, an array, a string, or
an unsigned integer that fits in `u32`. Booleans, `null`, floating-point
values, negative values, and integers ≥ 2³² do not occur in the vocabulary,
and a conforming implementation MUST NOT emit them in an identity payload.
An implementation whose payload construction is statically typed to this
vocabulary (as the Rust reference's payload structs are) satisfies that by
construction; a writer that can be handed arbitrary JSON values (as the
Python reference's can) MUST instead reject out-of-vocabulary values at
run time.

**Rules.**

1. **No whitespace.** No spaces, newlines, or indentation anywhere.
   Separators are exactly `{`, `}`, `[`, `]`, `:`, `,`.
2. **Objects.** Members are written sorted by key, comparing keys as UTF-8
   bytes. Every key in the vocabulary is one of the fixed ASCII schema
   names (`actor`, `args`, `canonicalization_profile`, `class`,
   `declarations`, `fields`, `format`, `format_version`, `function`, `id`,
   `init`, `inner`, `kind`, `methods`, `mode`, `name`, `primitive`,
   `results`, `semantics_profile`, `service`, `type`, `types`), for which
   UTF-8 byte order and UTF-16 code-unit order coincide.
3. **Arrays.** Elements are written in the order given by §8/§9. Array
   order is semantic; the writer never sorts arrays.
4. **Numbers.** A `u32` value is written as its shortest decimal form: no
   sign, no leading zeros, no exponent, no fraction (`0`, `1`, …,
   `4294967295`). This coincides with RFC 8785's ES6 serialization for
   every integer below 2⁵³.
5. **Strings.** Output is UTF-8. Each Unicode scalar of the value is
   written as:
   - `\"` for U+0022, `\\` for U+005C;
   - `\b` U+0008, `\t` U+0009, `\n` U+000A, `\f` U+000C, `\r` U+000D;
   - the six-character sequence backslash, `u`, `0`, `0`, and two
     **lowercase** hexadecimal digits for the remaining control characters
     U+0000–U+001F (U+0001 becomes backslash + `u0001`; U+001F becomes
     backslash + `u001f`);
   - the scalar's literal UTF-8 bytes for everything else — including
     U+007F, all non-ASCII BMP scalars, and supplementary-plane scalars.
     No `\uXXXX` escaping of printable characters, ever.
   This is exactly RFC 8785 §3.2.2.2 string serialization.
6. **Unicode hygiene.** Input strings MUST already be well-formed scalar
   sequences (§1); a writer encountering an unpaired surrogate or invalid
   sequence MUST fail, and MUST NOT normalize, replace (U+FFFD), or drop
   anything.

**Relationship to RFC 8785, precisely.** This profile does **not** claim a
general-purpose JCS implementation, and a conforming implementation MUST
NOT substitute one without checking the vocabulary constraints, because two
JCS behaviors are deliberately out of scope:

- *Property-name ordering.* RFC 8785 sorts object members by UTF-16 code
  units. This writer sorts by UTF-8 bytes. The two orders differ only when
  a key contains both a supplementary-plane scalar and a scalar in
  U+E000–U+FFFF — impossible here, because every payload key is a fixed
  ASCII schema name. All dynamic Unicode lives in string *values*
  (declaration names, method names), which JCS never reorders; their
  ordering *within arrays* is graph canonicalization (§4/§8), not JSON
  serialization.
- *Number serialization.* RFC 8785 serializes numbers through IEEE-754
  double-precision ES6 rules. This vocabulary contains only `u32` integers,
  whose shortest-decimal form is identical under both definitions;
  arbitrary doubles are simply not part of the format.

A general JCS library that is byte-correct on this vocabulary MAY be used;
the conformance vectors (§13) are the arbiter.

## 11. Domain-separated hashing

For a payload `P` and its domain string `D`:

```text
preimage = utf8(D) ++ 00 ++ canonical-json-bytes(P)
digest   = SHA-256(preimage)
identity = D ++ ":sha256:" ++ lowercase-hex(digest)
```

- Contract identity domain: `candid-core:contract:v1`.
- Interface identity domain: `candid-core:interface:v1`.
- The separator is exactly one `0x00` byte between the UTF-8 domain bytes
  and the canonical JSON bytes.
- The digest is rendered as 64 lowercase hexadecimal characters; uppercase
  spellings are invalid.

Example (the checked-in actorless vector): the canonical payload bytes and
full preimage are pinned in
`tests/fixtures/conformance/actorless.identity.json`, and hash to
`candid-core:contract:v1:sha256:d43274872cdb6c503456065d12c26b512ba9e3eac5b0a9533c8f9716293c6e18`.

## 12. Resource policy is not identity

Implementations SHOULD bound the work canonicalization performs on
untrusted input (the Rust reference meters node, edge, byte, sort, and
serialization work against `Limits`). Such policies are **non-normative
implementation behavior**: limit values, work-accounting formulas, and
which limit fires first are never part of this profile and MUST NOT
influence a single canonical byte. Two conforming implementations with
different resource policies differ only in *which inputs they refuse*,
never in the canonical bytes or IDs of an input both accept. The protocol
itself constrains sizes only through §2: `u32` references, counts, and
lengths.

## 13. Conformance

The fixture manifest `tests/fixtures/conformance/manifest.json` pins the
required scenario set — actorless, empty actor, class actor, basic service,
direct recursion, mutual recursion, `idl_hash` collision (including a
method whose alphabetical and id orders diverge, pinning `id` as the
primary method sort key), Unicode (scalar-order versus UTF-16,
non-normalization, string escaping), duplicate semantic nodes, arena
permutations, and declaration-root traversal order (with a strict
actor-reachable interface prefix) — with raw inputs and expected canonical
graphs, canonical JSON text and hex, domain preimages, and IDs for each.
The raw inputs mix deliberately noncanonical arrangements with
already-canonical forms, which pin idempotence. Scenarios with multiple raw
inputs MUST all converge to the identical canonical result. The "hash
collision" scenario is a genuine Candid `idl_hash` 32-bit collision between
two method names; refinement signatures themselves are exact bytes and are
never hashed, so no SHA-256-collision claim is made anywhere in this
profile.

Two independent implementations consume the manifest:

- the Rust test suite `tests/conformance_vectors.rs` (with supporting
  exact-fixture coverage in `tests/adr_conformance.rs` and ordering,
  normalization, and permutation properties in
  `tests/canonical_properties.rs`), and
- the standard-library Python reference canonicalizer,
  `python3 tests/fixtures/conformance/verify_vectors.py`, which recomputes
  every canonical graph, payload byte, preimage, and ID from the raw
  vectors without touching the Rust implementation. CI runs it as a
  dedicated job (`conformance-reference` in
  `.github/workflows/msrv.yml`).

An implementation conforms to `candid-core-canon-1` if and only if it
reproduces every pinned value of every manifest vector.
