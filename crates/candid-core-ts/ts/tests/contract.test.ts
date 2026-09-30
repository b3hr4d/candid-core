// `schemaFromContract` construction tests: the fail-closed paths, the
// collapsing-opt forms (which load and box), deferred handling, and the
// bounded-input guards. The golden cross-check in crosscheck.test.ts covers
// the happy paths against the generated builders.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  schemaFromContract,
  FIELD_NAMES_EXTENSION,
  type ContractIssueCode,
  type SchemaFromContractResult,
} from "../contract.ts";
import { validate } from "../validate.ts";
import { candidLabelHash } from "../labels.ts";
import { isBoxedOpt, serviceMethods, type AnySchema } from "../schema.ts";

type Json = ReturnType<typeof JSON.parse>;

/** A minimal canonical-shaped document around an arena and declarations. */
function document(types: Json[], declarations: Json[], actor?: Json): Json {
  return {
    format: "candid-core",
    format_version: 1,
    semantics_profile: "candid-1",
    canonicalization_profile: "candid-core-canon-1",
    types,
    declarations,
    ...(actor === undefined ? {} : { actor }),
  };
}

const primitive = (name: string): Json => ({ kind: "primitive", primitive: name });

function codesOf(result: SchemaFromContractResult): ContractIssueCode[] {
  return result.ok ? [] : result.issues.map((issue) => issue.code);
}

function failsWith(result: SchemaFromContractResult, code: ContractIssueCode, path: string): void {
  assert(!result.ok, "expected schema construction to fail");
  if (result.ok) {
    throw new Error("unreachable");
  }
  assert.strictEqual(result.issues.length, 1);
  assert.strictEqual(result.issues[0].code, code);
  assert.strictEqual(result.issues[0].path, path);
}

/** Build a document that must load, and return its schemas. */
function builds(result: SchemaFromContractResult): { readonly [name: string]: AnySchema } {
  assert(result.ok, `expected schema construction to succeed: ${JSON.stringify(codesOf(result))}`);
  if (!result.ok) {
    throw new Error("unreachable");
  }
  return result.schemas;
}

// Collapsing opts (`opt opt`, `opt null`, `opt reserved`) load and box their
// present value as `{ some: v }`; the four forms the loader used to refuse
// with `unrepresentable_option`, now each held to its three states.
test("collapsing opts load and box: opt opt", () => {
  const { DoubleOpt } = builds(
    schemaFromContract(
      document(
        [{ kind: "opt", inner: 1 }, { kind: "opt", inner: 2 }, primitive("nat")],
        [{ name: "DoubleOpt", type: 0 }],
      ),
    ),
  );
  assert.strictEqual(isBoxedOpt(DoubleOpt), true);
  for (const value of [null, { some: null }, { some: 5n }]) {
    assert.deepStrictEqual(validate(DoubleOpt, value), { ok: true });
  }
  assert.strictEqual(validate(DoubleOpt, 5n).ok, false);
});

test("collapsing opts load and box: opt null", () => {
  const { OptNull } = builds(
    schemaFromContract(
      document([{ kind: "opt", inner: 1 }, primitive("null")], [{ name: "OptNull", type: 0 }]),
    ),
  );
  assert.deepStrictEqual(validate(OptNull, { some: null }), { ok: true });
  assert.deepStrictEqual(validate(OptNull, null), { ok: true });
});

test("collapsing opts load and box: opt reserved", () => {
  const { OptReserved } = builds(
    schemaFromContract(
      document(
        [{ kind: "opt", inner: 1 }, primitive("reserved")],
        [{ name: "OptReserved", type: 0 }],
      ),
    ),
  );
  assert.deepStrictEqual(validate(OptReserved, { some: "anything" }), { ok: true });
  // `undefined` is not a Candid value, and a bare one is no box.
  assert.strictEqual(validate(OptReserved, undefined).ok, false);
});

test("collapsing opts load and box: the aliased form", () => {
  // `type Inner = opt nat; type Outer = opt Inner` — the arena holds no alias
  // indirection, so Outer's inner node *is* the opt node, and the box follows
  // the node, not its spelling. Inner itself stays `bigint | null`.
  const { Inner, Outer } = builds(
    schemaFromContract(
      document(
        [{ kind: "opt", inner: 1 }, { kind: "opt", inner: 2 }, primitive("nat")],
        [
          { name: "Inner", type: 1 },
          { name: "Outer", type: 0 },
        ],
      ),
    ),
  );
  assert.strictEqual(isBoxedOpt(Outer), true);
  assert.strictEqual(isBoxedOpt(Inner), false);
  assert.deepStrictEqual(validate(Outer, { some: 5n }), { ok: true });
  assert.deepStrictEqual(validate(Inner, 5n), { ok: true });
});

test("a self-recursive opt loads and boxes at every level", () => {
  // `type Chain = opt Chain`: the node's inner is itself. Construction is
  // lazy, so building never forces the cycle; walking decides per level.
  const { Chain } = builds(
    schemaFromContract(document([{ kind: "opt", inner: 0 }], [{ name: "Chain", type: 0 }])),
  );
  assert.deepStrictEqual(validate(Chain, { some: { some: null } }), { ok: true });
  assert.strictEqual(validate(Chain, { some: {} }).ok, false);
});

