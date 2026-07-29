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

test("a func nested in a supported type fails closed", () => {
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
  failsWith(result, "unsupported_construct", "$.types[0].fields[0].type");
});

test("top-level func and service declarations are skipped and reported", () => {
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
  assert(result.ok, "supported declarations must still build");
  if (result.ok) {
    assert.deepStrictEqual(Object.keys(result.schemas), ["Kept"]);
    assert.deepStrictEqual(result.deferred, [
      { name: "Callback", kind: "func" },
      { name: "Registry", kind: "service" },
    ]);
    assert.strictEqual(result.actorDeferred, true);
    assert.deepStrictEqual(validate(result.schemas.Kept, {}), { ok: true });
  }
});

test("an actorless document reports actorDeferred false", () => {
  const result = schemaFromContract(
    document([{ kind: "record", fields: [] }], [{ name: "Unit", type: 0 }]),
  );
  assert(result.ok);
  if (result.ok) {
    assert.strictEqual(result.actorDeferred, false);
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

test("colliding rendered field keys fail closed rather than dropping a field", () => {
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
  // Two different ids mapped to one supplied name.
  const named = schemaFromContract(doc, {
    names: [
      [0, 1, "same"],
      [0, 2, "same"],
    ],
  });
  failsWith(named, "duplicate_field_name", "$.types[0].fields[1]");
  // A supplied name colliding with an unnamed field's `_id_` rendering.
  const collided = schemaFromContract(doc, { names: [[0, 1, "_2_"]] });
  failsWith(collided, "duplicate_field_name", "$.types[0].fields[1]");
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
