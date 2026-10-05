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
import { serviceMethods, type AnySchema } from "../schema.ts";

import * as primitives from "../../tests/goldens/primitives.ts";
import * as collections from "../../tests/goldens/collections.ts";
import * as variants from "../../tests/goldens/variants.ts";
import * as recursion from "../../tests/goldens/recursion.ts";
import * as quoting from "../../tests/goldens/quoting.ts";
import * as deferred from "../../tests/goldens/deferred.ts";
import * as proto from "../../tests/goldens/proto.ts";
import * as ledger from "../../tests/goldens/ledger.ts";
import * as empties from "../../tests/goldens/empties.ts";
import * as arms from "../../tests/goldens/arms.ts";
import * as options from "../../tests/goldens/options.ts";
import * as fidelity from "../../tests/goldens/fidelity.ts";
import * as omissions from "../../tests/goldens/omissions.ts";

interface Fixture {
  readonly name: string;
  readonly module: Record<string, unknown>;
  /** Sample values per declaration: valid and invalid alike. */
  readonly samples: Record<string, readonly unknown[]>;
}

function load(name: string): { contract: unknown; names: FieldNameEntry[]; envelope: unknown } {
  const goldens = new URL("../../tests/goldens/", import.meta.url);
  return {
    contract: JSON.parse(readFileSync(new URL(`${name}.contract.json`, goldens), "utf8")),
    names: JSON.parse(
      readFileSync(new URL(`${name}.names.json`, goldens), "utf8"),
    ) as FieldNameEntry[],
    envelope: JSON.parse(readFileSync(new URL(`${name}.envelope.json`, goldens), "utf8")),
  };
}

/**
 * What the generator left out of a fixture's module (issue #189): the
 * `<name>.omitted.json` golden `tests/golden.rs` writes from the generator's
 * own `omitted` list, or nothing for a fixture that omits nothing.
 */
function generatorOmitted(name: string): unknown[] {
  const goldens = new URL("../../tests/goldens/", import.meta.url);
  try {
    return JSON.parse(readFileSync(new URL(`${name}.omitted.json`, goldens), "utf8")) as unknown[];
  } catch (error) {
    if ((error as { code?: string }).code === "ENOENT") {
      return [];
    }
    throw error;
  }
}

// A principal is canonical text (issue #187); the non-canonical spellings and
// the former `{ toText }` carrier appear below as samples both paths refuse.
const principal = "aaaaa-aa";
const carrier = { toText: () => "aaaaa-aa" };

const item = {
  id: 7,
  label: "seven",
  payload: new Uint8Array([7]),
};

