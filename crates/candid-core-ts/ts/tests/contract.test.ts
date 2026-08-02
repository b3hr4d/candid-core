// `schemaFromContract` construction tests: the fail-closed paths, the
// collapsing-opt rule in all four recorded forms, deferred handling, and the
// bounded-input guards. The golden cross-check in crosscheck.test.ts covers
// the happy paths against the generated builders.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  schemaFromContract,
  type ContractIssueCode,
  type SchemaFromContractResult,
} from "../contract.ts";
import { validate } from "../validate.ts";

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

function failsWith(
  result: SchemaFromContractResult,
  code: ContractIssueCode,
  path: string,
): void {
  assert(!result.ok, "expected schema construction to fail");
  if (result.ok) {
    throw new Error("unreachable");
  }
  assert.strictEqual(result.issues.length, 1);
  assert.strictEqual(result.issues[0].code, code);
  assert.strictEqual(result.issues[0].path, path);
}

test("collapsing opts are rejected at construction: opt opt", () => {
  const result = schemaFromContract(
    document(
      [{ kind: "opt", inner: 1 }, { kind: "opt", inner: 2 }, primitive("nat")],
      [{ name: "DoubleOpt", type: 0 }],
    ),
  );
  failsWith(result, "unrepresentable_option", "$.types[0].inner");
});

test("collapsing opts are rejected at construction: opt null", () => {
  const result = schemaFromContract(
    document(
      [{ kind: "opt", inner: 1 }, primitive("null")],
      [{ name: "OptNull", type: 0 }],
    ),
  );
  failsWith(result, "unrepresentable_option", "$.types[0].inner");
});

test("collapsing opts are rejected at construction: opt reserved", () => {
  const result = schemaFromContract(
    document(
      [{ kind: "opt", inner: 1 }, primitive("reserved")],
      [{ name: "OptReserved", type: 0 }],
    ),
  );
  failsWith(result, "unrepresentable_option", "$.types[0].inner");
});

test("collapsing opts are rejected at construction: the aliased form", () => {
  // `type Inner = opt nat; type Outer = opt Inner` — the arena holds no alias
  // indirection, so Outer's inner node *is* the opt node, and the check is on
  // the node, not its spelling.
  const result = schemaFromContract(
    document(
      [{ kind: "opt", inner: 1 }, { kind: "opt", inner: 2 }, primitive("nat")],
      [
        { name: "Inner", type: 1 },
        { name: "Outer", type: 0 },
      ],
    ),
  );
  failsWith(result, "unrepresentable_option", "$.types[0].inner");
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
      _1_: { principal: { toText: () => "aaaaa-aa" }, method: "go" },
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
    const funcValue = { principal: { toText: () => "aaaaa-aa" }, method: "go" };
    assert.deepStrictEqual(validate(result.schemas.Callback, funcValue), { ok: true });
    assert.deepStrictEqual(
      validate(result.schemas.Registry, { toText: () => "aaaaa-aa" }),
      { ok: true },
    );
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
  failsWith(
    schemaFromContract(document([42], [])),
    "invalid_contract_document",
    "$.types[0]",
  );
});

