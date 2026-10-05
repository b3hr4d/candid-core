// `schemaFromContract` refuses what `Contract::from_json` refuses (issue
// #228): unknown keys on every object the format defines
// (`deny_unknown_fields`), type nodes no root reaches (`orphan_type_node`),
// and a non-empty arena with no root at all (`rootless_type_arena`). Each
// case found by the #196 differential fuzz has its repro here, plus every
// position the unknown-key rule covers, the documents that must keep
// loading, and a document at the arena cap that must load without the new
// walk costing depth.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";

import {
  schemaFromContract,
  DEFAULT_MAX_TYPE_NODES,
  type ContractIssue,
  type SchemaFromContractResult,
} from "../contract.ts";
import { candidLabelHash } from "../labels.ts";

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

/** The issues of a refused document, as `[code, path]` pairs. */
function refusal(result: SchemaFromContractResult): [string, string][] {
  assert(!result.ok, "expected the document to be refused");
  if (result.ok) {
    throw new Error("unreachable");
  }
  return result.issues.map((issue: ContractIssue) => [issue.code, issue.path]);
}

function loads(result: SchemaFromContractResult): void {
  assert(
    result.ok,
    `expected the document to load: ${JSON.stringify(result.ok ? [] : result.issues)}`,
  );
}

/** `type T0 = nat;` as candid-core compiles it: the issue's repro base. */
const t0 = (): Json => document([primitive("nat")], [{ name: "T0", type: 0 }]);

/**
 * One valid document holding every node kind, every one reachable, with the
 * class as the actor root (the only place a class may stand):
 *
 *   0 nat · 1 opt 0 · 2 vec 0 · 3 record { 0: 0 } · 4 variant { 0: 0 }
 *   5 func (0) -> () query · 6 service { m: 5 } · 7 class (0) -> 6
 */
function everyKind(): Json {
  return document(
    [
      primitive("nat"),
      { kind: "opt", inner: 0 },
      { kind: "vec", inner: 0 },
      { kind: "record", fields: [{ id: 0, type: 0 }] },
      { kind: "variant", fields: [{ id: 0, type: 0 }] },
      { kind: "func", args: [0], results: [], mode: "query" },
      { kind: "service", methods: [{ name: "m", id: candidLabelHash("m"), function: 5 }] },
      { kind: "class", init: [0], service: 6 },
    ],
    [
      { name: "O", type: 1 },
      { name: "V", type: 2 },
      { name: "R", type: 3 },
      { name: "A", type: 4 },
      { name: "F", type: 5 },
      { name: "S", type: 6 },
    ],
    { kind: "class", class: 7 },
  );
}

// Case 6: unknown keys.

test("an unknown key on the root is refused at its path (issue #228 case 6)", () => {
  loads(schemaFromContract(t0()));
  assert.deepStrictEqual(refusal(schemaFromContract({ ...t0(), extra: false })), [
    ["unknown_key", "$.extra"],
  ]);
  // A key that is not identifier-shaped is bracketed, so the path names
  // exactly one position whatever the key's text. Issues come in the
  // object's own key order.
  assert.deepStrictEqual(refusal(schemaFromContract({ ...t0(), "x y": 1, "": 2 })), [
    ["unknown_key", '$["x y"]'],
    ["unknown_key", '$[""]'],
  ]);
  // A parsed `__proto__` key is an own key like any other, and is refused.
  assert.deepStrictEqual(
    refusal(schemaFromContract(JSON.parse(`{"__proto__": 1, ${JSON.stringify(t0()).slice(1)}`))),
    [["unknown_key", "$.__proto__"]],
  );
  // The root's optional keys are known keys, not extras: identities and
  // producer are carried by canonical documents (and not consulted).
  loads(schemaFromContract({ ...t0(), identities: {}, producer: {} }));
});

