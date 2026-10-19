# Changelog

This file records what changed between released versions of `candid-core`. The
release procedure that produces an entry here is [docs/releasing.md](docs/releasing.md).

`candid-core` is pre-1.0. Until 1.0, **any** release may change the public Rust
API, the serialized Contract/Compilation/envelope shapes, the canonical bytes,
and therefore the identities computed over them. Pin an exact version.

## Unreleased

### Resource bounds

- **The type-depth walks allocate the same way for the same input**
  ([issue #263]). Both walks that enforce `max_type_depth` memoized their
  states in an ordered set keyed by each type node's memory address, so how
  the set's nodes split, and therefore what a compile allocated, followed the
  heap layout: the same `.did` allocated a few more or fewer times depending
  on the process's arguments and working directory, and an unrelated change
  moved the counts. The memo now keys on a dense id each node receives on its
  first visit. A walk's visit order is fixed by its input, so its allocations
  are too. What a walk refuses and returns is unchanged. The id map holds one
  entry per distinct node, which moves the measured exchange rate of
  `max_type_preflight_work` from about 19 to about **21 bytes of peak live
  heap per unit** (20.8–21.5 across bundle sizes, release profile); its
  documented guidance moves with it, to roughly 210 MB at the default and
  ~21 MB at 1 000 000.

### Verification

- **Every compile stage after parsing is held to linear allocation** ([issue #256]).
  `tests/stage_scaling_cost.rs` runs each stage at 500 and 1,000 items, in
  two shapes (many one-record declarations used by as many service methods;
  one record and one variant each 1,000 wide), and fails if doubling the
  input more than triples the bytes a counting allocator sees. The stages are
  compiling without source info, canonicalization, structural validation,
  `Contract` and `ContractEnvelope` loading and writing, `SourceInfo`
  validation, and `Compilation` loading, which recompiles the embedded bundle.
  Each measures 2.00–2.03 today. Each test was shown to fail on an injected
  per-item clone of a growing collection. The standing rule from issue #39 is
  narrowed to say so: no *timing* measurement fails a workflow, while a
  deterministic allocation test that asserts a ratio is a correctness test.
  The suite bounds what a stage allocates, not the work it does: a stage that
  turned quadratic without allocating more, such as a nested scan, would pass
  it. Nothing in the published crate changed.

## 0.1.0 — published 2026-10-12

The first version without a prerelease suffix. It is still pre-1.0: any later
release may change the public Rust API, the serialized
Contract/Compilation/envelope shapes, the canonical bytes, and the identities
computed over them, so pin `=0.1.0` rather than take a caret requirement. The
Contract format is **not** promoted to a stable v1 by this release. Released
from commit `06aceae6b157de10add95c40d486a8768bf4bb0b`. The `.crate` SHA-256
crates.io recorded is
`95856ecec9c69d668eb0367085084c789d5427c78b00cafc176a568de2f7f91d`, matching
the digest the release gates measured, the one the `guard` job reproduced by
repackaging the commit independently, and the one the `confirm` job read back
from crates.io after publication.

This version follows the issue #81 bump protocol:
`ProducerInfo::current().version` (and therefore the octets of newly
serialized documents) is the only observable change from the bump itself, and
`tests/release_metadata.rs` asserts it together with the identities that must
not move. The frozen exact-octet artifact vectors were written before the first
beta with the placeholder producer version `0.1.0`, which this release now
shares; they are not regenerated, and their pinned artifact IDs, not the
version they carry, are what proves it.

**Upgrading from `0.1.0-beta.3`.** The public API is unchanged: both
inventories, the base and the all-features surface, are identical, and no
change below is acknowledged as breaking. Every change makes the crate refuse
something it used to crash on, stall on, or mis-read, so a consumer that
compiles or loads only well-formed input sees no difference beyond the producer
version. Four inputs that `0.1.0-beta.3` accepted are now refused:

- a record field whose id is `u32::MAX`, which panicked in a debug build and
  numbered the next unnamed field 0 in a release build ([issue #230]);
- a type on a cycle that passes only through `opt` ([issue #234]);
- a run of consecutive comments longer than `max_source_nesting`, which
  aborted the process from the stack ([issue #219]);
- a Contract document that writes a struct as a JSON array or a unit variant
  as a map ([issue #235], [issue #238]), forms candid-core never writes and
  `@candid-core/schema` already refused.

Every Contract, envelope and Compilation document candid-core has written,
`0.1.0-beta.3`'s included, still loads, with the same identities.

### Source input

- **A `.did` source may begin with one UTF-8 byte order mark** ([issue #176],
  [PR #177]). Windows editors and PowerShell 5.1 write `EF BB BF` before UTF-8
  text, and every entry point refused such a file with `did_parse_error`
  "Unknown token … at bytes 0..3", quoting a character the reader cannot see.
  The mark is not Candid syntax, so the compiler now skips exactly one leading
  U+FEFF immediately before tokenizing, and nowhere else. Every entry point
  shares that parse helper: `compile_did`, `compile_did_with_options`,
  `compile_did_with_context`, `compile_with_resolver` (the entry source and
  every imported one), `compile_did_file`, `compile_did_file_with_options`,
  `compile_did_file_with_context`, the `candid-core` binary's `compile`,
  `--envelope` included, and the `SourceInfo` rederivation behind
  `SourceInfo::validate` and the `Compilation` loaders (`from_json_with_limits`,
  `from_slice_with_limits`, `try_from_raw`, and their `_with_context` forms);
  `@candid-core/cli`'s `didToContract` and `didToModule`, which embed this
  source, inherit it. In this one place `candid-core` accepts a superset of
  what upstream `candid_parser` accepts.

  Everything that describes the raw source keeps the mark. A marked file and
  its unmarked twin share `contract_id` and `interface_id`, and differ in
  `source_bundle_id`, which hashes the raw bytes, mark included (it is a
  raw-source bundle identity, ADR 0001). The `SourceInfo` source text keeps
  the mark, resolver digests cover it, and the byte limits count it, so
  `max_source_bytes` sees three more bytes than the twin. Parse diagnostics
  still index the file as stored: their spans and "at bytes" text count the
  mark. A second mark, a mark after any other byte, and U+FFFE stay unknown
  tokens, and the UTF-16 marks stay invalid UTF-8.

  Nothing `0.1.0-beta.3` accepts changes its output or any identity, because a
  marked source failed there. The reverse does not hold: a Compilation document
  whose `SourceInfo` carries a marked source, such as `candid-core compile`
  output for a marked file, validates with this change and does not validate
  under `0.1.0-beta.3`, whose rederivation fails on the mark. Error precedence
  changes for marked input only: the `source_nesting` preflight now tokenizes
  past the mark too, so an over-nested marked source reports
  `resource_limit_exceeded` naming `source_nesting` where `0.1.0-beta.3`
  reported `did_parse_error` on the mark. No public API, error code, or
  serialized shape changed, so this entry acknowledges no break. ADR 0004
  records the rule, and `tests/source_bom.rs` pins each case above.

- **A record field whose id is `u32::MAX` is refused instead of panicking or
  misnumbering** ([issue #230]). The pinned upstream `candid_parser` record
  rule computes the next positional id as `id + 1` at every record field,
  labelled or not. At 4294967295 that addition panicked in a debug build and
  wrapped to 0 in a release build, where the next unnamed field silently got
  id 0: `record { 4294967295 : nat; text }` compiled to ids `[0, 4294967295]`.
  A name reaches the same id through its Candid hash (`hlrnuwa` does), and so
  does an unnamed field after `4294967294`.

  Before the upstream grammar runs, a token preflight now refuses every record
  field whose id would be `u32::MAX` (written as a decimal or hex number, as a
  name or string literal hashing to it, or reached by an unnamed field) with
  `did_parse_error`, "record field id 4294967295 is not supported at bytes
  …", and the field's exact span, in every build and on every entry point:
  `compile_did` and its variants, `compile_with_resolver` (naming the
  importing source), `compile_did_file` and its variants, the `candid-core`
  binary, and the `SourceInfo` rederivation behind `SourceInfo::validate` and
  the `Compilation` loaders. The message locates the field and never renders
  its name. The refusal waits for the record's closing brace, as upstream's
  reduction does, so a lexical error inside the record keeps the parser's own
  diagnostic, and an id past `u32` keeps upstream's "field id out of u32
  range". Variants and service methods compute no next id and are unaffected.

  **This refuses input that compiled in a release build before**, including
  valid Candid whose record field id is exactly 4294967295 (about one field
  name in 2^32). The owner chose this over a new error code and over waiting
  for an upstream fix; the decision is recorded on the issue. Every other
  source compiles exactly as before. No public API, error code, or serialized
  shape changed. `tests/record_field_id_overflow.rs` pins each case above,
  and, in debug builds, that the pinned upstream grammar still panics on every
  refused source.

### Resource bounds

- **A long run of consecutive comments is refused instead of aborting the
  host** ([issue #219]). The pinned upstream `candid_parser` tokenizer skips a
  comment by calling itself, so every comment between two real tokens costs it
  a stack frame, and it runs twice over each source: in the `source_nesting`
  preflight itself and in the parser. A flat run of `//` lines a few kilobytes
  long overflowed the stack and aborted the process, which no caller can catch:
  measured on macOS arm64, the first abort came at 673 comments (about 2 KB of
  `//` lines) on a 2 MiB thread in a debug build and at 3 554 in release, and
  `max_source_bytes` admits 349 525 such lines. Every entry
  point was exposed: `compile_did` and its variants, `compile_with_resolver`,
  `compile_did_file` and its variants, the `candid-core` binary's `compile`,
  and the `SourceInfo` rederivation behind `SourceInfo::validate` and the
  `Compilation` loaders.

  A constant-stack byte scan now counts consecutive comments, line and block
  alike, before that tokenizer first runs, and refuses a run longer than
  `max_source_nesting` (default 256) with `resource_limit_exceeded` naming
  `source_nesting`, `observed` being the limit plus one. Whitespace neither
  counts nor ends a run, so blank lines between comments do not split one; any
  real token ends it; a nested block comment counts once; comment markers
  inside a string literal are not comments. A run at the default costs about
  800 KB of stack in a debug build and 150 KB in release (measured, and stated
  in `Limits::max_source_nesting`).

  **This refuses input that compiled before**: a `.did` with more than 256
  consecutive comments anywhere, such as a long license header written as `//`
  lines, now fails under default limits. Raise the limit with
  `Limits::with_max_source_nesting`, or write the header as one `/* */` block,
  which counts once (a `//` header directly above a declaration is also its
  documentation, which a block comment is not). `@candid-core/cli`'s `gen`,
  `didToContract` and `didToModule` embed this compiler at the default limits
  and expose no override, so they refuse such a file too. A comment-run refusal
  also takes precedence over a lexical error anywhere in the source, because
  upstream would recurse through the run before reaching the error; inside the
  limit, a lexical error still reports the parser's own diagnostic. Inputs
  within the limit compile exactly as before: the same Contract, the same
  `contract_id`, `interface_id` and `source_bundle_id`, and the same
  `SourceInfo` documentation. No public API, error code, resource name, limit
  default, or serialized shape changed. `tests/deep_nesting.rs` pins each case
  above, including every entry point refusing the largest run
  `max_source_bytes` admits on a 64 KiB stack.

- **Source-info collection is linear in the source** ([issue #252]).
  Compiling with source info, the default, lowered every record, variant,
  func and service type written in the source through the pinned upstream
  `candid_parser`'s `ast_to_type`, which clones the whole type environment on
  every call, so the work grew as (those types) x (declarations), and it ran
  before any source-info limit was checked. Measured on aarch64 Linux in a
  release build, 8 000 one-record declarations compiled in 35.6 s with source
  info and 0.61 s without; an input over `source_string_bytes` paid the whole
  collection before being refused. The environment is now cloned once per
  compilation and every type is checked against it with the same upstream
  `check_type`, which outside its declaration pass only reads it: the same
  8 000 declarations compile in 0.86 s. The collection now also honours the
  runtime context's cancellation and deadline at every declaration, as
  lowering already did. Every entry point that collects source info is
  affected, including `@candid-core/cli`'s `gen`, `didToContract`,
  `didToModule`, `projectDid` and `checkCompatible`, which embed this compiler.

  Output is unchanged: the same Contract, identities and `SourceInfo`, the same
  error codes and precedence. No public API, limit default, or serialized
  shape changed. `tests/source_info_cost.rs` pins the property by counting
  allocated bytes, not time (#39): doubling a flat bundle of 500 declarations
  must less than triple what the compiler allocates (it doubles; the per-call
  clone made it 3.47x).

### Type checking

- **A type on a cycle that passes only through `opt` is refused**
  ([issue #234]). `type T = opt T;`, `type T = opt opt T;`, `type A = opt B;
  type B = opt A;` and the same through an alias, `type A = B; type B = opt
  A;`, type-check upstream, but decoding a value that is not an `opt` at such
  a type unwraps `opt` without end: Candid's coercion has no finite derivation
  there, so the reference `candid` crate stops only at its stack guard and a
  runtime only at its depth budget. The rule, applied to the lowered Contract
  arena, where aliases are already resolved: a type is refused when it lies on
  a cycle of the type graph whose every edge leaves an `opt` node. Any other
  constructor on the cycle (`record`, `variant`, `vec`, `func`, `service`)
  makes it productive, so `type L = opt record { head : nat; tail : L }`,
  `type T = opt vec T` and `type T = record { a : opt T }` compile as before.
  The check reuses the iterative cycle marking of the type-depth preflight
  (ADR 0005), linear in the arena.

  The refusal is `did_type_check_error`, phase `type_check`, the code
  `candid_parser`'s own cyclic-alias refusal already reports: "type T lies on
  a cycle that passes only through opt; candid-core refuses it, because
  decoding a value that is not an opt at such a type unwraps opt without end",
  naming the first such declaration in name order, with a source-scoped span
  naming the source that declares it (no byte offsets: the upstream AST
  carries none for a declaration). Every entry point applies it:
  `compile_did` and its variants, `compile_with_resolver`, `compile_did_file`
  and its variants, the `candid-core` binary's `compile`, and the `SourceInfo`
  rederivation behind `SourceInfo::validate` and the `Compilation` loaders;
  `@candid-core/cli` embeds this compiler.

  **This refuses input that compiled before**: a `.did` holding such a type
  anywhere, whether or not the actor reaches it, now fails to compile, and a
  `Compilation` document whose `SourceInfo` embeds such a source no longer
  loads, because its provenance no longer rederives. The Contract loaders do
  not change: a Contract document that holds such a cycle, written by hand or
  by 0.1.0-beta.3, still loads through `Contract::from_json`, and
  `ContractDraft::build` still accepts the graph; decoding a non-`opt` value at
  it still ends in a runtime's depth refusal. Every other input compiles
  exactly as before: the same Contract, `contract_id`, `interface_id`,
  `source_bundle_id` and `SourceInfo`. No public API, error code, resource
  name, limit or serialized shape changed. `tests/opt_only_cycle.rs` pins each
  case above.

### Contract JSON

- **A struct written as a JSON array is refused** ([issue #235]). Serde's
  derived `Deserialize` also reads a struct written as an array of its field
  values in declaration order, and an internally tagged enum written as
  `[tag, field…]`, at any depth, so `Contract::from_json` accepted a record
  field written `[97, 1]` and a declaration written `["R", 0]`, which
  `@candid-core/schema`'s `schemaFromContract` refuses (found by the #196
  differential fuzz). The Contract format is objects only: candid-core
  writes nothing else, and its DTOs already deny unknown keys. The bounded
  loaders now refuse that form at every struct position: the document, its
  `identities`, `producer` and `actor`, a type node, a record field or variant
  arm, a service method and a declaration; the envelope and its `contract`;
  the Compilation, its `contract` and `source_info`, each entry of the
  sidecar's collections, and an entry's `origin` and `label`. The error is the
  one a value of the wrong type gets, `ContractJsonError::MalformedJson`
  (`malformed_contract_json` from `candid-core validate`), with the message
  `invalid type: sequence, expected an object at <path>, line <l> column <c>`,
  `<path>` being `$`-rooted, such as `$.types[0].fields[0]`.

  **This refuses input that loaded before**: a document with a struct written
  as an array fails `Contract::from_json`, `from_json_with_limits`,
  `from_json_with_context`, `from_slice_with_limits` and
  `from_slice_with_context`, the same four `_with_limits`/`_with_context`
  loaders of `ContractEnvelope` and of `Compilation`, and `candid-core
  validate`. Write the object form. Every document candid-core writes,
  `0.1.0-beta.3`'s included, is objects only and loads exactly as before, with
  the same identities. A document the typed decode refuses keeps the decode's
  own error, and envelope extension values stay free-form JSON. Not changed:
  the serde DTOs themselves (`RawContract`, `ContractDraft`, `TypeNode`,
  `RawSourceInfo` and the rest) still read the array form when a caller
  decodes them with serde directly, so `Contract::try_from_raw` and
  `Compilation::try_from_raw` take whatever the caller decoded. No public API,
  error variant, code, or serialized shape changed.

  The check is one forward pass over the bytes the typed decode has just
  accepted, linear in their length, with an explicit stack. It allocates one
  frame per open object or array, plus a short-lived string for each key
  written with an escape, which it decodes as serde does.
  `tests/struct_sequence_form.rs` pins each position and each loader entry
  point.

- **A unit variant written as a JSON map is refused** ([issue #238]). Serde's
  derived `Deserialize` also reads a unit variant of an externally tagged enum
  written as a map from the variant's name to a unit, so `Contract::from_json`
  accepted a primitive node written `"primitive": {"nat": null}` instead of
  `"primitive": "nat"`, and a func's `"mode": {"query": null}`, which
  `@candid-core/schema`'s `schemaFromContract` refuses. Inside a type node,
  whose fields serde buffers before it reads them, `{"nat": {}}` was read as
  well. The format writes every unit variant as a string. The bounded loaders
  now refuse the map form at every unit-variant position: a type node's
  `primitive` and a func's `mode`, in the Contract and in the `contract` of an
  envelope or a Compilation, and an import's `kind` and a function argument's
  `direction` in the Compilation's `source_info`. The error is the one a value
  of the wrong type gets, `ContractJsonError::MalformedJson`
  (`malformed_contract_json` from `candid-core validate`), with the message
  `invalid type: map, expected a string at <path>, line <l> column <c>`, such
  as `$.types[1].primitive`. Every other map spelling at those positions
  (`{"nat": []}`, `{"nat": 0}`, `{}`, two entries, an unknown name) was
  already refused by the decode and keeps the decode's own error.

  **This refuses input that loaded before**: a document with a unit variant
  written as a map fails the same loaders as above (`Contract`'s five,
  `ContractEnvelope`'s and `Compilation`'s four each) and `candid-core
  validate`. Write the string. Every document candid-core writes,
  `0.1.0-beta.3`'s included, writes the string and loads exactly as before,
  with the same identities. The serde DTOs themselves still read the map form
  when a caller decodes them with serde directly. No public API, error
  variant, code, or serialized shape changed.

  The check is the same forward pass as the struct-as-array one, which now
  also allows an object only at the root, inside an array, or as the value of
  one of the format's object-valued keys (`identities`, `producer`, `actor`,
  `contract`, `extensions`, `source_info`, `origin`, `label`), and refuses one
  anywhere else. It stays linear in the input's length, with an explicit
  stack, and allocates as before: one frame per open object or array, plus a
  short-lived string for each key written with an escape.
  `tests/unit_variant_map_form.rs` pins each position in each spelling and
  each loader entry point.

### Release gates

- **The semver gate reads a release's own section on its release commit**
  ([issue #222]). `verify_semver.py` accepted a reported break only when a
  list item beginning with the bolded `BREAKING` marker acknowledged it under
  `## Unreleased`. Release prep renames that section to the version being
  released while the gate still compares against the previous release, so a
  release that carried an acknowledged break failed the gate on its own
  release commit. The gate now also reads the section headed by the
  `Cargo.toml` version whenever that version is not the baseline, which is
  exactly the release commit. Between releases `Cargo.toml` carries the
  version already published, and its section is not read: an acknowledgement
  there is for a break that release already shipped. A section is also found
  when it is the last one in the file. `tests/fixtures/packaging/test_verify_semver.py`
  pins each case, and the `Semver compatibility` job runs it before the
  comparison. Nothing in the published crate changed.

### Known limitations

- **ADR verification is incomplete.** ADR 0002 is **Verified**; ADRs 0001 and
  0003–0007 remain **Implemented, verification pending**
  ([docs/verification.md](docs/verification.md)).
- **Decided before 1.0, not in this release.** A record field's name attaches
  to the shared canonical node, so two declarations with the same structure
  can show each other's spelling ([issue #229]); the fix changes the Contract
  format. What `maxElements` counts when `@candid-core/schema` validates a
  record-shaped value differs from candid-core's own bound ([issue #241]).
- **The `candid-core` binary** refuses an entry `.did` named through a file
  symlink, or in a different letter case on a case-insensitive filesystem,
  with `did_source_not_found` ([issue #251]).
- **Carried from earlier releases.** The HostValue ↔ Candid binary bridge is
  deferred. Bounded JSON decoding enforces `max_input_bytes` before decoding
  rather than element by element. `HostValue::from_json_with_limits` gates on
  `max_value_bytes` with its own error shape. Raising `max_value_nesting`
  above 128 has no effect beneath `serde_json`'s fixed ceiling.
  `max_type_preflight_work` bounds the depth guards' own work, so a host whose
  allocator fails first gets an allocation abort in place of the structured
  refusal.
- **The npm packages version independently.** `@candid-core/schema` and
  `@candid-core/cli` are not part of this crate's published surface;
  `@candid-core/cli` still embeds an earlier revision of this crate until its
  own next release.

## 0.1.0-beta.3 — published 2026-08-24

The third prerelease, and the first that carries a fix for a defect in an
already-published version. Released from commit
`fcbf7eb39d8c8a337c5c2546ba0fdd13201e595c`. The `.crate` SHA-256 crates.io
recorded is
`c9c83cc8bf68bfa4747b076bfdf41b366367431d7e5659fdd1c547661699e2b5`, matching
the digest the release gates measured, the one the `guard` job reproduced by
repackaging the commit independently, and the one the `confirm` job read back
from crates.io after publication.

The archive published under this version was built and digested before
publication, so the copy of this file inside it still describes the release as
prepared. This version follows the issue #81 bump protocol:
`ProducerInfo::current().version` (and therefore the octets of newly serialized
documents) is the only observable change from the bump itself — no
`contract_id`, `interface_id`, `source_bundle_id`, or frozen exact-octet vector
moved, and `tests/release_metadata.rs` asserts the change and the non-change
together.

Two things made this release due rather than optional. The two versions before
it — `0.1.0-beta.1` and `0.1.0-beta.2` — carry the unbounded type-depth walk
described below, reachable from the public `compile_did` entry point under
default `Limits`, and a published version cannot be corrected in place, so both
should be left behind rather than pinned. And `compile --envelope` — the step
`@candid-core/schema` tells a consumer to run immediately after `cargo install
candid-core` — existed in no published version of this crate until this one, so
that on-ramp could not ship honestly until it did.

### Resource bounds

- **The type-depth guard walks no longer re-expand shared subtrees, and they
  charge for the work they do** ([issue #125]). Both traversals that enforce
  `max_type_depth` — the parse-side preflight over declaration references and
  the checked-type walk before lowering — walked declaration DAGs as trees,
  re-expanding a subtree reached through two sibling references once per
  incoming edge: O(2^n) node visits from a sub-kilobyte source, with nothing
  charged, because the loops observed only cancellation and deadlines and
  `compile_did` configures neither. On the issue's reproduction a 781-byte
  `.did` burned 5.097 s at n = 20, doubling per level, and no refusal ever came
  — the walk returned `Ok`. Every consumer that compiles untrusted or
  user-supplied `.did` source inherited this, in `0.1.0-beta.1` and
  `0.1.0-beta.2` alike, so both should be left behind rather than pinned.

  Each walk now builds a recursion map first — the declaration reference graph
  and the members of its cycles, computed iteratively — tracks only cycle
  members in the per-path active set, and skips any expansion state
  `(node, depth, active recursive names)` it has already seen. Pruning exact
  duplicate states cannot change what a walk refuses, which is the evidence that
  this is a cost fix rather than a behaviour one: the alias-chain pins hold
  unchanged at `type_depth` observed 257, recursive types compile as before, and
  the n = 24 fan-out — O(2^24) visits under the old walks — compiles at a pinned
  2 796 work units.

- **`Limits::max_type_preflight_work` and `Limits::with_max_type_preflight_work`
  are new public API** ([issue #125]), defaulting to **10 000 000**. Every
  recursion-map node and expansion state is charged against this counter, plus
  one unit per recursive name tracked on a state's path, so the shapes that
  still multiply states fail closed with `resource_limit_exceeded` naming
  `type_preflight_work` instead of hanging. It is deliberately separate from
  `max_canonicalization_work`: both accrue on one budget in a single
  compilation, and metering the depth guards on the graph counter would let a
  type-heavy bundle starve the canonicalization that follows it. Realistic
  contracts are nowhere near the ceiling — the largest corpus fixture,
  `ledger.did`, consumes 806 units end to end, pinned by
  `tests/deep_nesting.rs`.

  Deduplication is also what makes these walks *retain*, where the tree walks
  they replaced held only a stack, so this counter bounds memory as well as
  time: about **19 bytes of peak live heap per unit**, measured across bundle
  sizes and pinned inside a band by `tests/type_preflight_memory.rs`. At the
  default that authorizes roughly 190 MB before the counter refuses — ample on
  an ordinary host, and more than a small 32-bit `wasm32` heap can serve. A host
  whose allocator fails before the counter does gets an allocation abort in
  place of the structured refusal this is designed to return, so a browser or
  other constrained host should lower the limit to what its heap can actually
  hold: 1 000 000 keeps the walks under ~19 MB and still clears every realistic
  contract by three orders of magnitude.

  The addition is additive in both senses that matter. The public surface gains
  two inherent methods and removes nothing, so the semver gate reports no break
  against `0.1.0-beta.2` and this release acknowledges none. The serialized
  override key is additive exactly like `max_artifact_identity_work`'s:
  `Limits::default` and every configuration that leaves this limit at its
  profile value still serialize with no `max_type_preflight_work` key at all, so
  existing documents are unchanged in both directions, while a document that
  does carry the key is readable by this build and newer ones and rejected by
  one that predates it — the intended failure, since silently ignoring an
  unknown limit override would apply a policy the document did not ask for.
  `LIMITS_CONFIG_VERSION` is unchanged: `Limits` fields are private precisely so
  that adding one is never a breaking change, and no existing key, value, or
  default moved.

### Command-line interface

- **`compile <path> --envelope` emits a one-document `ContractEnvelope`**
  ([issue #152]): `{"contract": …, "extensions":
  {"org.candid-core.field-names/v1": [[container, id, name], …]}}` — the
  canonical Contract plus its field-name table as an envelope extension, so
  one self-describing document feeds `@candid-core/schema`'s
  `schemaFromContract` without a separate provenance sidecar. The triples are
  the named field labels only, sorted and deduplicated — exactly the table
  the TypeScript generator builds from `SourceInfo`, pinned against its
  goldens by test. Extensions live outside `contract_id` and `interface_id`
  by the envelope's design, so the emitted document keeps the identities of
  its plain-`compile` sibling. The flag is mutually exclusive with
  `--no-source-info` (the envelope exists to carry the names that flag
  suppresses); existing output shapes are untouched, and envelope-emission
  failures use the standard `{"ok": false, "diagnostics": […]}` channel.
  No library API changed.

### Documentation

- **The packaged `README.md` documents the TypeScript on-ramp and the full CLI
  grammar** ([issue #148], [issue #152], [issue #153]). It names
  `@candid-core/schema` — the Zod-style schema runtime this repository also
  produces, versioned independently — records that the `candid-core` binary is
  what turns a `.did` file into the document its `schemaFromContract` consumes,
  notes the prepared `@candid-core/cli` wasm package, and states the `compile`
  grammar, the usage-error contract, and the envelope output shape the flag
  above adds.
- **ADR 0005 records the type-preflight counter and what it trades**
  ([issue #125]) — the recursion map, per-path cycle stopping, and state
  deduplication, written down beside the other dedicated counters, including the
  measured per-unit heap figure that makes the default a memory bound as well as
  a time bound.

## 0.1.0-beta.2 — published 2026-07-28

The second prerelease, released from commit
`13cc0284c3fe9059311a408d90da96a151a87d06`. The `.crate` SHA-256 crates.io
recorded is
`8ef93f40f5da95e4d0b6bdf34df0f9a41f1e2bf0a535564051cbfe8f36fa5095`, matching
the digest the release gates measured and the one the `confirm` job read back
from crates.io after publication. This was the first version released through
the dispatch-only `Release` workflow described below.

The archive published under this version was built and digested before
publication, so the copy of this file inside it still describes the release as
prepared rather than published — unavoidable, since editing it would change the
commit and therefore the digest. This version follows the issue #81 bump
protocol: `ProducerInfo::current().version` (and therefore the octets of newly
serialized documents) is the only observable change from the bump itself — no
`contract_id`, `interface_id`, `source_bundle_id`, or frozen exact-octet vector
moved, and `tests/release_metadata.rs` asserts the change and the non-change
together.

### Release automation

- **The tag, crates.io publish, and GitHub release are performed by a
  dispatch-only workflow behind three separately-approved protected
  environments** ([issue #89]). The guard repackages the release commit with
  the pinned Cargo and refuses on any digest mismatch; publication uses
  crates.io Trusted Publishing, so no long-lived registry credential exists
  anywhere; the published checksum is read back from crates.io and compared
  automatically. [docs/releasing.md](docs/releasing.md) §7 now documents the
  dispatch-and-approve procedure, with the manual path retained as a recorded
  fallback. This version is the first released through it.

### Release gates

- **Semver compatibility is now checked against the published baseline**
  ([issue #92]). `cargo semver-checks` compares both published surfaces against
  the latest release and reports breaking changes. Because this crate is pre-1.0
  and reserves the right to break, the gate does not forbid them — it forbids
  *undocumented* ones: a reported break must be acknowledged by a list item in
  the Unreleased section of this file that begins with a bolded `BREAKING`
  marker followed by a colon. The marker has to start the item; a mention
  inside prose does not count, which is why this entry does not itself
  acknowledge anything.
  Adding this gate does not promise semver stability before 1.0, and does not
  promote the Contract format to a stable v1.

### Documentation

- **The effective ceiling on HostValue record width is now documented**
  ([issue #88]). Record validation is deliberately allocation-free: it scans
  pairwise and charges `canonicalization_work` per comparison rather than
  building a field-ID index, so the cost is roughly `1.5n²` for an `n`-field
  record. At the default `max_canonicalization_work` that binds a single record
  at **2 581 fields** — far below the 500 000 `max_fields` permits and the
  1 000 000 `max_value_elements` permits, neither of which is the binding limit
  for a wide record. The rustdoc on all three limits now says so, and a
  regression test pins the boundary and the exact resource metadata one field
  over. No behaviour changed: this release documents what the code already did.
  The failure was, and remains, structured and deadline-interruptible rather
  than a hang.
- **Benchmark comparison governance is documented**
  ([issue #39], now delivered and closed).
  [docs/benchmarks.md](docs/benchmarks.md) records the durable-baseline
  format, the comparison tool that refuses rather than guesses when runs are
  not comparable, the three CI tiers, and the standing decision that no timing
  or allocation measurement ever fails a workflow in this repository.

### Packaging

- **The repository became a Cargo workspace** ahead of the TypeScript
  generator crate ([issue #38]). This is invisible to consumers: Cargo strips
  the `[workspace]` table from the normalized manifest, the archive remains
  exactly the intentional allowlist, and the packaged-consumer gates prove it.
  The generator crate itself is unpublishable and outside the archive.

## 0.1.0-beta.1 — published 2026-07-27

The first published version, released from commit
`819a3c9062bf6420bec66fb6e8fd9c7c67add50c` with Cargo 1.94.1. The `.crate`
SHA-256 crates.io recorded is
`4082434fe0057bf9bccabd9e987cb9772137488e6294e05f28c094b544cf8224`, matching the
digest the release gates measured.

The archive published under this version was built before publication, so the
copy of this file inside it still describes the release as prepared rather than
published — unavoidable, since editing it would change the commit and therefore
the digest. The tag, the crates.io publish, and the GitHub prerelease were
separately authorized steps, set out in [docs/releasing.md](docs/releasing.md).

Because this is the first version, everything below describes the shape of the
initial surface rather than a change from a predecessor.

### The crate

- **Four build surfaces from one published package.** The implicit base — what
  is left when `default-features = false` removes every feature — is the pure
  Contract model: DTOs, validation, canonicalization, the semantic identities,
  detached artifact identity, `Limits`/`RuntimeContext`/`CancellationToken`,
  `ContractEnvelope`, and the diagnostics those need. It links no Candid engine
  and no filesystem capability crate. `compiler` adds Candid source compilation
  with no host filesystem. `filesystem-compiler` adds native filesystem access
  and the `candid-core` binary on top of `compiler`. `host-value` adds the
  lossless tagged host value ABI and graph-directed value validation. All three
  are on by default.

  These are *dependency-graph* boundaries, and they are verified as such:
  `tests/fixtures/packaging/verify_feature_graph.py` resolves `cargo metadata`
  per feature set and target and asserts what each one may and may not contain.
  Cargo unifies features across a build, so feature selection bounds what a
  dependency graph must contain, not what a single unified build produces.

- **Browser-WASM imported compilation.** `compiler` is the surface a browser
  host builds, and it covers multi-file bundles as well as self-contained
  sources. `compile_with_resolver` takes an entry ID plus a `SourceResolver` and
  compiles the whole bundle in memory through the official `candid_parser`
  merged-program APIs — no materialization, no temporary directory, no
  `cap-std`, no ambient authority. `tests/browser_wasm.rs` is the runtime
  evidence: it compiles both import kinds plus a diamond inside headless Chrome,
  pins the resulting identities and provenance, and asserts that resolver,
  resource, cancellation, and deadline failures stay structured on a target with
  no filesystem and no clock.

- **Semantic identity and exact-octet identity are different things.**
  `contract_id` and `interface_id` are *semantic* Contract identities: documents
  that mean the same thing share them on purpose, so rewriting `producer`,
  editing an envelope extension, or re-encoding the JSON leaves both untouched.
  `source_bundle_id` is a raw-source bundle content identity over source bytes
  and import edges. None of the three identifies a complete serialized document.
  `artifact_id_with_limits` does, per declared `ArtifactKind`, and returns the ID
  to the caller rather than writing it into the document. No unkeyed content ID
  authenticates itself.

- **All untrusted work is bounded.** Untrusted JSON goes through the
  `*_with_limits`/`*_with_context` parse APIs, which gate byte length before
  decoding and then share one budget with validation. `Contract`,
  `ContractEnvelope`, `Compilation`, and `HostValue` deliberately do not
  implement `Deserialize`, because a trait impl has no argument position for a
  resource policy.

### Pre-1.0 API and wire instability

- The public Rust API is unstable. `tests/fixtures/packaging/public-api-base.txt`
  and `public-api-all-features.txt` are committed inventories of both published
  surfaces so that a change is visible in review. They record the API; they do
  not promise it.
- The serialized Contract, Compilation, and envelope shapes are unstable, and so
  are the canonical bytes and every identity computed over them. A future
  release may move `contract_id` for an unchanged input; if that happens it will
  be a canonicalization-profile change recorded here.
- The Contract format is **not** promoted to a stable v1 by this release. See
  the ADR status note under Known limitations.
- `cargo semver-checks` is deliberately absent: there is no published
  `candid-core` baseline to compare against. Semver comparison begins with the
  release *after* this one.
- Because `0.1.0-beta.1` is a prerelease, a `"0.1"` requirement does not select
  it. Consumers ask for it by name: `candid-core = "=0.1.0-beta.1"`.

### Migrations already in place

These removals happened during the pre-1.0 API cleanup ([issue #23]) and are
documented in the README under "Migrating from the pre-cleanup producer APIs".
They are repeated here because a first-time reader of this changelog will not
have seen them:

- `RawContract::new` and `Contract::build_raw`/`build_raw_with_context` are
  gone. A producer-facing constructor that fabricated placeholder zero
  identities made the intuitive `RawContract::new` → `Contract::try_from_raw`
  pairing fail by construction. Use `ContractDraft`, which has no identity
  fields at all, plus `.with_producer(..)` when a caller-supplied producer is
  needed.
- `Limits` no longer exposes public fields or exhaustive struct literals.
  Construct through a profile plus builders (`Limits::default().with_…`) and
  read through getters.
- `ResourceLimitInfo.limit`/`.observed` and `SourceSpan.start_byte`/`.end_byte`
  moved from platform-width `usize` to fixed-width `u64`. The serialized JSON
  numeric text is unchanged.
- Serialized `Limits` documents moved from a bare field map to the versioned
  portable configuration.

### Packaging

- The published archive is a positive `include` allowlist: production source,
  runnable examples, the public documentation set, `README.md`, `CHANGELOG.md`,
  `LICENSE`, and the manifest material Cargo adds itself. Test suites and their
  fixtures, benchmarks and their corpus, the fuzz crate, CI workflows, and
  agent/skill assets are not published.
  `tests/fixtures/packaging/verify_package_manifest.py` asserts both halves:
  every required path present, every forbidden path absent, and no unexplained
  extra path.
- A root `LICENSE` carries the complete Apache License 2.0 text.
- The manifest declares `repository`, `homepage`, `documentation`, `readme`, and
  a narrow keyword/category set.
- A `Release candidate` workflow packages the crate with the exact Cargo pinned
  in `tests/fixtures/packaging/release-tools.env` — `cargo package` bytes are
  reproducible within a Cargo version and not across versions, so the digest and
  the publish have to share one — verifies the archive manifest, records its
  SHA-256, Cargo version, and exact file list as CI evidence, and builds
  external consumers — base, `compiler`, full native, the installed CLI, and two
  `wasm32-unknown-unknown` surfaces — against the *unpacked archive* rather than
  against this repository. It holds `contents: read`, accepts no crates.io
  token, and neither tags, releases, nor publishes anything.

### Known limitations

- **ADR verification is incomplete.** ADR 0002 (versioning and canonical bytes)
  is **Verified**: an independent Python reference reproduces all 11
  canonicalization vectors, and the run is recorded in
  [docs/verification.md](docs/verification.md). ADRs 0001 and 0003–0007 remain
  **Implemented, verification pending**. The implementations are in place and,
  for ADR 0007, an independent reference and a CI job exist, but no run of that
  job is recorded yet. Wiring is not evidence.
- **The host-value ↔ Candid binary bridge is deferred.** `HostValue` is a
  lossless tagged ABI validated against the Contract graph; encoding to and
  decoding from Candid's binary wire format is not part of this crate yet.
- **Bounded decoding is coarse.** `from_json_with_limits` and its siblings
  enforce `max_input_bytes` before decoding, which bounds peak decode allocation
  to a multiple of the caller's ceiling. They do not reject element by element
  during decode; that remains a follow-up.
- **`HostValue` uses a different byte gate.** `HostValue::from_json_with_limits`
  gates on `max_value_bytes`, not `max_input_bytes`, and reports
  `HostValueJsonError::Limit`, which carries no `resource` name. Lowering
  `max_input_bytes` alone does not bound HostValue decoding.
- **Raising `max_value_nesting` above 128 has no effect**, because serde_json's
  own fixed 128-frame ceiling is left in place underneath the crate-owned check.
- **TypeScript generation and comparison are not in this release**
  ([issue #38]).
- **Benchmark regression gating is not in this release** ([issue #39]). Pull
  requests compile and exercise every benchmark once; weekly runs retain
  Criterion estimates, allocation measurements, toolchain, host, and commit as
  CI artifacts, but no wall-clock threshold is enforced.

[issue #23]: https://github.com/b3hr4d/candid-core/issues/23
[issue #38]: https://github.com/b3hr4d/candid-core/issues/38
[issue #39]: https://github.com/b3hr4d/candid-core/issues/39
[issue #88]: https://github.com/b3hr4d/candid-core/issues/88
[issue #89]: https://github.com/b3hr4d/candid-core/issues/89
[issue #92]: https://github.com/b3hr4d/candid-core/issues/92
[issue #125]: https://github.com/b3hr4d/candid-core/issues/125
[issue #148]: https://github.com/b3hr4d/candid-core/issues/148
[issue #152]: https://github.com/b3hr4d/candid-core/issues/152
[issue #153]: https://github.com/b3hr4d/candid-core/issues/153
[issue #176]: https://github.com/b3hr4d/candid-core/issues/176
[PR #177]: https://github.com/b3hr4d/candid-core/pull/177
[issue #219]: https://github.com/b3hr4d/candid-core/issues/219
[issue #222]: https://github.com/b3hr4d/candid-core/issues/222
[issue #229]: https://github.com/b3hr4d/candid-core/issues/229
[issue #230]: https://github.com/b3hr4d/candid-core/issues/230
[issue #234]: https://github.com/b3hr4d/candid-core/issues/234
[issue #235]: https://github.com/b3hr4d/candid-core/issues/235
[issue #238]: https://github.com/b3hr4d/candid-core/issues/238
[issue #241]: https://github.com/b3hr4d/candid-core/issues/241
[issue #251]: https://github.com/b3hr4d/candid-core/issues/251
[issue #252]: https://github.com/b3hr4d/candid-core/issues/252
[issue #256]: https://github.com/b3hr4d/candid-core/issues/256
[issue #263]: https://github.com/b3hr4d/candid-core/issues/263
