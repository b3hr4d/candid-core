// The round-trip acceptance criterion of issue #102: for every golden
// fixture, the schema built dynamically from the checked-in Contract JSON
// document must validate exactly the values the generated builder describes —
// same verdicts, same issue codes, same paths, same messages, asserted by
// deep equality on the whole `validate` result.
//
// The Contract JSON and name-table documents are goldens emitted by the Rust
// side (`tests/golden.rs`, `UPDATE_GOLDENS=1`) from the same fixtures the
// generated modules come from, so the two schemas under comparison share one
// source of truth. Samples deliberately stay far from the depth limit: the
// dynamic schema carries one extra `rec` hop per edge, so near-limit values
// would diverge on the resource issue alone — the depth behavior itself is
// pinned in validate.test.ts.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { schemaFromContract, type FieldNameEntry } from "../contract.ts";
import { validate } from "../validate.ts";
import type { AnySchema } from "../schema.ts";

import * as primitives from "../../tests/goldens/primitives.ts";
import * as collections from "../../tests/goldens/collections.ts";
import * as variants from "../../tests/goldens/variants.ts";
import * as recursion from "../../tests/goldens/recursion.ts";
import * as quoting from "../../tests/goldens/quoting.ts";
import * as deferred from "../../tests/goldens/deferred.ts";
import * as proto from "../../tests/goldens/proto.ts";

interface Fixture {
  readonly name: string;
  readonly module: Record<string, unknown>;
  /** Sample values per declaration: valid and invalid alike. */
  readonly samples: Record<string, readonly unknown[]>;
  readonly deferredNames?: readonly string[];
}

function load(name: string): { contract: unknown; names: FieldNameEntry[] } {
  const goldens = new URL("../../tests/goldens/", import.meta.url);
  return {
    contract: JSON.parse(
      readFileSync(new URL(`${name}.contract.json`, goldens), "utf8"),
    ),
    names: JSON.parse(
      readFileSync(new URL(`${name}.names.json`, goldens), "utf8"),
    ) as FieldNameEntry[],
  };
}

const principal = { toText: () => "aaaa-aa" };

const item = {
  id: 7,
  label: "seven",
  payload: new Uint8Array([7]),
};

const FIXTURES: readonly Fixture[] = [
  {
    name: "primitives",
    module: primitives,
    samples: {
      P00: [null, 0, undefined],
      P01: [true, 1],
      P02: [0n, 2n ** 100n, -1n, 5],
      P03: [-5n, 0n, 5],
      P04: [0, 255, 256, -1, 1.5, 1n],
      P05: [65_535, 65_536],
      P06: [4_294_967_295, 4_294_967_296],
      P07: [0n, 2n ** 64n - 1n, 2n ** 64n, -1n, 7],
      P08: [-128, 127, -129, 128],
      P09: [-32_768, 32_767, -32_769, 32_768],
      P10: [-2_147_483_648, 2_147_483_647, 2_147_483_648],
      P11: [-(2n ** 63n), 2n ** 63n - 1n, 2n ** 63n, 3],
      P12: [1.5, NaN, 1n, "x"],
      P13: [Infinity, -0, "1.5"],
      P14: ["", "hi", 5],
      P15: [null, 5, {}, undefined, [1]],
      P16: [null, 0, undefined, {}],
      P17: [principal, "aaaa-aa", {}, null],
    },
  },
  {
    name: "collections",
    module: collections,
    samples: {
      Unit: [{}, { a: 1 }, [], null],
      Pair: [[1n, "x"], [1n], ["x", 1n], [1, "x"], "pair"],
      Item: [
        item,
        { id: 7, label: "seven" },
        { ...item, extra: true },
        { ...item, payload: [7] },
        null,
      ],
      MaybeItem: [null, item, undefined, 5],
      Note: [null, "s", 5],
      Items: [[], [item], [item, 5]],
      Grid: [[], [new Uint8Array([1])], [[1]]],
    },
  },
  {
    name: "variants",
    module: variants,
    samples: {
      Status: [
        { tag: "ok" },
        { tag: "ok", value: null },
        { tag: "busy", value: 3 },
        { tag: "busy", value: -1 },
        { tag: "busy" },
        { tag: "failed", value: "why" },
        { tag: "nope" },
        {},
        "ok",
      ],
      Numbered: [
        { tag: "_0_" },
        { tag: "_5_", value: 4n },
        { tag: "_5_", value: 4 },
        { tag: "_0_", value: 1n },
      ],
      Tree: [
        { tag: "leaf", value: 1n },
        {
          tag: "node",
          value: {
            left: { tag: "leaf", value: 1n },
            right: { tag: "leaf", value: 2n },
          },
        },
        { tag: "node", value: { left: { tag: "leaf", value: 1n } } },
      ],
    },
  },
  {
    name: "recursion",
    module: recursion,
    samples: {
      Even: [{ next: null }, { next: { next: null } }, { next: { next: 5 } }, {}],
      Odd: [{ next: null }, { next: { next: null } }],
      List: [
        null,
        { head: 1n, tail: null },
        { head: 1n, tail: { head: 2n, tail: null } },
        { head: 1, tail: null },
        { head: 1n },
      ],
      ListAlias: [null, { head: 1n, tail: null }, { head: 1, tail: null }],
    },
  },
  {
    name: "quoting",
    module: quoting,
    samples: {
      Weird: [
        { 'quote"mark': true, naïve: "x", "has space": 1n },
        { 'quote"mark': true, naïve: "x" },
        { 'quote"mark': 1, naïve: "x", "has space": 1n },
      ],
    },
  },
  {
    name: "deferred",
    module: deferred,
    samples: {
      Kept: [{ value: 1n }, { value: 1 }, {}],
    },
    deferredNames: ["Callback", "Registry"],
  },
  {
    name: "proto",
    module: proto,
    samples: {
      // JSON.parse, never a literal: `{"__proto__": 5}` written as a literal
      // in this file would set the sample's prototype — the very hazard the
      // fixture pins (#114).
      Holder: [JSON.parse('{"__proto__": 5}'), JSON.parse('{"__proto__": true}'), {}],
      Event: [
        { tag: "__proto__", value: 5 },
        { tag: "idle" },
        { tag: "__proto__" },
        { tag: "absent" },
      ],
    },
  },
];

