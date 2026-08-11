// The issue #149 acceptance gates for the introspection surface: rec chains
// resolve, a service's method table is readable without duck-typing, and both
// helpers fail closed on the two programmer errors — a chain that never
// terminates and an object that is not a schema.
//
// The ledger fixture is exercised through *both* routes a consumer meets,
// because their runtime shapes differ and only a resolving walk handles both:
// the generated module passes `c.func(...)` straight into `c.service`, while
// `schemaFromContract` wraps every method in a lazy `c.rec` thunk (cyclic
// services exist). A walk that duck-typed `methods[name].kind === "func"`
// would see "rec" for every method of the second one.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  c,
  resolveSchema,
  serviceMethods,
  type AnyFieldSchema,
  type AnySchema,
  type Schema,
} from "../schema.ts";
import { createActor, type Transport } from "../actor.ts";
import { schemaFromContract, type FieldNameEntry } from "../contract.ts";

import * as ledger from "../../tests/goldens/ledger.ts";
import * as recursion from "../../tests/goldens/recursion.ts";

// ---------------------------------------------------------------------------
// resolveSchema: rec chains
// ---------------------------------------------------------------------------

test("resolveSchema follows a rec chain to the node underneath", () => {
  // One hop, the shape every generated declaration has.
  assert.strictEqual(resolveSchema(c.rec(() => c.nat)), c.nat);
  // Several hops: a declaration whose body is another declaration.
  const inner = c.record({ id: c.nat });
  const alias: Schema<{ id: bigint }> = c.rec(() => c.rec(() => c.rec(() => inner)));
  assert.strictEqual(resolveSchema(alias), inner);
  // A node that is not a rec is returned untouched, not re-wrapped.
  assert.strictEqual(resolveSchema(inner), inner);
});

test("resolveSchema resolves the goldens' declarations to their real kinds", () => {
  // Every generated declaration is `c.rec`, so `.kind` read directly is
  // "rec" for all of them and says nothing about the type.
  assert.strictEqual((ledger.Account as { kind: string }).kind, "rec");
  assert.strictEqual(resolveSchema(ledger.Account).kind, "record");
  assert.strictEqual(resolveSchema(ledger.TransferResult).kind, "variant");
  assert.strictEqual(resolveSchema(ledger.actor).kind, "service");
  // A declared func: the archived-transactions callback the ledger pages on.
  const callback = resolveSchema(ledger.ArchiveCallback);
  assert.strictEqual(callback.kind, "func");
  if (callback.kind !== "func") {
    return;
  }
  assert.strictEqual(callback.mode, "query");
  assert.strictEqual(callback.args.length, 1);
  assert.strictEqual(callback.results.length, 1);
  // Recursion terminates: List's body refers to List, and resolving the
  // reference is one hop, not a descent.
  assert.strictEqual(resolveSchema(recursion.List as AnySchema).kind, "opt");
});

/** A chain of `hops` rec wrappers around `c.nat`. */
function chain(hops: number): AnyFieldSchema {
  let node: AnyFieldSchema = c.nat;
  for (let hop = 0; hop < hops; hop += 1) {
    const body: AnyFieldSchema = node;
    node = c.rec((): Schema<never> => body as Schema<never>);
  }
  return node;
}

test("resolveSchema is bounded: a self-referential chain throws, not hangs", () => {
  // The shape a mis-built declaration takes — a rec whose body is itself,
  // with no node in between. The bound is what makes this terminate.
  const loop: AnyFieldSchema = c.rec((): Schema<never> => loop as Schema<never>);
  assert.throws(
    () => resolveSchema(loop),
    (error: unknown) =>
      error instanceof TypeError && error.message === "rec chain exceeds the depth limit",
  );
  // The exact boundary and one step over it: 255 hops is the longest chain
  // that resolves, and 256 is refused. A bound asserted only from far away
  // would not notice an off-by-one either way.
  assert.strictEqual(resolveSchema(chain(255)), c.nat);
  assert.throws(
    () => resolveSchema(chain(256)),
    (error: unknown) =>
      error instanceof TypeError && error.message === "rec chain exceeds the depth limit",
  );
});