test("an unknown key on a type node of every kind is refused (issue #228 case 6)", () => {
  loads(schemaFromContract(everyKind()));
  // The issue's repro: `set ["types", 0, "extra"] = false` on `type T0 = nat;`.
  const repro = t0();
  repro.types[0].extra = false;
  assert.deepStrictEqual(refusal(schemaFromContract(repro)), [["unknown_key", "$.types[0].extra"]]);
  for (let index = 0; index < 8; index += 1) {
    const mutated = everyKind();
    mutated.types[index].extra = false;
    assert.deepStrictEqual(
      refusal(schemaFromContract(mutated)),
      [["unknown_key", `$.types[${index}].extra`]],
      `node ${index} (${mutated.types[index].kind})`,
    );
  }
  // A key that belongs to another kind is still unknown on this one.
  const crossed = everyKind();
  crossed.types[1].fields = [];
  assert.deepStrictEqual(refusal(schemaFromContract(crossed)), [
    ["unknown_key", "$.types[1].fields"],
  ]);
});

test("an unknown key on a field, arm, or method is refused (issue #228 case 6)", () => {
  for (const [index, path] of [
    [3, "$.types[3].fields[0].extra"],
    [4, "$.types[4].fields[0].extra"],
  ] as const) {
    const mutated = everyKind();
    mutated.types[index].fields[0].extra = false;
    assert.deepStrictEqual(refusal(schemaFromContract(mutated)), [["unknown_key", path]]);
  }
  const method = everyKind();
  method.types[6].methods[0].extra = false;
  assert.deepStrictEqual(refusal(schemaFromContract(method)), [
    ["unknown_key", "$.types[6].methods[0].extra"],
  ]);
});

test("an unknown key on a declaration or the actor is refused (issue #228 case 6)", () => {
  const declaration = t0();
  declaration.declarations[0].extra = false;
  assert.deepStrictEqual(refusal(schemaFromContract(declaration)), [
    ["unknown_key", "$.declarations[0].extra"],
  ]);
  const actor = everyKind();
  actor.actor.extra = false;
  assert.deepStrictEqual(refusal(schemaFromContract(actor)), [["unknown_key", "$.actor.extra"]]);
  // The actor's keys are closed per kind: a service actor carries no class.
  const service = document(
    [
      { kind: "func", args: [], results: [], mode: "update" },
      { kind: "service", methods: [{ name: "m", id: candidLabelHash("m"), function: 0 }] },
    ],
    [],
    { kind: "service", service: 1, class: 1 },
  );
  assert.deepStrictEqual(refusal(schemaFromContract(service)), [["unknown_key", "$.actor.class"]]);
});

test("unknown keys are all reported, beside the document's other defects", () => {
  const mutated = everyKind();
  mutated.types[0].b = 1;
  mutated.types[0].a = 2;
  mutated.types[2].inner = 99;
  mutated.declarations[1].note = "";
  assert.deepStrictEqual(refusal(schemaFromContract(mutated)), [
    ["unknown_key", "$.types[0].b"],
    ["unknown_key", "$.types[0].a"],
    ["dangling_type_ref", "$.types[2].inner"],
    ["unknown_key", "$.declarations[1].note"],
  ]);
});

test("unknown keys inside an envelope's contract are rooted at $.contract", () => {
  const contract = t0();
  contract.types[0].extra = false;
  assert.deepStrictEqual(refusal(schemaFromContract({ contract })), [
    ["unknown_key", "$.contract.types[0].extra"],
  ]);
  assert.deepStrictEqual(refusal(schemaFromContract({ contract: { ...t0(), "x y": 0 } })), [
    ["unknown_key", '$.contract["x y"]'],
  ]);
  // A Contract's own unknown root key `names` sits where a name table's
  // issues are reported (`$.names`), and is still the Contract's: rooted at
  // `$.contract.names`, while the name table's own issues keep their base.
  assert.deepStrictEqual(refusal(schemaFromContract({ contract: { ...t0(), names: [] } })), [
    ["unknown_key", "$.contract.names"],
  ]);
  assert.deepStrictEqual(
    refusal(
      schemaFromContract({ contract: { ...t0(), names: [] } }, { names: "not a table" as never }),
    ),
    [["unknown_key", "$.contract.names"]],
  );
  assert.deepStrictEqual(
    refusal(schemaFromContract({ contract: t0() }, { names: "not a table" as never })),
    [["invalid_name_table", "$.names"]],
  );
});