for (const fixture of FIXTURES) {
  test(`dynamic and generated schemas agree on ${fixture.name}`, () => {
    const { contract, names } = load(fixture.name);
    const built = schemaFromContract(contract, { names });
    assert(built.ok, `schemaFromContract must accept the ${fixture.name} golden`);
    if (!built.ok) {
      return;
    }

    // The dynamic schema set is exactly the generated declaration set.
    const generatedNames = Object.keys(fixture.module).sort();
    assert.deepStrictEqual(Object.keys(built.schemas).sort(), generatedNames);
    assert.deepStrictEqual(
      built.deferred.map((entry) => entry.name),
      fixture.deferredNames ?? [],
    );

    // Every declaration has samples; every sample must get the identical
    // result — verdict, codes, paths, and messages — from both schemas.
    assert.deepStrictEqual(
      Object.keys(fixture.samples).sort(),
      generatedNames,
      "every generated declaration needs samples",
    );
    for (const [declaration, samples] of Object.entries(fixture.samples)) {
      const generated = fixture.module[declaration] as AnySchema;
      const dynamic = built.schemas[declaration];
      for (const sample of samples) {
        assert.deepStrictEqual(
          validate(dynamic, sample),
          validate(generated, sample),
          `${fixture.name}.${declaration} diverged on ${String(sample)}`,
        );
      }
    }
  });
}

// The #114 regression the tsc equality gate provably cannot make: TypeScript
// types a non-computed `__proto__` object-literal key as an ordinary
// property, but at runtime it sets the prototype and the field vanishes. Only
// executing the generated builder can pin that the emitted computed key
// creates an own field.
test("a generated __proto__ field is an own key, not a prototype write", () => {
  const empty = validate(proto.Holder, {});
  assert(!empty.ok, "the __proto__ field is required");
  if (!empty.ok) {
    assert.deepStrictEqual(
      empty.issues.map((issue) => [issue.code, issue.path]),
      [["missing_field", "$.__proto__"]],
    );
  }
  assert.deepStrictEqual(
    validate(proto.Holder, JSON.parse('{"__proto__": 5}')),
    { ok: true },
  );
  assert.deepStrictEqual(
    validate(proto.Event, { tag: "__proto__", value: 5 }),
    { ok: true },
  );
});