test("a func nested in a supported type builds (issue #104)", () => {
  const result = schemaFromContract(
    document(
      [
        {
          kind: "record",
          fields: [{ id: 1, type: 1 }],
        },
        { kind: "func", args: [], results: [], mode: "query" },
      ],
      [{ name: "Holder", type: 0 }],
    ),
  );
  assert(result.ok, "nested funcs are constructible since #104");
  if (result.ok) {
    const value = {
      _1_: { principal: "aaaaa-aa", method: "go" },
    };
    assert.deepStrictEqual(validate(result.schemas.Holder, value), { ok: true });
    assert(!validate(result.schemas.Holder, { _1_: "nope" }).ok);
  }
});

test("reference declarations build and the actor is a service schema", () => {
  // Issue #104: func/service/class are no longer deferred.
  const result = schemaFromContract(
    document(
      [
        { kind: "func", args: [], results: [], mode: "query" },
        { kind: "service", methods: [] },
        { kind: "record", fields: [] },
      ],
      [
        { name: "Callback", type: 0 },
        { name: "Registry", type: 1 },
        { name: "Kept", type: 2 },
      ],
      { kind: "service", service: 1 },
    ),
  );
  assert(result.ok, "every declaration must build");
  if (result.ok) {
    assert.deepStrictEqual(Object.keys(result.schemas), ["Callback", "Registry", "Kept"]);
    assert(result.actor !== undefined, "the document carries an actor");
    // A func value is { principal, method }; a service value is a principal.
    const funcValue = { principal: "aaaaa-aa", method: "go" };
    assert.deepStrictEqual(validate(result.schemas.Callback, funcValue), { ok: true });
    assert.deepStrictEqual(validate(result.schemas.Registry, "aaaaa-aa"), {
      ok: true,
    });
    assert.deepStrictEqual(validate(result.schemas.Kept, {}), { ok: true });
    assert(!validate(result.schemas.Callback, { method: "go" }).ok);
  }
});

test("an actorless document has no actor schema", () => {
  const result = schemaFromContract(
    document([{ kind: "record", fields: [] }], [{ name: "Unit", type: 0 }]),
  );
  assert(result.ok);
  if (result.ok) {
    assert.strictEqual(result.actor, undefined);
  }
});

test("dangling type references fail closed, path-addressed", () => {
  failsWith(
    schemaFromContract(document([{ kind: "opt", inner: 7 }], [{ name: "A", type: 0 }])),
    "dangling_type_ref",
    "$.types[0].inner",
  );
  failsWith(
    schemaFromContract(document([primitive("nat")], [{ name: "A", type: 3 }])),
    "dangling_type_ref",
    "$.declarations[0].type",
  );
});

test("format and profile markers are required to match exactly", () => {
  const wrong = document([], []);
  wrong.format = "not-candid-core";
  wrong.format_version = 2;
  wrong.semantics_profile = "candid-9";
  wrong.canonicalization_profile = "other";
  const result = schemaFromContract(wrong);
  assert.deepStrictEqual(codesOf(result), [
    "unsupported_contract_format",
    "unsupported_format_version",
    "unsupported_semantics_profile",
    "unsupported_canonicalization_profile",
  ]);
});

test("non-object documents and malformed nodes fail closed", () => {
  failsWith(schemaFromContract(null), "invalid_contract_document", "$");
  failsWith(schemaFromContract("{}"), "invalid_contract_document", "$");
  failsWith(
    schemaFromContract(document([{ kind: "wat" }], [])),
    "invalid_contract_document",
    "$.types[0].kind",
  );
  failsWith(
    schemaFromContract(document([primitive("wat")], [])),
    "invalid_contract_document",
    "$.types[0].primitive",
  );
  failsWith(schemaFromContract(document([42], [])), "invalid_contract_document", "$.types[0]");
});

test("declaration defects reuse candid-core's codes", () => {
  failsWith(
    schemaFromContract(document([primitive("nat")], [{ name: "", type: 0 }])),
    "empty_declaration_name",
    "$.declarations[0].name",
  );
  failsWith(
    schemaFromContract(
      document(
        [primitive("nat")],
        [
          { name: "A", type: 0 },
          { name: "A", type: 0 },
        ],
      ),
    ),
    "duplicate_declaration_name",
    "$.declarations[1].name",
  );
});

test("duplicate field ids fail closed", () => {
  failsWith(
    schemaFromContract(
      document(
        [
          {
            kind: "record",
            fields: [
              { id: 5, type: 1 },
              { id: 5, type: 1 },
            ],
          },
          primitive("nat"),
        ],
        [{ name: "A", type: 0 }],
      ),
    ),
    "duplicate_field_id",
    "$.types[0].fields[1].id",
  );
});

test("a lying name table fails closed at the table, before any key renders", () => {
  const doc = document(
    [
      {
        kind: "record",
        fields: [
          { id: 1, type: 1 },
          { id: 2, type: 1 },
        ],
      },
      primitive("nat"),
    ],
    [{ name: "A", type: 0 }],
  );
  // A name that does not hash back to its id would make the codec encode a
  // wrong wire id; hash enforcement (issue #103) rejects the entry itself.
  const lying = schemaFromContract(doc, { names: [[0, 1, "same"]] });
  failsWith(lying, "invalid_name_table", "$.names[0]");
  // An `_N_`-shaped name that does not hash to its id is a lying entry like
  // any other, refused at the table.
  const reserved = schemaFromContract(doc, { names: [[0, 1, "_2_"]] });
  failsWith(reserved, "invalid_name_table", "$.names[0]");
});