test("declaration defects reuse candid-core's codes", () => {
  failsWith(
    schemaFromContract(document([primitive("nat")], [{ name: "", type: 0 }])),
    "empty_declaration_name",
    "$.declarations[0].name",
  );
  failsWith(
    schemaFromContract(
      document([primitive("nat")], [
        { name: "A", type: 0 },
        { name: "A", type: 0 },
      ]),
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
  // `_N_`-shaped names are reserved for the numeric-id rendering: erased to
  // a key, `_2_` the name and `_2_` the rendering of id 2 are identical.
  const reserved = schemaFromContract(doc, { names: [[0, 1, "_2_"]] });
  failsWith(reserved, "invalid_name_table", "$.names[0]");
  // The shape refusal must hold even when the hash is honest: "_2_" hashes
  // to 4735500, so this entry passes the hash check and only the reserved
  // shape stands between the codec and a silently wrong wire id.
  const honestHash = schemaFromContract(
    document(
      [{ kind: "record", fields: [{ id: 4_735_500, type: 1 }] }, primitive("nat")],
      [{ name: "A", type: 0 }],
    ),
    { names: [[0, 4_735_500, "_2_"]] },
  );
  failsWith(honestHash, "invalid_name_table", "$.names[0]");
  // Cross-path agreement with the generator (#115): the document its
  // ReservedFieldName refuses — a source field genuinely named "_123_",
  // hash 3550129612 — is refused here too, by the same reservation.
  const generatorTwin = schemaFromContract(
    document(
      [{ kind: "record", fields: [{ id: 3_550_129_612, type: 1 }] }, primitive("nat8")],
      [{ name: "Holder", type: 0 }],
    ),
    { names: [[0, 3_550_129_612, "_123_"]] },
  );
  failsWith(generatorTwin, "invalid_name_table", "$.names[0]");
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
  const result = schemaFromContract(
    document([primitive("nat"), primitive("nat")], []),
    { maxTypeNodes: 1 },
  );
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
    assert.deepStrictEqual(
      validate(record.schemas.R, JSON.parse('{"__proto__": 5}')),
      { ok: true },
    );
  }

  const variantDoc = document(
    [primitive("nat8"), { kind: "variant", fields: [{ id: protoId, type: 0 }] }],
    [{ name: "V", type: 1 }],
  );
  const variant = schemaFromContract(variantDoc, { names: [[1, protoId, "__proto__"]] });
  assert(variant.ok);
  if (variant.ok) {
    assert.deepStrictEqual(
      validate(variant.schemas.V, { tag: "__proto__", value: 5 }),
      { ok: true },
    );
  }

  const declarationDoc = document(
    [primitive("nat")],
    [{ name: "__proto__", type: 0 }],
  );
  const declaration = schemaFromContract(declarationDoc);
  assert(declaration.ok);
  if (declaration.ok) {
    assert.deepStrictEqual(Object.keys(declaration.schemas), ["__proto__"]);
    assert.deepStrictEqual(
      validate(declaration.schemas["__proto__"], 1n),
      { ok: true },
    );
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
  const result = schemaFromContract(
    document([primitive("nat")], [{ name: "A", type: 0 }]),
    { names },
  );
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
    new Proxy({}, {
      get() {
        throw new Error("boom");
      },
    }),
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

test("a nat8 vec whose element type is declared by name stays a vec", () => {
  // The blob rule applies to the *anonymous* nat8 node only: `type Byte =
  // nat8; type Bytes = vec Byte` is a deliberate abstraction and renders
  // c.vec(Byte) in the generator, so the dynamic schema must expect number
  // arrays, not Uint8Array.
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
    assert.deepStrictEqual(validate(result.schemas.Bytes, [1, 2]), { ok: true });
    const blobValue = validate(result.schemas.Bytes, new Uint8Array([1, 2]));
    assert(!blobValue.ok, "a declared element type is not a blob");
  }
});

test("a func nested under opt or vec builds too (issue #104)", () => {
  const funcValue = { principal: { toText: () => "aaaaa-aa" }, method: "go" };
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
    assert.deepStrictEqual(
      validate(result.schemas.Sparse, { _0_: 1n, _5_: "x" }),
      { ok: true },
    );
    const asTuple = validate(result.schemas.Sparse, [1n, "x"]);
    assert(!asTuple.ok, "a sparse-id record is not a tuple");
  }
});

test("declaration names need not be TypeScript identifiers", () => {
  // A deliberate divergence from the generator, which emits source text and
  // must refuse `type delete`; a map key has no such constraint.
  const result = schemaFromContract(
    document([primitive("nat")], [{ name: "delete", type: 0 }]),
  );
  assert(result.ok);
  if (result.ok) {
    assert.deepStrictEqual(validate(result.schemas.delete, 1n), { ok: true });
  }
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
    const principal = { toText: () => "aaaaa-aa" };
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