/** A valid `fidelity.Raw`: every blob position holds a `Uint8Array`. */
function rawValue(): Record<string, unknown> {
  return {
    raw: new Uint8Array([1, 2]),
    bytes: new Uint8Array([3, 4]),
    grid: [new Uint8Array([5])],
    maybe: new Uint8Array([6]),
    one: 7,
    plain: 8,
  };
}

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
      P17: [principal, "2vxsx-fae", "aaaa-aa", "AAAAA-AA", carrier, {}, null],
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
      Callback: [
        { principal, method: "go" },
        { principal: carrier, method: "go" },
        { principal: "AAAAA-AA", method: "go" },
        { principal },
        "nope",
      ],
      Registry: [principal, carrier, "AAAAA-AA", null],
    },
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
  {
    name: "ledger",
    module: ledger,
    samples: {
      Account: [
        { owner: principal, subaccount: null },
        { owner: principal, subaccount: new Uint8Array(32) },
        { owner: carrier, subaccount: null },
        {},
      ],
      ArchiveCallback: [
        { principal, method: "get" },
        { principal, method: "" },
        { principal },
        principal,
      ],
      ArchivedRange: [
        { callback: { principal, method: "get" }, start: 1n, length: 2n },
        { callback: null, start: 1n, length: 2n },
      ],
      Tokens: [{ e8s: 5n }, { e8s: 5 }, {}],
      Transaction: [
        {
          to: null,
          fee: null,
          from: null,
          memo: null,
          timestamp: 1n,
          index: 2n,
          amount: { e8s: 3n },
        },
        { timestamp: 1n },
      ],
      TransactionRange: [{ transactions: [] }, { transactions: null }],
      TransactionsResponse: [
        {
          log_length: 9n,
          transactions: [],
          archived_transactions: [
            { callback: { principal, method: "get" }, start: 0n, length: 9n },
          ],
        },
        { log_length: 9n },
      ],
      TransferArg: [
        {
          to: { owner: principal, subaccount: null },
          fee: null,
          memo: null,
          from_subaccount: null,
          created_at_time: null,
          amount: { e8s: 1n },
        },
        { to: null },
      ],
      TransferError: [
        { tag: "too_old" },
        { tag: "bad_fee", value: { expected_fee: { e8s: 1n } } },
        { tag: "nope" },
      ],
      TransferResult: [
        { tag: "ok", value: 5n },
        { tag: "err", value: { tag: "too_old" } },
        { tag: "ok" },
      ],
    },
  },
  {
    // Issue #126: `empty` in the composite positions the AnyFieldSchema
    // bound admits. No value inhabits the record or tuple — both paths must
    // refuse every sample identically — while the func value stays an inert
    // reference and vec/opt keep their inhabited cases.
    name: "empties",
    module: empties,
    samples: {
      EmptyField: [{ g: 5n }, { f: 0, g: 5n }, {}, null],
      EmptyTuple: [[0, 5n], [], null],
      EmptyFunc: [{ principal, method: "m" }, { principal }, "nope"],
      EmptyVec: [[], [0]],
      EmptyOpt: [null, 0],
    },
  },
  {
    // Issue #127: the variant-arm classification rows — both paths must
    // give identical verdicts: `value` demanded (and uninhabited) for an
    // empty payload, `value: null` accepted for an opt-empty payload, bare
    // tags through the null alias on the aliased and deduped arm alike.
    name: "arms",
    module: arms,
    samples: {
      EmptyVariant: [
        { tag: "a" },
        { tag: "a", value: 0 },
        { tag: "b", value: 5n },
        { tag: "nope" },
      ],
      EmptyOptArm: [
        { tag: "a" },
        { tag: "a", value: null },
        { tag: "a", value: 0 },
        { tag: "b", value: 5n },
      ],
      NullAlias: [null, 0],
      AliasArms: [{ tag: "tagged" }, { tag: "tagged", value: null }, { tag: "plain" }],
    },
  },
  {
    // Collapsing options box as `{ some: v }`; the rest stay `T | null`.
    // Each boxed declaration gets its three states plus the shapes the box
    // must refuse (a bare payload, an empty box, an extra key, `undefined`).
    name: "options",
    module: options,
    samples: {
      DoubleOpt: [null, { some: null }, { some: 5n }, 5n, {}, { some: 5n, extra: 0 }, undefined],
      TripleOpt: [
        null,
        { some: null },
        { some: { some: null } },
        { some: { some: 1n } },
        { some: 1n },
      ],
      OptNull: [null, { some: null }, { some: 0 }, { some: undefined }],
      OptNothing: [null, { some: null }, "x"],
      OptReserved: [null, { some: null }, { some: "anything" }, "anything", undefined],
      OptEmpty: [null, { some: null }, 0],
      OptOptEmpty: [null, { some: null }, { some: { some: null } }],
      MaybeText: [null, "x", { some: "x" }],
      AliasedOuter: [null, { some: null }, { some: "x" }, "x", { some: 5 }],
      Nothing: [null, 0],
      Ping: [
        null,
        { pong: null },
        { pong: { some: null } },
        { pong: { some: { pong: null } } },
        { some: null },
      ],
      Pong: [null, { some: null }, { some: { pong: { some: null } } }, { pong: null }],
      Settings: [
        { label: null, limit: null, flag: null },
        { label: { some: null }, limit: { some: 5n }, flag: "f" },
        { label: { some: "x" }, limit: { some: null }, flag: null },
        { label: "x", limit: null, flag: null },
        { label: null, limit: { some: 5 }, flag: null },
      ],
      Change: [
        { tag: "keep" },
        { tag: "clear", value: null },
        { tag: "clear", value: { some: null } },
        { tag: "set", value: { some: "x" } },
        { tag: "set", value: { some: null } },
        { tag: "set", value: "x" },
        { tag: "clear" },
      ],
    },
  },
  {
    // Issue #191: a declared primitive names only itself, and every `vec
    // nat8` is a blob. Each blob position is sampled with the `Uint8Array`
    // both paths must accept and the `number[]` both must refuse — the
    // pre-fix generator and loader agreed with each other and were wrong the
    // same way, so agreement alone is not the claim; the verdicts are pinned
    // in fidelity.test.ts too.
    name: "fidelity",
    module: fidelity,
    samples: {
      Memo: [0n, 5n, 5, -1n, 2n ** 64n],
      R: [{ a: 1n, b: 2n }, { a: 1n, b: 2 }, { a: 1, b: 2n }, { a: 1n }],
      Byte: [0, 255, 256, -1, 1n],
      Raw: [
        rawValue(),
        { ...rawValue(), raw: [1, 2] },
        { ...rawValue(), bytes: [3, 4] },
        { ...rawValue(), grid: [[5]] },
        { ...rawValue(), maybe: [6] },
        { ...rawValue(), maybe: null },
        { ...rawValue(), one: 256 },
      ],
      Seconds: [0n, 5n, 5, -(2n ** 63n), 2n ** 63n],
      Millis: [0n, 5n, "5"],
      Timing: [
        { started: 1n, elapsed: 2n, raw: 3n },
        { started: 1n, elapsed: 2n, raw: 3 },
        { started: 1, elapsed: 2n, raw: 3n },
      ],
      Tokens: [0n, 2n ** 100n, -1n, 5],
      BlockIndex: [0n, 2n ** 100n, -1n, 5],
      Account: [
        { owner: principal, subaccount: null },
        { owner: principal, subaccount: new Uint8Array([1]) },
        { owner: principal, subaccount: [1] },
        { owner: carrier, subaccount: null },
      ],
      TransferArg: [
        { to: { owner: principal, subaccount: null }, amount: 1n, fee: null },
        { to: { owner: principal, subaccount: null }, amount: 1n, fee: 2n },
        { to: { owner: principal, subaccount: null }, amount: 1, fee: null },
        { to: { owner: principal, subaccount: null }, amount: 1n, fee: 2 },
      ],
      TransferResult: [
        { tag: "Ok", value: 1n },
        { tag: "Err", value: "e" },
        { tag: "Ok", value: "x" },
        { tag: "Err", value: 1n },
      ],
      Nothing: [null, 0],
      Never: [null, 0, undefined],
      Anything: [null, 5, {}, undefined, [1]],
      Odd: [
        { tag: "a" },
        { tag: "a", value: null },
        { tag: "b", value: 1 },
        { tag: "c", value: 5 },
        { tag: "d", value: null },
        { tag: "d", value: { some: null } },
        { tag: "d", value: 5 },
        { tag: "e", value: null },
        { tag: "e", value: 1 },
        { tag: "f", value: [1, "x", null] },
        { tag: "f", value: "x" },
      ],
      Owner: [principal, "2vxsx-fae", carrier, 5],
      Owned: [
        { by: principal, alias: principal },
        { by: principal, alias: 5 },
        { by: carrier, alias: principal },
      ],
    },
  },
  {
    // Issue #189: what the generator omits, the loader omits — the schema
    // set equality below and the omitted-list equality after it — and what
    // both keep validates identically.
    name: "omissions",
    module: omissions,
    samples: {
      Good: [{ a: 1n }, { a: 1 }, {}],
      NoValue: [null, 0, { some: null }],
      List: [null, { head: 1n, tail: null }, { head: 1n, tail: { head: 2n, tail: null } }, {}],
      Directory: [{ svc: principal }, { svc: carrier }, {}],
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

    // The dynamic schema set is exactly the generated declaration set. The
    // `actor` export is the actor surface, not a declaration — compared
    // against `built.actor` separately.
    const generatedNames = Object.keys(fixture.module)
      .filter((key) => key !== "actor")
      .sort();
    assert.deepStrictEqual(Object.keys(built.schemas).sort(), generatedNames);
    const generatedActor = (fixture.module as { actor?: AnySchema }).actor;
    assert.strictEqual(
      built.actor !== undefined,
      generatedActor !== undefined,
      "both paths agree on whether the contract carries an actor",
    );
    if (built.actor !== undefined && generatedActor !== undefined) {
      const principalSample = "aaaaa-aa";
      assert.deepStrictEqual(
        validate(built.actor, principalSample),
        validate(generatedActor, principalSample),
      );
      // The same methods survive on both paths, in the same order.
      assert.deepStrictEqual(
        [...serviceMethods(built.actor).keys()],
        [...serviceMethods(generatedActor).keys()],
      );
    }

    // Loader parity (issue #189): the same omissions — names, kinds,
    // reasons, `via` — in the same order as the generator's list.
    assert.deepStrictEqual(built.omitted, generatorOmitted(fixture.name));

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

// The issue #152 round-trip acceptance criterion: the one-document envelope
// path must produce schemas verdict-for-verdict identical to the two-file
// path — same verdicts, same issue codes, same paths, same messages, on
// every golden cross-check sample. The envelope goldens carry the same
// contract and the same triples as the two files, wrapped the way
// `candid-core compile --envelope` wraps them, so any divergence here is a
// divergence in the envelope-reading path itself.
for (const fixture of FIXTURES) {
  test(`the envelope path matches the two-file path on ${fixture.name}`, () => {
    const { contract, names, envelope } = load(fixture.name);
    const twoFile = schemaFromContract(contract, { names });
    const oneDocument = schemaFromContract(envelope);
    assert(twoFile.ok, `the two-file path must accept the ${fixture.name} goldens`);
    assert(oneDocument.ok, `the envelope path must accept the ${fixture.name} golden`);
    if (!twoFile.ok || !oneDocument.ok) {
      return;
    }
    assert.deepStrictEqual(
      Object.keys(oneDocument.schemas).sort(),
      Object.keys(twoFile.schemas).sort(),
      "both paths build the same declaration set",
    );
    assert.strictEqual(
      oneDocument.actor !== undefined,
      twoFile.actor !== undefined,
      "both paths agree on whether the contract carries an actor",
    );
    assert.deepStrictEqual(oneDocument.omitted, twoFile.omitted);
    if (oneDocument.actor !== undefined && twoFile.actor !== undefined) {
      const principalSample = "aaaaa-aa";
      assert.deepStrictEqual(
        validate(oneDocument.actor, principalSample),
        validate(twoFile.actor, principalSample),
      );
    }
    for (const [declaration, samples] of Object.entries(fixture.samples)) {
      for (const sample of samples) {
        assert.deepStrictEqual(
          validate(oneDocument.schemas[declaration], sample),
          validate(twoFile.schemas[declaration], sample),
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
  assert.deepStrictEqual(validate(proto.Holder, JSON.parse('{"__proto__": 5}')), { ok: true });
  assert.deepStrictEqual(validate(proto.Event, { tag: "__proto__", value: 5 }), { ok: true });
});