// Issue #189, amending #103 and #115: an *honest* `_N_`-shaped name — "_2_"
// hashes to 4735500 — is real provenance, but erased to a schema key it is
// indistinguishable from the rendering of numeric id 2, so the codec would
// derive the wrong wire id from it. It never becomes a key: the declaration
// that would render it is omitted, with everything referencing it, exactly
// as the generator omits it — no longer a whole-document refusal.
test("an honest _N_-shaped name omits its declaration instead of refusing", () => {
  const honestHash = schemaFromContract(
    document(
      [{ kind: "record", fields: [{ id: 4_735_500, type: 1 }] }, primitive("nat")],
      [{ name: "A", type: 0 }],
    ),
    { names: [[0, 4_735_500, "_2_"]] },
  );
  assert(honestHash.ok, "an honest reserved name is an omission, not a refusal");
  if (honestHash.ok) {
    assert.deepStrictEqual(Object.keys(honestHash.schemas), []);
    assert.deepStrictEqual(honestHash.omitted, [
      { kind: "declaration", name: "A", reason: "reserved_field_name" },
    ]);
  }
  // Cross-path agreement with the generator (#115): the document whose
  // `_123_` field the generator omits — a source field genuinely named
  // "_123_", hash 3550129612 — is omitted here too, and a declaration beside
  // it still loads.
  const generatorTwin = schemaFromContract(
    document(
      [
        { kind: "record", fields: [{ id: 3_550_129_612, type: 1 }] },
        primitive("nat8"),
        { kind: "record", fields: [{ id: 1, type: 1 }] },
      ],
      [
        { name: "Holder", type: 0 },
        { name: "Other", type: 2 },
      ],
    ),
    { names: [[0, 3_550_129_612, "_123_"]] },
  );
  assert(generatorTwin.ok);
  if (generatorTwin.ok) {
    assert.deepStrictEqual(Object.keys(generatorTwin.schemas), ["Other"]);
    assert.deepStrictEqual(generatorTwin.omitted, [
      { kind: "declaration", name: "Holder", reason: "reserved_field_name" },
    ]);
  }
  // The #115 collision document — `_123_` the name beside `123` the id —
  // no longer reaches the duplicate-key check: the reserved name never
  // renders a key, so the node is omitted rather than refused.
  const collision = schemaFromContract(
    document(
      [
        {
          kind: "record",
          fields: [
            { id: 123, type: 1 },
            { id: 3_550_129_612, type: 1 },
          ],
        },
        primitive("nat8"),
      ],
      [{ name: "T", type: 0 }],
    ),
    { names: [[0, 3_550_129_612, "_123_"]] },
  );
  assert(collision.ok);
  if (collision.ok) {
    assert.deepStrictEqual(collision.omitted, [
      { kind: "declaration", name: "T", reason: "reserved_field_name" },
    ]);
  }
  // An entry that names no rendered key — here a container that is not a
  // record at all — is ignored, as the generator ignores it.
  const unused = schemaFromContract(document([primitive("nat")], [{ name: "N", type: 0 }]), {
    names: [[0, 4_735_500, "_2_"]],
  });
  assert(unused.ok);
  if (unused.ok) {
    assert.deepStrictEqual(Object.keys(unused.schemas), ["N"]);
    assert.deepStrictEqual(unused.omitted, []);
  }
});

// Two honest spellings can share one Candid hash — `_0_` and `` 6,/`U``
// both hash to 4735054 — so a table may address one `(container, id)` twice.
// The last entry wins, as in `TsNames`, and only the winner is classified:
// `tests/golden.rs::the_last_name_for_a_key_wins_before_it_is_classified`
// pins the generator's two outcomes for the same table in both orders
// (PR #210 review).
test("the last name for a key wins before it is classified, in generator parity", () => {
  const reserved = "_0_";
  const collision = " 6,/`U";
  const id = candidLabelHash(reserved);
  assert.strictEqual(id, 4_735_054);
  assert.strictEqual(candidLabelHash(collision), id);
  const doc = document(
    [{ kind: "record", fields: [{ id, type: 1 }] }, primitive("nat")],
    [{ name: "A", type: 0 }],
  );

  // Reserved first, ordinary last: the ordinary spelling wins and renders.
  const ordinaryWins = schemaFromContract(doc, {
    names: [
      [0, id, reserved],
      [0, id, collision],
    ],
  });
  assert(ordinaryWins.ok);
  if (ordinaryWins.ok) {
    assert.deepStrictEqual(ordinaryWins.omitted, []);
    assert.deepStrictEqual(validate(ordinaryWins.schemas.A, { [collision]: 1n }), { ok: true });
  }

  // Ordinary first, reserved last: the reserved spelling wins and omits.
  const reservedWins = schemaFromContract(doc, {
    names: [
      [0, id, collision],
      [0, id, reserved],
    ],
  });
  assert(reservedWins.ok);
  if (reservedWins.ok) {
    assert.deepStrictEqual(Object.keys(reservedWins.schemas), []);
    assert.deepStrictEqual(reservedWins.omitted, [
      { kind: "declaration", name: "A", reason: "reserved_field_name" },
    ]);
  }
});

test("a malformed name table entry fails closed", () => {
  failsWith(
    schemaFromContract(document([primitive("nat")], [{ name: "A", type: 0 }]), {
      names: [[0, 1]] as unknown as [number, number, string][],
    }),
    "invalid_name_table",
    "$.names[0]",
  );
});