test("resolveSchema rejects anything that is not a schema object", () => {
  const rejected: readonly (readonly [string, unknown])[] = [
    ["null", null],
    ["undefined", undefined],
    ["a number", 7],
    ["a string", "nat"],
    ["an array", []],
    ["a function", () => c.nat],
    ["a bare object", {}],
    ["an object whose kind is not a string", { kind: 42 }],
  ];
  // Compared as a message rather than asserted with `assert.throws`, so a
  // case that does *not* throw names itself in the failure instead of
  // reporting an anonymous missing exception.
  for (const [what, value] of rejected) {
    let thrown: string | undefined;
    try {
      resolveSchema(value as AnyFieldSchema);
    } catch (error) {
      thrown = error instanceof TypeError ? error.message : `not a TypeError: ${String(error)}`;
    }
    assert.strictEqual(thrown, "not a schema object", `${what} must be refused`);
  }
  // A rec whose body returns a non-schema is refused at that hop rather than
  // handed back as if it were a node.
  const bad = c.rec(() => null as unknown as Schema<never>);
  assert.throws(
    () => resolveSchema(bad),
    (error: unknown) => error instanceof TypeError && error.message === "not a schema object",
  );
});

test("resolveSchema rejects a kind this package does not define", () => {
  // `Schema` requires only that `kind` be *a* string, so `{ kind: "bogus" }`
  // is a schema to the compiler. Returning it would hand an exhaustive
  // consumer a discriminator its `never` default says cannot exist, so the
  // terminal kind is checked rather than asserted.
  assert.throws(
    () => resolveSchema({ kind: "bogus" }),
    (error: unknown) =>
      error instanceof TypeError && error.message === 'unknown schema kind "bogus"',
  );
  // Including one that a prototype would answer for: the check is a Set
  // lookup, not `in`.
  assert.throws(
    () => resolveSchema({ kind: "toString" }),
    (error: unknown) =>
      error instanceof TypeError && error.message === 'unknown schema kind "toString"',
  );
  // At the end of a chain too, not only at the root.
  assert.throws(
    () => resolveSchema(c.rec(() => ({ kind: "bogus" }) as Schema<never>)),
    (error: unknown) =>
      error instanceof TypeError && error.message === 'unknown schema kind "bogus"',
  );
  // And every kind the package does define still resolves.
  const defined: readonly AnyFieldSchema[] = [
    c.nat,
    c.opt(c.text),
    c.vec(c.nat),
    c.blob(),
    c.unit(),
    c.record({ a: c.nat }),
    c.tuple([c.nat]),
    c.variant({ a: c.nat }),
    c.func([], [], "query"),
    c.service({}),
  ];
  assert.deepStrictEqual(
    defined.map((schema) => resolveSchema(schema).kind),
    ["primitive", "opt", "vec", "blob", "unit", "record", "tuple", "variant", "func", "service"],
    "every kind the union covers is admitted, so the check cannot over-refuse",
  );
});

// ---------------------------------------------------------------------------
// serviceMethods: the method table
// ---------------------------------------------------------------------------

const LEDGER_METHODS: readonly (readonly [string, string])[] = [
  ["fee", "query"],
  ["decimals", "query"],
  ["name", "query"],
  ["balance_of", "query"],
  ["get_transactions", "query"],
  ["transfer", "update"],
  ["total_supply", "query"],
  ["symbol", "query"],
];

test("serviceMethods reads the generated ledger service", () => {
  const table = serviceMethods(ledger.actor);
  assert.deepStrictEqual(
    [...table].map(([name, method]) => [name, method.mode]),
    LEDGER_METHODS.map(([name, mode]) => [name, mode]),
    "names and modes in declaration order",
  );
  // The entry names itself, so a values() iteration stays self-describing.
  for (const [name, method] of table) {
    assert.strictEqual(method.name, name);
  }
  // Arities come off the func node, not off the actor interface's types.
  assert.strictEqual(table.get("fee")?.args.length, 0);
  assert.strictEqual(table.get("fee")?.results.length, 1);
  const balanceOf = table.get("balance_of");
  assert(balanceOf !== undefined, "balance_of is a ledger method");
  if (balanceOf === undefined) {
    return;
  }
  assert.strictEqual(balanceOf.args.length, 1);
  // And the argument schemas are the real ones: the account record resolves.
  assert.strictEqual(resolveSchema(balanceOf.args[0]).kind, "record");
});

