import { test } from "node:test";
import assert from "node:assert/strict";
import { c, type Schema } from "../src/schema.ts";
import { emitServiceDid } from "../src/did.ts";
import type { AnySchema } from "../src/walk.ts";

test("a small service emits exactly and deterministically", () => {
  const methods = [
    {
      name: "get",
      mode: "query" as const,
      input: c.nat64 as AnySchema,
      output: c.opt(c.record({ id: c.nat64, label: c.text })) as AnySchema,
    },
    {
      name: "put",
      mode: "update" as const,
      input: null,
      output: c.variant({ ok: c.null, err: c.text }) as AnySchema,
    },
  ];
  const expected = [
    "service : {",
    "  get : (nat64) -> (opt record { id : nat64; label : text }) query;",
    "  put : () -> (variant { ok; err : text });",
    "}",
    "",
  ].join("\n");
  assert.equal(emitServiceDid(methods), expected);
  assert.equal(emitServiceDid(methods), emitServiceDid([...methods].reverse()));
});

test("recursive schemas get named type definitions", () => {
  type Tree =
    | { tag: "leaf"; value: bigint }
    | { tag: "node"; value: { left: Tree; right: Tree } };
  const Tree: Schema<Tree> = c.rec(() =>
    c.variant({ leaf: c.nat, node: c.record({ left: Tree, right: Tree }) }),
  );
  const text = emitServiceDid([
    { name: "walk", mode: "query", input: Tree as AnySchema, output: Tree as AnySchema },
  ]);
  const expected = [
    "type rec0 = variant { leaf : nat; node : record { left : rec0; right : rec0 } };",
    "",
    "service : {",
    "  walk : (rec0) -> (rec0) query;",
    "}",
    "",
  ].join("\n");
  assert.equal(text, expected);
});

test("mutual recursion emits one named cycle-closer, deterministically", () => {
  // Only the node that closes the cycle needs a name; the other inlines its
  // body through it. Semantically identical either way — the integration
  // suite proves spellings converge on interface_id.
  // eslint-disable-next-line prefer-const
  let B: AnySchema;
  const A: AnySchema = c.rec(() => c.record({ b: c.vec(B) }));
  B = c.rec(() => c.record({ a: c.vec(A) }));
  const text = emitServiceDid([
    { name: "roots", mode: "query", input: null, output: A },
  ]);
  const expected = [
    "type rec0 = record { b : vec record { a : vec rec0 } };",
    "",
    "service : {",
    "  roots : () -> (rec0) query;",
    "}",
    "",
  ].join("\n");
  assert.equal(text, expected);
});

test("numeric, keyword, and hostile labels emit correctly or fail closed", () => {
  const numbered = c.variant({ _0_: c.null, _5_: c.nat }) as AnySchema;
  const keyworded = c.record({ record: c.text, type: c.nat8 }) as AnySchema;
  const quoted = c.record({ "has space": c.nat, 'quote"mark': c.bool }) as AnySchema;
  const text = emitServiceDid([
    { name: "a", mode: "query", input: numbered, output: keyworded },
    { name: "b", mode: "query", input: quoted, output: c.bool as AnySchema },
  ]);
  assert.match(text, /variant \{ 0; 5 : nat \}/);
  assert.match(text, /record \{ "record" : text; "type" : nat8 \}/);
  assert.match(text, /record \{ "has space" : nat; "quote\\"mark" : bool \}/);

  assert.throws(
    () =>
      emitServiceDid([
        {
          name: "bad",
          mode: "query",
          input: c.record({ "ctrl\u0000char": c.nat8 }) as AnySchema,
          output: c.bool as AnySchema,
        },
      ]),
    /did_unencodable_label|control/,
  );
});

test("tuples, blobs, and unit spell as candid", () => {
  const text = emitServiceDid([
    {
      name: "shapes",
      mode: "update",
      input: c.tuple([c.nat, c.text]) as AnySchema,
      output: c.record({ data: c.blob(), unit: c.unit() }) as AnySchema,
    },
  ]);
  assert.match(text, /shapes : \(record \{ nat; text \}\) -> \(record \{ data : blob; unit : record \{\} \}\);/);
});

test("method-name and structural failures are closed", () => {
  const bool = c.bool as AnySchema;
  assert.throws(
    () => emitServiceDid([{ name: "not valid", mode: "query", input: null, output: bool }]),
    /did_invalid_method_name|identifier/,
  );
  assert.throws(
    () => emitServiceDid([{ name: "record", mode: "query", input: null, output: bool }]),
    /did_invalid_method_name|identifier/,
  );
  assert.throws(
    () =>
      emitServiceDid([
        { name: "a", mode: "query", input: null, output: bool },
        { name: "a", mode: "update", input: null, output: bool },
      ]),
    /did_duplicate_method|twice/,
  );
  assert.throws(
    () => emitServiceDid([{ name: "a", mode: "query", input: null, output: c.variant({}) as AnySchema }]),
    /did_empty_variant|no arms/,
  );
  assert.throws(
    () =>
      emitServiceDid([
        { name: "a", mode: "query", input: c.opt(c.opt(c.text)) as AnySchema, output: bool },
      ]),
    /did_unrepresentable_option|opt/,
  );
});

test("`true`/`false` labels and method names fail closed (no named spelling)", () => {
  // candid_parser rejects `true`/`false` as a record/variant label in every
  // form, quoted included, so the emitter refuses rather than emit text the
  // pinned compiler cannot parse.
  assert.throws(
    () =>
      emitServiceDid([
        {
          name: "flags",
          mode: "query",
          input: c.record({ true: c.nat }) as AnySchema,
          output: c.bool as AnySchema,
        },
      ]),
    /did_unencodable_label|Boolean token/,
  );
  assert.throws(
    () =>
      emitServiceDid([
        {
          name: "flags",
          mode: "query",
          input: null,
          output: c.variant({ false: c.null }) as AnySchema,
        },
      ]),
    /did_unencodable_label|Boolean token/,
  );
  assert.throws(
    () => emitServiceDid([{ name: "true", mode: "query", input: null, output: c.bool as AnySchema }]),
    /did_invalid_method_name|identifier/,
  );
});

test("lone-surrogate labels fail closed instead of drifting on disk", () => {
  assert.throws(
    () =>
      emitServiceDid([
        {
          name: "m",
          mode: "query",
          input: c.record({ ["a\uD800b"]: c.text }) as AnySchema,
          output: c.bool as AnySchema,
        },
      ]),
    /did_unencodable_label|surrogate/,
  );
  // A valid astral character (surrogate *pair*) stays allowed.
  const ok = emitServiceDid([
    {
      name: "m",
      mode: "query",
      input: c.record({ ["a\u{1F600}b"]: c.text }) as AnySchema,
      output: c.bool as AnySchema,
    },
  ]);
  assert.match(ok, /"a\u{1F600}b" : text/u);
});