test("the arena size cap fails closed with the resource triple", () => {
  const result = schemaFromContract(document([primitive("nat"), primitive("nat")], []), {
    maxTypeNodes: 1,
  });
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues[0].code, "resource_limit_exceeded");
    assert.deepStrictEqual(result.issues[0].resource_limit, {
      resource: "type_nodes",
      limit: 1,
      observed: 2,
    });
  }
});

test("the total field cap fails closed with the resource triple", () => {
  const result = schemaFromContract(
    document(
      [
        {
          kind: "record",
          fields: [
            { id: 1, type: 1 },
            { id: 2, type: 1 },
          ],
        },
        primitive("nat"),
      ],
      [{ name: "A", type: 0 }],
    ),
    { maxFields: 1 },
  );
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues[0].code, "resource_limit_exceeded");
    assert.deepStrictEqual(result.issues[0].resource_limit, {
      resource: "fields",
      limit: 1,
      observed: 2,
    });
  }
});

test("an adversarially deep type chain builds and validates without overflow", () => {
  // 50_000 nested vecs around nat (vec has no collapsing rule, so the chain
  // is legal) — construction must be O(1) deep via the lazy thunks, and
  // validation must fail closed at the depth limit rather than overflow.
  const types: Json[] = [];
  const chain = 50_000;
  for (let i = 0; i < chain; i += 1) {
    types.push({ kind: "vec", inner: i + 1 });
  }
  types.push(primitive("nat"));
  const result = schemaFromContract(document(types, [{ name: "Deep", type: 0 }]));
  assert(result.ok, "construction must not recurse over the chain");
  if (result.ok) {
    // A value deep enough to reach the schema's depth limit: nested arrays.
    let value: unknown = [];
    for (let i = 0; i < 400; i += 1) {
      value = [value];
    }
    const outcome = validate(result.schemas.Deep, value);
    assert(!outcome.ok);
    if (!outcome.ok) {
      assert.strictEqual(outcome.issues[0].code, "resource_limit_exceeded");
    }
  }
});

test("a field, arm, or declaration named __proto__ is an ordinary key", () => {
  // On a plain object, assigning "__proto__" invokes the inherited prototype
  // setter and silently drops the entry — for records that was fail-open:
  // validate(schema, {}) reported ok for a schema with a required field. The
  // builders use null-prototype maps, so the name is just a key.
  // The honest id for the name: hash enforcement (issue #103) refuses a
  // table entry whose name does not hash back to its id.
  const protoId = 2_111_641_832;
  const recordDoc = document(
    [primitive("nat8"), { kind: "record", fields: [{ id: protoId, type: 0 }] }],
    [{ name: "R", type: 1 }],
  );
  const record = schemaFromContract(recordDoc, { names: [[1, protoId, "__proto__"]] });
  assert(record.ok);
  if (record.ok) {
    const empty = validate(record.schemas.R, {});
    assert(!empty.ok, "the __proto__ field is required");
    if (!empty.ok) {
      assert.deepStrictEqual(
        empty.issues.map((issue) => [issue.code, issue.path]),
        [["missing_field", "$.__proto__"]],
      );
    }
    // JSON.parse creates an own enumerable "__proto__" data property — the
    // exact domain value this schema describes.
    assert.deepStrictEqual(validate(record.schemas.R, JSON.parse('{"__proto__": 5}')), {
      ok: true,
    });
  }

  const variantDoc = document(
    [primitive("nat8"), { kind: "variant", fields: [{ id: protoId, type: 0 }] }],
    [{ name: "V", type: 1 }],
  );
  const variant = schemaFromContract(variantDoc, { names: [[1, protoId, "__proto__"]] });
  assert(variant.ok);
  if (variant.ok) {
    assert.deepStrictEqual(validate(variant.schemas.V, { tag: "__proto__", value: 5 }), {
      ok: true,
    });
  }

  const declarationDoc = document([primitive("nat")], [{ name: "__proto__", type: 0 }]);
  const declaration = schemaFromContract(declarationDoc);
  assert(declaration.ok);
  if (declaration.ok) {
    assert.deepStrictEqual(Object.keys(declaration.schemas), ["__proto__"]);
    assert.deepStrictEqual(validate(declaration.schemas["__proto__"], 1n), { ok: true });
  }
});

test("the declaration count cap fails closed with the resource triple", () => {
  const declarations: Json[] = [];
  for (let i = 0; i < 3; i += 1) {
    declarations.push({ name: `D${i}`, type: 0 });
  }
  const result = schemaFromContract(document([primitive("nat")], declarations), {
    maxDeclarations: 2,
  });
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues[0].code, "resource_limit_exceeded");
    assert.deepStrictEqual(result.issues[0].resource_limit, {
      resource: "declarations",
      limit: 2,
      observed: 3,
    });
  }
});

test("an oversized name table fails closed instead of amplifying issues", () => {
  const names = new Array(500_001).fill([0, 0, "x"]) as [number, number, string][];
  const result = schemaFromContract(document([primitive("nat")], [{ name: "A", type: 0 }]), {
    names,
  });
  assert(!result.ok);
  if (!result.ok) {
    assert.strictEqual(result.issues.length, 1);
    assert.strictEqual(result.issues[0].code, "resource_limit_exceeded");
    assert.strictEqual(result.issues[0].resource_limit?.resource, "name_table_entries");
  }
});