// Case 7: orphan type nodes.

test("a type node no root reaches is refused (issue #228 case 7)", () => {
  // The issue's repro: `insert ["types", 1] = text` on `type T0 = nat;`.
  const repro = t0();
  repro.types.push(primitive("text"));
  assert.deepStrictEqual(refusal(schemaFromContract(repro)), [["orphan_type_node", "$.types[1]"]]);
  // Reachability is from the roots, not in-degree: an unreached cycle is
  // orphaned whole, every node reported in arena order.
  const cycle = document(
    [primitive("nat"), { kind: "opt", inner: 2 }, { kind: "record", fields: [{ id: 0, type: 1 }] }],
    [{ name: "T0", type: 0 }],
  );
  assert.deepStrictEqual(refusal(schemaFromContract(cycle)), [
    ["orphan_type_node", "$.types[1]"],
    ["orphan_type_node", "$.types[2]"],
  ]);
  // An actor root does not reach what only a dropped declaration reached.
  const actorOnly = everyKind();
  actorOnly.declarations = [];
  assert.deepStrictEqual(refusal(schemaFromContract(actorOnly)), [
    ["orphan_type_node", "$.types[1]"],
    ["orphan_type_node", "$.types[2]"],
    ["orphan_type_node", "$.types[3]"],
    ["orphan_type_node", "$.types[4]"],
  ]);
});

test("every edge reaches, the actor class's init included (issue #228 case 7)", () => {
  // Nodes reached only through the actor: the service's func, its result
  // (node 5, which only the func's `results` edge reaches), and the class's
  // init argument (the record), which no schema reads.
  loads(
    schemaFromContract(
      document(
        [
          { kind: "record", fields: [{ id: 1, type: 4 }] },
          { kind: "func", args: [4], results: [5], mode: "query" },
          { kind: "service", methods: [{ name: "m", id: candidLabelHash("m"), function: 1 }] },
          { kind: "class", init: [0], service: 2 },
          primitive("nat"),
          primitive("text"),
        ],
        [],
        { kind: "class", class: 3 },
      ),
    ),
  );
  // Nodes reached only through a nested func and service edge.
  loads(
    schemaFromContract(
      document(
        [
          { kind: "vec", inner: 1 },
          { kind: "service", methods: [{ name: "f", id: candidLabelHash("f"), function: 2 }] },
          { kind: "func", args: [3], results: [], mode: "oneway" },
          primitive("text"),
        ],
        [{ name: "Services", type: 0 }],
      ),
    ),
  );
});

// Case 8: a rootless arena.

test("a non-empty arena with no root is refused (issue #228 case 8)", () => {
  // The issue's repro: `set ["declarations"] = []` on `type T0 = nat;`.
  const repro = t0();
  repro.declarations = [];
  assert.deepStrictEqual(refusal(schemaFromContract(repro)), [["rootless_type_arena", "$.types"]]);
  // Reported once, not once per node, exactly as candid-core reports it.
  assert.deepStrictEqual(
    refusal(schemaFromContract(document([primitive("nat"), { kind: "opt", inner: 0 }], []))),
    [["rootless_type_arena", "$.types"]],
  );
  // Inside an envelope, rooted at $.contract.
  assert.deepStrictEqual(refusal(schemaFromContract({ contract: repro })), [
    ["rootless_type_arena", "$.contract.types"],
  ]);
});

test("an empty arena needs no root and still loads, as candid-core loads it", () => {
  const result = schemaFromContract(document([], []));
  loads(result);
  if (result.ok) {
    assert.deepStrictEqual(Object.keys(result.schemas), []);
    assert.strictEqual(result.actor, undefined);
  }
});

// Every Contract the compiler writes still loads.