test("serviceMethods reads a service whose methods are lazy rec thunks", () => {
  // schemaFromContract wraps every method edge in `c.rec`, so this is the
  // shape a runtime-loaded service has — the one duck-typing gets wrong.
  const goldens = new URL("../../tests/goldens/", import.meta.url);
  const contract: unknown = JSON.parse(
    readFileSync(new URL("ledger.contract.json", goldens), "utf8"),
  );
  const names = JSON.parse(
    readFileSync(new URL("ledger.names.json", goldens), "utf8"),
  ) as FieldNameEntry[];
  const loaded = schemaFromContract(contract, { names });
  assert(loaded.ok, "the ledger contract document must load");
  if (!loaded.ok) {
    return;
  }
  const service = loaded.actor;
  assert(service !== undefined, "the ledger document declares an actor");
  if (service === undefined) {
    return;
  }
  // The claim this test exists for: every method is a rec, and the table
  // still reads the func underneath each one.
  const node = resolveSchema(service);
  assert.strictEqual(node.kind, "service");
  if (node.kind !== "service") {
    return;
  }
  for (const name of Object.keys(node.methods)) {
    assert.strictEqual(
      (node.methods[name] as { kind: string }).kind,
      "rec",
      `${name} is wrapped, so a non-resolving walk would see "rec"`,
    );
  }
  assert.deepStrictEqual(
    [...serviceMethods(service)].map(([name, method]) => [name, method.mode]),
    LEDGER_METHODS.map(([name, mode]) => [name, mode]),
    "the same table the generated module yields",
  );
});

test("serviceMethods and createActor walk the same table", () => {
  const transport: Transport = {
    query: () => Promise.reject(new Error("no call is made in this test")),
    call: () => Promise.reject(new Error("no call is made in this test")),
  };
  const actor = createActor<Record<string, () => Promise<unknown>>>(
    ledger.actor,
    "aaaaa-aa",
    transport,
  );
  assert.deepStrictEqual(
    Object.keys(actor),
    [...serviceMethods(ledger.actor).keys()],
    "same method names, same order",
  );
});

test("serviceMethods keys are ordinary entries, prototype names included", () => {
  // A Map rather than an object, because `__proto__` and `toString` are
  // method names Candid admits: on a returned object literal the first would
  // hit the inherited setter and the second would read as a function that is
  // not a method of this service. Built the way the Contract loader builds
  // its method maps — a null prototype, so both really are own properties.
  const methods: { [name: string]: AnySchema } = Object.create(null);
  methods["__proto__"] = c.func([], [c.nat], "query");
  methods["toString"] = c.func([c.text], [], "update");
  const table = serviceMethods(c.service(methods));
  assert.deepStrictEqual([...table.keys()], ["__proto__", "toString"]);
  assert.strictEqual(table.get("__proto__")?.mode, "query");
  assert.strictEqual(table.get("toString")?.mode, "update");
});

test("serviceMethods rejects a non-service and a non-func method", () => {
  assert.throws(
    () => serviceMethods(c.nat),
    (error: unknown) =>
      error instanceof TypeError && error.message === "serviceMethods needs a service schema",
  );
  assert.throws(
    () => serviceMethods(ledger.Account),
    (error: unknown) =>
      error instanceof TypeError && error.message === "serviceMethods needs a service schema",
  );
  // A method that resolves to something other than a func fails eagerly,
  // naming the method — the same message the actor factory raises.
  const wrong = c.service({ balance: c.rec(() => c.nat) });
  assert.throws(
    () => serviceMethods(wrong),
    (error: unknown) =>
      error instanceof TypeError && error.message === 'method "balance" is not a func schema',
  );
  // And so does a non-schema in method position.
  const foreign = c.service({ balance: null as unknown as AnySchema });
  assert.throws(
    () => serviceMethods(foreign),
    (error: unknown) => error instanceof TypeError && error.message === "not a schema object",
  );
});

test("the ledger's func-reference methods are reachable through the table", () => {
  // The paging pattern end to end at the type level: get_transactions
  // returns a response carrying a func *value*, and the schema of that
  // reference is a declared func the table's result schema leads to.
  const method = serviceMethods(ledger.actor).get("get_transactions");
  assert(method !== undefined, "get_transactions is a ledger method");
  if (method === undefined) {
    return;
  }
  const response = resolveSchema(method.results[0]);
  assert.strictEqual(response.kind, "record");
  if (response.kind !== "record") {
    return;
  }
  const archived = resolveSchema(response.fields.archived_transactions);
  assert.strictEqual(archived.kind, "vec");
  if (archived.kind !== "vec") {
    return;
  }
  const range = resolveSchema(archived.inner);
  assert.strictEqual(range.kind, "record");
  if (range.kind !== "record") {
    return;
  }
  const callback = resolveSchema(range.fields.callback);
  assert.strictEqual(callback.kind, "func");
  if (callback.kind !== "func") {
    return;
  }
  assert.strictEqual(callback.mode, "query");
});