test("a document that throws while inspected fails closed", () => {
  const hostile = {
    get format(): string {
      throw new Error("boom");
    },
  };
  const result = schemaFromContract(hostile);
  assert(!result.ok);
  if (!result.ok) {
    assert.deepStrictEqual(
      result.issues.map((issue) => [issue.code, issue.path]),
      [["invalid_contract_document", "$"]],
    );
  }
  const proxied = schemaFromContract(
    new Proxy(
      {},
      {
        get() {
          throw new Error("boom");
        },
      },
    ),
  );
  assert(!proxied.ok);
});

test("tuple-shaped records build positionally; a lying table still fails at the table", () => {
  // The generator renders tuple elements positionally and never consults the
  // name table. With hash enforcement (issue #103) an honest table cannot
  // name tuple positions at all — a name for id 0 would have to hash to 0 —
  // so the positional build needs no entries, and entries that lie fail
  // closed before any node is considered.
  const doc = document(
    [
      {
        kind: "record",
        fields: [
          { id: 0, type: 1 },
          { id: 1, type: 2 },
        ],
      },
      primitive("nat"),
      primitive("text"),
    ],
    [{ name: "Pair", type: 0 }],
  );
  const bare = schemaFromContract(doc);
  assert(bare.ok, "tuple-shaped records build with no name entries");
  if (bare.ok) {
    assert.deepStrictEqual(validate(bare.schemas.Pair, [1n, "x"]), { ok: true });
  }
  const lying = schemaFromContract(doc, { names: [[0, 0, "same"]] });
  failsWith(lying, "invalid_name_table", "$.names[0]");
});

test("a nat8 vec is a blob whatever its element type is called (issue #191)", () => {
  // `type Byte = nat8; type Bytes = vec Byte` is `vec nat8`, which Candid
  // calls `blob`: the declared element name changes nothing about the value
  // domain, so the dynamic schema expects a Uint8Array exactly as the
  // generator's `$.c.blob()` does. Before #191 a declaration of the element
  // turned every blob in the interface into a number array.
  const doc = document(
    [primitive("nat8"), { kind: "vec", inner: 0 }],
    [
      { name: "Byte", type: 0 },
      { name: "Bytes", type: 1 },
    ],
  );
  const result = schemaFromContract(doc);
  assert(result.ok);
  if (result.ok) {
    assert.deepStrictEqual(validate(result.schemas.Bytes, new Uint8Array([1, 2])), { ok: true });
    const numbers = validate(result.schemas.Bytes, [1, 2]);
    assert(!numbers.ok, "a number array is not a blob");
    assert.deepStrictEqual(validate(result.schemas.Byte, 7), { ok: true });
  }
});

test("a func nested under opt or vec builds too (issue #104)", () => {
  const funcValue = { principal: "aaaaa-aa", method: "go" };
  for (const kind of ["opt", "vec"] as const) {
    const result = schemaFromContract(
      document(
        [
          { kind, inner: 1 },
          { kind: "func", args: [], results: [], mode: "query" },
        ],
        [{ name: "Holder", type: 0 }],
      ),
    );
    assert(result.ok, `${kind} of func is constructible since #104`);
    if (result.ok) {
      const sample = kind === "opt" ? funcValue : [funcValue];
      assert.deepStrictEqual(validate(result.schemas.Holder, sample), { ok: true });
    }
  }
});

test("numeric ids starting at 0 but not contiguous build a record, not a tuple", () => {
  // Candid `record { 0 : nat; 5 : text }` is tuple-like only in its first
  // field; the generator's is_tuple_shaped demands ids exactly 0..n-1.
  const doc = document(
    [
      {
        kind: "record",
        fields: [
          { id: 0, type: 1 },
          { id: 5, type: 2 },
        ],
      },
      primitive("nat"),
      primitive("text"),
    ],
    [{ name: "Sparse", type: 0 }],
  );
  const result = schemaFromContract(doc);
  assert(result.ok);
  if (result.ok) {
    assert.deepStrictEqual(validate(result.schemas.Sparse, { _0_: 1n, _5_: "x" }), { ok: true });
    const asTuple = validate(result.schemas.Sparse, [1n, "x"]);
    assert(!asTuple.ok, "a sparse-id record is not a tuple");
  }
});

test("declaration names that no generated module can bind are omitted in parity", () => {
  // Reserved words are identifier names: `delete` binds `$delete` in a
  // generated module (issue #188) and loads here as a plain key.
  const keyword = schemaFromContract(document([primitive("nat")], [{ name: "delete", type: 0 }]));
  assert(keyword.ok);
  if (keyword.ok) {
    assert.deepStrictEqual(validate(keyword.schemas.delete, 1n), { ok: true });
    assert.deepStrictEqual(keyword.omitted, []);
  }
  // A name that is not identifier-shaped used to load here as a deliberate
  // divergence from the generator. Since issue #189 both leave it out, with
  // what references it, so the loaded set is the generated set.
  const result = schemaFromContract(
    document(
      [
        { kind: "record", fields: [{ id: 1, type: 1 }] },
        { kind: "record", fields: [{ id: 2, type: 2 }] },
        primitive("nat"),
      ],
      [
        { name: "a-b", type: 1 },
        { name: "Uses", type: 0 },
        { name: "has space", type: 2 },
      ],
    ),
  );
  assert(result.ok);
  if (result.ok) {
    assert.deepStrictEqual(Object.keys(result.schemas), []);
    assert.deepStrictEqual(result.omitted, [
      { kind: "declaration", name: "Uses", reason: "references_omitted", via: "a-b" },
      { kind: "declaration", name: "a-b", reason: "invalid_declaration_name" },
      { kind: "declaration", name: "has space", reason: "invalid_declaration_name" },
    ]);
  }
});