test("every golden and fixture Contract the compiler wrote still loads", () => {
  const repository = new URL("../../../../", import.meta.url);
  // Each directory with the names in it that hold a compiler-written Contract
  // (the rest are name tables, sources, and other artifacts), and where in
  // each document the Contract or its envelope sits.
  const whole = (parsed: Json): Json => parsed;
  const member =
    (key: string) =>
    (parsed: Json): Json =>
      parsed[key];
  const sources: readonly (readonly [string, RegExp, (parsed: Json) => Json])[] = [
    ["crates/candid-core-ts/tests/goldens/", /\.(contract|envelope)\.json$/, whole],
    ["crates/candid-core-ts/tests/goldens/depth/", /\.envelope\.json$/, whole],
    ["crates/candid-core-ts/tests/goldens/wire/", /^coercion\.json$/, member("envelope")],
    ["fuzz/seeds/canonicalization/", /\.json$/, whole],
    ["fuzz/seeds/contract_json/", /\.json$/, whole],
    ["fuzz/seeds/envelope_json/", /\.json$/, whole],
    ["fuzz/seeds/provenance/", /\.json$/, member("contract")],
    ["tests/fixtures/artifact-identity/artifacts/", /^contract.*\.json$/, whole],
    ["tests/fixtures/artifact-identity/artifacts/", /^compilation.*\.json$/, member("contract")],
    ["tests/fixtures/conformance/", /\.contract\.json$/, whole],
    ["tests/fixtures/envelope/", /\.envelope\.json$/, whole],
  ];
  let loaded = 0;
  for (const [directory, pattern, contract] of sources) {
    const base = new URL(directory, repository);
    for (const name of readdirSync(base).filter((entry) => pattern.test(entry))) {
      const parsed: Json = JSON.parse(readFileSync(new URL(name, base), "utf8"));
      const result = schemaFromContract(contract(parsed));
      assert(
        result.ok,
        `${directory}${name}: ${JSON.stringify(result.ok ? [] : result.issues.slice(0, 3))}`,
      );
      loaded += 1;
    }
  }
  // 30 goldens, 21 fuzz seeds, 15 fixtures today: a vanished directory or a
  // pattern that stopped matching must not pass this test vacuously.
  assert(loaded >= 66, `only ${loaded} documents loaded`);
});

// The walk is iterative and linear: a document at the arena cap loads.

test("a document at the arena cap, deep and wide, loads without recursion", (t) => {
  // Half the arena is one chain of 50_000 nested vecs (a recursive walk would
  // overflow the stack long before its end), and half is one record with a
  // field per remaining node, each a distinct leaf — 100_000 nodes, every one
  // reached only through the chain or the fan-out.
  const half = DEFAULT_MAX_TYPE_NODES / 2;
  const types: Json[] = [];
  for (let i = 0; i < half - 1; i += 1) {
    types.push({ kind: "vec", inner: i + 1 });
  }
  types.push(primitive("nat"));
  const fields: Json[] = [];
  for (let i = 0; i < half - 1; i += 1) {
    fields.push({ id: 2 * i + 1, type: half + 1 + i });
  }
  types.push({ kind: "record", fields });
  for (let i = 0; i < half - 1; i += 1) {
    types.push(primitive(i % 2 === 0 ? "text" : "bool"));
  }
  assert.strictEqual(types.length, DEFAULT_MAX_TYPE_NODES);
  const doc = document(types, [
    { name: "Deep", type: 0 },
    { name: "Wide", type: half },
  ]);
  const started = performance.now();
  const result = schemaFromContract(doc);
  const elapsed = performance.now() - started;
  loads(result);
  // Informational only: no timing ever fails a run (recorded on #39).
  t.diagnostic(`100_000-node document loaded in ${elapsed.toFixed(1)} ms (informational)`);
  // The same document with its last leaf unreached reports exactly that node.
  doc.types[half].fields.pop();
  assert.deepStrictEqual(refusal(schemaFromContract(doc)), [
    ["orphan_type_node", `$.types[${DEFAULT_MAX_TYPE_NODES - 1}]`],
  ]);
});
