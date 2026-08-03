// Issue #126: `c.empty` (`Schema<never>`) is admitted by every composite
// position through the `AnyFieldSchema` bound, and the widening must not
// weaken the invariant equality gate. The `@ts-expect-error` lines are
// compile-time assertions wired into the tsc harness: each marks an
// assignment the gate must refuse, so if the gate ever weakens, the
// suppressed error disappears and tsc reports the unused directive —
// turning the harness red. The runtime tests pin what `validate` says about
// the same shapes, so type admission and runtime behavior move together.

import { test } from "node:test";
import assert from "node:assert/strict";

import { c, type Schema } from "../schema.ts";
import { validate, type ValidateResult } from "../validate.ts";
import { encode, encodeArgs, decodeArgs } from "../codec.ts";
import { formModel } from "../forms.ts";

// Every composite position admits the empty leaf (the #126 matrix: record
// field, variant arm, tuple element, func arg/result; vec and opt always
// did). These are positive compile-time probes — the file failing to
// type-check is the regression signal.
export const emptyInRecord = c.record({ f: c.empty, g: c.nat });
export const emptyInVariant = c.variant({ a: c.empty, b: c.nat });
export const emptyInTuple = c.tuple([c.empty, c.nat]);
export const emptyInFunc = c.func([c.empty], [c.empty], "update");
export const emptyInVec = c.vec(c.empty);
export const emptyInOpt = c.opt(c.empty);

// The generated-module shape compiles for the admitted positions, under the
// invariant annotation the equality gate rests on. (A variant arm with an
// `empty` payload is deliberately absent: its alias/builder agreement is
// issue #127's classification divergence, tracked there.)
type EmptyField = { f: never; g: bigint };
export const EmptyField: Schema<EmptyField> = c.rec(() =>
  c.record({ f: c.empty, g: c.nat }),
);
type EmptyTuple = [never, bigint];
export const EmptyTuple: Schema<EmptyTuple> = c.rec(() =>
  c.tuple([c.empty, c.nat]),
);

// The gate keeps full strength around the admitted leaf: every wrong alias
// below must stay a compile error.
type WrongFieldType = { f: null; g: bigint };
// @ts-expect-error the alias says null where the builder infers never
export const wrongFieldType: Schema<WrongFieldType> = c.rec(() =>
  c.record({ f: c.empty, g: c.nat }),
);
type MissingField = { f: never };
// @ts-expect-error the alias omits a field the builder infers
export const missingField: Schema<MissingField> = c.rec(() =>
  c.record({ f: c.empty, g: c.nat }),
);
type ExtraField = { f: never; g: bigint; h: string };
// @ts-expect-error the alias adds a field the builder does not infer
export const extraField: Schema<ExtraField> = c.rec(() =>
  c.record({ f: c.empty, g: c.nat }),
);
type WidenedField = { f: unknown; g: bigint };
// @ts-expect-error the alias widens never to unknown
export const widenedField: Schema<WidenedField> = c.rec(() =>
  c.record({ f: c.empty, g: c.nat }),
);

function firstIssue(result: ValidateResult): { code: string; path: string } {
  assert.strictEqual(result.ok, false);
  if (result.ok) {
    throw new Error("unreachable");
  }
  const issue = result.issues[0];
  return { code: issue.code, path: issue.path };
}

test("a record with an empty field admits no value", () => {
  assert.deepStrictEqual(firstIssue(validate(emptyInRecord, { g: 5n })), {
    code: "missing_field",
    path: "$.f",
  });
  assert.deepStrictEqual(firstIssue(validate(emptyInRecord, { f: 0, g: 5n })), {
    code: "uninhabited_type",
    path: "$.f",
  });
});

test("a tuple with an empty element admits no value", () => {
  assert.deepStrictEqual(firstIssue(validate(emptyInTuple, [0, 5n])), {
    code: "uninhabited_type",
    path: "$[0]",
  });
});

test("an empty variant payload rejects every carried value", () => {
  assert.deepStrictEqual(firstIssue(validate(emptyInVariant, { tag: "a", value: 0 })), {
    code: "uninhabited_type",
    path: "$.value",
  });
});

test("a func value with empty args is still an inert reference", () => {
  const principal = { toText: () => "aaaa-aa" };
  assert.strictEqual(validate(emptyInFunc, { principal, method: "m" }).ok, true);
});

test("vec empty and opt empty keep their inhabited cases", () => {
  assert.strictEqual(validate(emptyInVec, []).ok, true);
  assert.strictEqual(validate(emptyInOpt, null).ok, true);
  assert.deepStrictEqual(firstIssue(validate(emptyInVec, [0])), {
    code: "uninhabited_type",
    path: "$[0]",
  });
});

// The package's own exports must keep composing: a func schema's stored
// args/results feed the codec's argument-sequence entries directly. These
// lines type-checking is the probe — a codec entry left at the narrower
// `AnySchema[]` bound rejects `FuncSchema`'s `AnyFieldSchema[]` storage for
// every func, empty-free ones included.
test("func schemas compose with the codec argument entries", () => {
  const plain = c.func([c.nat], [c.text], "update");
  const encoded = encodeArgs(plain.args, [1n]);
  assert.strictEqual(encoded.ok, true);
  if (encoded.ok) {
    const decoded = decodeArgs(plain.args, encoded.bytes);
    assert.strictEqual(decoded.ok, true);
  }
  // An empty arg admits no value, and the codec agrees at runtime — through
  // the sequence entry and the one-argument wrapper alike.
  const refused = encodeArgs(emptyInFunc.args, [0]);
  assert.strictEqual(refused.ok, false);
  assert.strictEqual(encode(c.empty, 0).ok, false);
});

// `formModel` admits the empty leaf (the compile probe) and renders it as
// the uninhabited control — a UI must not offer an input — including when
// it arrives through a composite.
test("formModel renders the empty leaf uninhabited", () => {
  assert.strictEqual(formModel(c.empty).control, "uninhabited");
  const record = formModel(emptyInRecord);
  assert.strictEqual(record.control, "group");
  if (record.control === "group") {
    const f = record.fields[record.fieldLabels.indexOf("f")]();
    assert.strictEqual(f.control, "uninhabited");
  }
});