test("omission follows every edge to the containing declaration; the actor drops methods", () => {
  // types: 0 Bad = variant { _1_ (reserved) }, 1 nat, 2 Holder = record { svc },
  // 3 service { f : 4 }, 4 func (Bad) -> (), 5 Good = record { a : nat },
  // 6 func (Good) -> (), 7 the actor service { bad : 4; ok : 6 },
  // 8 actor-named record, 9 func (8) -> (), 10 NoValue = opt empty,
  // 11 empty, 12 Ambiguous = variant { a : NoValue }.
  const types = [
    { kind: "variant", fields: [{ id: 3_550_129_612, type: 1 }] },
    primitive("nat"),
    { kind: "record", fields: [{ id: 5, type: 3 }] },
    { kind: "service", methods: [{ name: "f", id: candidLabelHash("f"), function: 4 }] },
    { kind: "func", args: [0], results: [], mode: "update" },
    { kind: "record", fields: [{ id: 6, type: 1 }] },
    { kind: "func", args: [5], results: [], mode: "query" },
    {
      kind: "service",
      methods: [
        { name: "bad", id: candidLabelHash("bad"), function: 4 },
        { name: "ok", id: candidLabelHash("ok"), function: 6 },
        { name: "who", id: candidLabelHash("who"), function: 9 },
      ],
    },
    { kind: "record", fields: [{ id: 7, type: 1 }] },
    { kind: "func", args: [8], results: [], mode: "update" },
    { kind: "opt", inner: 11 },
    primitive("empty"),
    { kind: "variant", fields: [{ id: 8, type: 10 }] },
  ];
  const result = schemaFromContract(
    document(
      types,
      [
        { name: "Ambiguous", type: 12 },
        { name: "Bad", type: 0 },
        { name: "Good", type: 5 },
        { name: "Holder", type: 2 },
        { name: "NoValue", type: 10 },
        { name: "actor", type: 8 },
      ],
      { kind: "service", service: 7 },
    ),
    { names: [[0, 3_550_129_612, "_123_"]] },
  );
  assert(result.ok, `expected success: ${JSON.stringify(codesOf(result))}`);
  if (!result.ok) {
    return;
  }
  // The nested service omits its containing declaration, not just `f`.
  assert.deepStrictEqual(Object.keys(result.schemas), ["Good", "NoValue"]);
  assert.deepStrictEqual(result.omitted, [
    { kind: "declaration", name: "Ambiguous", reason: "ambiguous_variant_arm" },
    { kind: "declaration", name: "Bad", reason: "reserved_field_name" },
    { kind: "declaration", name: "Holder", reason: "references_omitted", via: "Bad" },
    { kind: "declaration", name: "actor", reason: "reserved_export_name" },
    { kind: "method", name: "bad", reason: "references_omitted", via: "Bad" },
    { kind: "method", name: "who", reason: "references_omitted", via: "actor" },
  ]);
  const actor = result.actor;
  if (actor === undefined) {
    throw new Error("the actor itself is never omitted");
  }
  assert.deepStrictEqual([...serviceMethods(actor).keys()], ["ok"]);
});

test("reference structural constraints fail closed (issue #104 review)", () => {
  // A service method must denote a func type.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "service", methods: [{ name: "ping", id: 1247277682, function: 1 }] },
          primitive("nat"),
        ],
        [{ name: "R", type: 0 }],
      ),
    ),
    "invalid_contract_document",
    "$.types[0].methods[0].function",
  );
  // A method id must be the Candid hash of its name.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "service", methods: [{ name: "ping", id: 1, function: 1 }] },
          { kind: "func", args: [], results: [], mode: "update" },
        ],
        [{ name: "R", type: 0 }],
      ),
    ),
    "invalid_contract_document",
    "$.types[0].methods[0].id",
  );
  // A class must denote a service type.
  failsWith(
    schemaFromContract(
      document(
        [{ kind: "class", init: [], service: 1 }, primitive("nat")],
        [{ name: "C", type: 0 }],
      ),
    ),
    "invalid_contract_document",
    "$.types[0].service",
  );
  // The actor must be shaped { kind, service|class } and denote a service.
  failsWith(
    schemaFromContract(
      document([{ kind: "record", fields: [] }], [{ name: "U", type: 0 }], {
        kind: "record",
        record: 0,
      }),
    ),
    "invalid_contract_document",
    "$.actor",
  );
  failsWith(
    schemaFromContract(
      document([primitive("nat")], [{ name: "A", type: 0 }], {
        kind: "service",
        service: 0,
      }),
    ),
    "invalid_contract_document",
    "$.actor.service",
  );
});

test("a class actor denotes its running service; classes elsewhere are refused", () => {
  // Declarations naming the service and func nodes of a class-actor document
  // are legal on the Rust side; the class rules must not refuse them.
  const doc = document(
    [
      { kind: "class", init: [1], service: 2 },
      primitive("nat"),
      { kind: "service", methods: [{ name: "ping", id: 1247277682, function: 3 }] },
      { kind: "func", args: [], results: [], mode: "update" },
    ],
    [
      { name: "Running", type: 2 },
      { name: "Ping", type: 3 },
    ],
    { kind: "class", class: 0 },
  );
  const result = schemaFromContract(doc);
  assert(result.ok, "a canonical class-actor document loads");
  if (result.ok) {
    const principal = "aaaaa-aa";
    assert.deepStrictEqual(Object.keys(result.schemas), ["Running", "Ping"]);
    assert(result.actor !== undefined);
    if (result.actor !== undefined) {
      assert.deepStrictEqual(validate(result.actor, principal), { ok: true });
    }
  }
  // candid-core's class_not_actor_root rule, mirrored: a class anywhere but
  // the actor root — a declaration included — fails closed.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "class", init: [], service: 1 },
          { kind: "service", methods: [] },
        ],
        [{ name: "Main", type: 0 }],
      ),
    ),
    "invalid_contract_document",
    "$.types[0]",
  );
  // The declaration half of the rule has no actor-root exemption (issue
  // #129): a declaration naming the class node that IS the actor root is
  // refused at the declaration edge, as candid-core refuses it at
  // $.declarations[0].type. Without the dedicated declaration walk this
  // document loaded, because the exempted node itself is legal.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "class", init: [], service: 1 },
          { kind: "service", methods: [] },
        ],
        [{ name: "X", type: 0 }],
        { kind: "class", class: 0 },
      ),
    ),
    "invalid_contract_document",
    "$.declarations[0].type",
  );
  // The reported path carries the declaration's own index, not the first.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "class", init: [], service: 1 },
          { kind: "service", methods: [] },
        ],
        [
          { name: "Running", type: 1 },
          { name: "X", type: 0 },
        ],
        { kind: "class", class: 0 },
      ),
    ),
    "invalid_contract_document",
    "$.declarations[1].type",
  );
  // The exemption is the actor root's index, not "some actor exists": a
  // second class node in a class-actor document is refused even when no
  // declaration or type edge reaches it, as candid-core refuses it at
  // $.types[1].
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "class", init: [], service: 2 },
          { kind: "class", init: [], service: 2 },
          { kind: "service", methods: [] },
        ],
        [{ name: "Running", type: 2 }],
        { kind: "class", class: 0 },
      ),
    ),
    "invalid_contract_document",
    "$.types[1]",
  );
});

test("core-validator parity: oneway results, empty methods, class edges (PR #121 review)", () => {
  // oneway_has_results, mirrored.
  failsWith(
    schemaFromContract(
      document(
        [{ kind: "func", args: [], results: [1], mode: "oneway" }, primitive("nat")],
        [{ name: "F", type: 0 }],
      ),
    ),
    "invalid_contract_document",
    "$.types[0].results",
  );
  // empty_method_name, mirrored — hash("") is 0, so the hash check alone
  // would pass this document.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "service", methods: [{ name: "", id: 0, function: 1 }] },
          { kind: "func", args: [], results: [], mode: "update" },
        ],
        [{ name: "S", type: 0 }],
      ),
    ),
    "invalid_contract_document",
    "$.types[0].methods[0].name",
  );
  // class_not_first_class_type, mirrored: the actor-root exception must not
  // let a class become a first-class type via its own init...
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "class", init: [0], service: 1 },
          { kind: "service", methods: [] },
        ],
        [],
        { kind: "class", class: 0 },
      ),
    ),
    "invalid_contract_document",
    "$.types[0].init[0]",
  );
  // ...or via a func result reaching back to it.
  failsWith(
    schemaFromContract(
      document(
        [
          { kind: "class", init: [], service: 1 },
          {
            kind: "service",
            methods: [{ name: "make", id: 1213610478, function: 2 }],
          },
          { kind: "func", args: [], results: [0], mode: "update" },
        ],
        [],
        { kind: "class", class: 0 },
      ),
    ),
    "invalid_contract_document",
    "$.types[2].results[0]",
  );
});

// --- ContractEnvelope documents (issue #152) -------------------------------

/** The `$.extensions[…]` path base envelope-carried name issues report. */
const NAMES_BASE = `$.extensions[${JSON.stringify(FIELD_NAMES_EXTENSION)}]`;

/** A one-field record contract whose field id is the hash of `owner`. */
function ownerDocument(): Json {
  return document(
    [{ kind: "record", fields: [{ id: candidLabelHash("owner"), type: 1 }] }, primitive("nat")],
    [{ name: "A", type: 0 }],
  );
}

test("an envelope document is detected and its field names are consumed", () => {
  const built = schemaFromContract({
    contract: ownerDocument(),
    extensions: {
      [FIELD_NAMES_EXTENSION]: [[0, candidLabelHash("owner"), "owner"]],
    },
  });
  assert(built.ok, "the envelope must build");
  if (!built.ok) {
    return;
  }
  assert.deepStrictEqual(validate(built.schemas.A, { owner: 5n }), { ok: true });
  const missing = validate(built.schemas.A, {});
  assert(!missing.ok && missing.issues[0].path === "$.owner", "the envelope name must render");
});

test("an envelope without extensions builds with the _id_ rendering", () => {
  const built = schemaFromContract({ contract: ownerDocument() });
  assert(built.ok, "a bare envelope is legal");
  if (!built.ok) {
    return;
  }
  const key = `_${candidLabelHash("owner")}_`;
  assert.deepStrictEqual(validate(built.schemas.A, { [key]: 5n }), { ok: true });
});

test("an explicit names option wins over envelope-carried names", () => {
  const envelope: Json = {
    contract: ownerDocument(),
    extensions: {
      [FIELD_NAMES_EXTENSION]: [[0, candidLabelHash("owner"), "owner"]],
    },
  };
  // An explicit empty table: fields render by `_id_`, proving the envelope's
  // table was not consulted.
  const explicit = schemaFromContract(envelope, { names: [] });
  assert(explicit.ok, "the explicit table must be used");
  if (explicit.ok) {
    const key = `_${candidLabelHash("owner")}_`;
    assert.deepStrictEqual(validate(explicit.schemas.A, { [key]: 5n }), { ok: true });
    const named = validate(explicit.schemas.A, { owner: 5n });
    assert(!named.ok, "the envelope name must not render when an explicit table wins");
  }
  // Precedence means not consulted at all: a lying envelope table cannot
  // fail a build that supplies its own valid names.
  const lyingEnvelope: Json = {
    contract: ownerDocument(),
    extensions: { [FIELD_NAMES_EXTENSION]: [[0, 1, "lies"]] },
  };
  const overridden = schemaFromContract(lyingEnvelope, {
    names: [[0, candidLabelHash("owner"), "owner"]],
  });
  assert(overridden.ok, "an unused envelope table is not consulted");
});

test("envelope-carried names are hash-enforced exactly like caller-supplied ones", () => {
  // A name that does not hash back to its id fails closed at the extension.
  failsWith(
    schemaFromContract({
      contract: ownerDocument(),
      extensions: { [FIELD_NAMES_EXTENSION]: [[0, candidLabelHash("owner"), "lies"]] },
    }),
    "invalid_name_table",
    `${NAMES_BASE}[0]`,
  );
  // An `_N_`-shaped name that does not hash to its id lies, same rule as
  // the options table.
  failsWith(
    schemaFromContract({
      contract: ownerDocument(),
      extensions: { [FIELD_NAMES_EXTENSION]: [[0, candidLabelHash("owner"), "_2_"]] },
    }),
    "invalid_name_table",
    `${NAMES_BASE}[0]`,
  );
  // A malformed entry fails closed, path-addressed at the extension.
  failsWith(
    schemaFromContract({
      contract: ownerDocument(),
      extensions: { [FIELD_NAMES_EXTENSION]: [[0, 1]] },
    }),
    "invalid_name_table",
    `${NAMES_BASE}[0]`,
  );
  // A non-array extension value is not a name table at all.
  failsWith(
    schemaFromContract({
      contract: ownerDocument(),
      extensions: { [FIELD_NAMES_EXTENSION]: { 0: "owner" } },
    }),
    "invalid_name_table",
    NAMES_BASE,
  );
});

test("the envelope shell fails closed on malformed shapes", () => {
  // Unknown envelope keys are refused, mirroring the Rust loader's
  // deny_unknown_fields — including the hybrid that carries contract markers
  // beside a contract key.
  failsWith(
    schemaFromContract({ contract: ownerDocument(), format: "candid-core" }),
    "invalid_contract_document",
    "$",
  );
  // extensions must be a JSON object.
  for (const extensions of [null, [], "names", 5]) {
    failsWith(
      schemaFromContract({ contract: ownerDocument(), extensions }),
      "invalid_contract_document",
      "$.extensions",
    );
  }
  // A non-object contract value fails inside the contract, path re-rooted.
  failsWith(schemaFromContract({ contract: "nope" }), "invalid_contract_document", "$.contract");
});

test("extension names are validated by the Rust loader's grammar", () => {
  for (const name of [
    "unversioned",
    "nodot/v1",
    "org.candid_core.field-names/v1",
    "Org.Example/v1",
    "org.example/v0",
    "org.example/v01",
    "org..example/v1",
  ]) {
    failsWith(
      schemaFromContract({
        contract: ownerDocument(),
        extensions: { [name]: [] },
      }),
      "invalid_extension_name",
      "$.extensions",
    );
  }
  // Foreign extensions with valid names are tolerated and ignored: the map
  // is namespaced ecosystem metadata, not a closed set.
  const built = schemaFromContract({
    contract: ownerDocument(),
    extensions: {
      [FIELD_NAMES_EXTENSION]: [[0, candidLabelHash("owner"), "owner"]],
      "org.example.other/v2": { anything: true },
    },
  });
  assert(built.ok, "a foreign extension must not fail the build");
});

test("contract-side issues inside an envelope are rooted at $.contract", () => {
  failsWith(
    schemaFromContract({
      contract: { ...ownerDocument(), format: "not-candid-core" },
    }),
    "unsupported_contract_format",
    "$.contract.format",
  );
  failsWith(
    schemaFromContract({
      contract: document([{ kind: "opt", inner: 7 }], [{ name: "O", type: 0 }]),
    }),
    "dangling_type_ref",
    "$.contract.types[0].inner",
  );
});

test("an inherited extensions property is not envelope data", () => {
  // Own-key gating (review finding): `extensions` reached through the
  // prototype chain must be ignored exactly as the unknown-key walk ignores
  // it — the document behaves as a bare `{ contract }` envelope.
  const hostile = Object.assign(Object.create({ extensions: [] }), {
    contract: ownerDocument(),
  });
  const built = schemaFromContract(hostile);
  assert(built.ok, "inherited junk must not reach the shell checks");
  if (built.ok) {
    const key = `_${candidLabelHash("owner")}_`;
    assert.deepStrictEqual(validate(built.schemas.A, { [key]: 5n }), { ok: true });
  }
});

test("an envelope that throws while inspected fails closed", () => {
  const hostile = {
    contract: ownerDocument(),
    get extensions(): never {
      throw new Error("gotcha");
    },
  };
  failsWith(schemaFromContract(hostile), "invalid_contract_document", "$");
});
