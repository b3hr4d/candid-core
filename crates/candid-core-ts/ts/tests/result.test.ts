// The issue #151 acceptance gates for schema-directed result unwrapping.
//
// The adversarial cases are the point, because the ecosystem's precedent —
// probing a decoded value for ok/err keys — gets exactly them wrong: a record
// that legitimately carries `ok` and `err` fields is not a result, and neither
// is a variant that adds a third arm to the pair. Both are pinned below
// against generated goldens rather than only hand-built schemas, so a real
// interface is what proves it.
//
// Both routes a consumer meets are exercised for the same reason introspection
// exercises them (issue #149): the generated module wraps each declaration in
// `c.rec`, while `schemaFromContract` wraps the declaration *and every arm*,
// so only a resolving classifier sees a variant through either one.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { c, type AnyFieldSchema, type Schema } from "../schema.ts";
import { serviceMethods } from "../schema.ts";
import {
  isResultSchema,
  unwrapResult,
  validate,
  type ResultErr,
  type ResultOk,
  type UnwrapResult,
  type ValidationIssue,
} from "../validate.ts";
import { encode, decode } from "../codec.ts";
import { schemaFromContract, type FieldNameEntry } from "../contract.ts";

import * as ledger from "../../tests/goldens/ledger.ts";
import * as variants from "../../tests/goldens/variants.ts";
import * as arms from "../../tests/goldens/arms.ts";

/** The first issue of a not-ok outcome, as `{ code, path }`. */
function firstIssue(outcome: { readonly issues?: readonly ValidationIssue[] }): {
  code: string;
  path: string;
} {
  assert(outcome.issues !== undefined, "expected a not-a-result-value outcome");
  if (outcome.issues === undefined) {
    throw new Error("unreachable");
  }
  return { code: outcome.issues[0].code, path: outcome.issues[0].path };
}

// ---------------------------------------------------------------------------
// isResultSchema: the classification
// ---------------------------------------------------------------------------

test("isResultSchema accepts both casings, in either arm order", () => {
  // The generated ledger declaration: a `rec` around `variant { ok; err }`,
  // the Motoko-shaped pair this repository's own fixture declares.
  assert.strictEqual((ledger.TransferResult as { kind: string }).kind, "rec");
  assert.strictEqual(isResultSchema(ledger.TransferResult), true);
  // The Rust candid-derive pair, which ICRC-1 ledgers publish.
  assert.strictEqual(isResultSchema(c.variant({ Ok: c.nat, Err: c.text })), true);
  // Arm order is not part of the convention: a variant is a set of arms.
  assert.strictEqual(isResultSchema(c.variant({ err: c.text, ok: c.nat })), true);
  assert.strictEqual(isResultSchema(c.variant({ Err: c.text, Ok: c.nat })), true);
  // Bare-tag arms qualify — the payload type is what varies, not the shape.
  assert.strictEqual(isResultSchema(c.variant({ ok: c.null, err: c.null })), true);
  // Through a chain of indirections, which is what a declaration that aliases
  // another declaration looks like — under the invariant annotation a
  // generated module carries, so the chain is the real shape.
  type OkErr = { tag: "ok"; value: bigint } | { tag: "err"; value: string };
  const alias: Schema<OkErr> = c.rec(() => c.rec(() => c.variant({ ok: c.nat, err: c.text })));
  assert.strictEqual(isResultSchema(alias), true);
});

test("a record carrying ok and err fields is not a result", () => {
  // The headline adversarial case: value-probing cannot tell this apart from
  // a result, and every field name here is legitimate domain data.
  const record = c.record({ ok: c.bool, err: c.opt(c.text) });
  assert.strictEqual(isResultSchema(record), false);
  assert.strictEqual(isResultSchema(c.rec(() => record)), false);
  assert.strictEqual(isResultSchema(c.record({ Ok: c.nat, Err: c.text })), false);
  // Nor is any other composite that can carry two members.
  assert.strictEqual(isResultSchema(c.tuple([c.nat, c.text])), false);
  assert.strictEqual(isResultSchema(c.opt(c.variant({ ok: c.nat, err: c.text }))), false);
  assert.strictEqual(isResultSchema(c.vec(c.variant({ ok: c.nat, err: c.text }))), false);
  assert.strictEqual(isResultSchema(c.nat), false);
  assert.strictEqual(isResultSchema(c.blob()), false);
  assert.strictEqual(isResultSchema(c.unit()), false);
  assert.strictEqual(isResultSchema(c.func([], [c.nat], "query")), false);
  assert.strictEqual(isResultSchema(ledger.actor), false);
  assert.strictEqual(isResultSchema(ledger.Account), false);
});

test("a variant with an extra arm is not a result", () => {
  // The generated `Status` golden is the fixture for this: it really does
  // have an `ok` arm, and it really is not a result.
  assert.strictEqual(isResultSchema(variants.Status), false);
  assert.strictEqual(isResultSchema(c.variant({ ok: c.nat, err: c.text, pending: c.null })), false);
  assert.strictEqual(isResultSchema(c.variant({ Ok: c.nat, Err: c.text, Pending: c.null })), false);
  // And neither is half a pair.
  assert.strictEqual(isResultSchema(c.variant({ ok: c.nat })), false);
  assert.strictEqual(isResultSchema(c.variant({ err: c.text })), false);
  assert.strictEqual(isResultSchema(c.variant({})), false);
});

test("the pair is matched as a pair: mixed casing and near-misses are refused", () => {
  // No generator emits a mixed pair, so admitting one would be unwrapping on
  // coincidence — which is the failure this helper exists to replace.
  assert.strictEqual(isResultSchema(c.variant({ Ok: c.nat, err: c.text })), false);
  assert.strictEqual(isResultSchema(c.variant({ ok: c.nat, Err: c.text })), false);
  // Nor is any other spelling or alias a convention.
  assert.strictEqual(isResultSchema(c.variant({ OK: c.nat, ERR: c.text })), false);
  assert.strictEqual(isResultSchema(c.variant({ ok: c.nat, error: c.text })), false);
  assert.strictEqual(isResultSchema(c.variant({ Ok: c.nat, Error: c.text })), false);
  assert.strictEqual(isResultSchema(c.variant({ success: c.nat, failure: c.text })), false);
  assert.strictEqual(isResultSchema(c.variant({ Some: c.nat, None: c.null })), false);
  assert.strictEqual(isResultSchema(arms.AliasArms), false);
});

test("an arm reached only through the prototype is not an arm", () => {
  // `"err" in arms` would answer true here; own-key membership does not.
  const inherited: { [name: string]: AnyFieldSchema } = Object.create({ err: c.text });
  inherited.ok = c.nat;
  assert.strictEqual(isResultSchema(c.variant(inherited)), false);
  // The prototype's own method names are not arms either.
  const bare: { [name: string]: AnyFieldSchema } = { ok: c.nat, toString: c.text };
  assert.strictEqual(isResultSchema(c.variant(bare)), false);
});

test("isResultSchema refuses anything that is not a schema", () => {
  // Including the mistake it exists to prevent: the decoded *value* passed
  // where its schema belongs fails loudly instead of answering false.
  assert.throws(
    () => isResultSchema({ tag: "ok", value: 5n } as unknown as AnyFieldSchema),
    (error: unknown) => error instanceof TypeError && error.message === "not a schema object",
  );
  assert.throws(
    () => isResultSchema(null as unknown as AnyFieldSchema),
    (error: unknown) => error instanceof TypeError && error.message === "not a schema object",
  );
});

test("a service method's result schema classifies through the method table", () => {
  // What a wire debugger asks: does this method return a result? The table
  // and the classifier compose without either knowing about the other.
  const transfer = serviceMethods(ledger.actor).get("transfer");
  assert(transfer !== undefined, "transfer is a ledger method");
  if (transfer === undefined) {
    return;
  }
  assert.strictEqual(isResultSchema(transfer.results[0]), true);
  const balance = serviceMethods(ledger.actor).get("balance_of");
  assert.strictEqual(isResultSchema(balance?.results[0] ?? c.nat), false);
});

// ---------------------------------------------------------------------------
// unwrapResult: the two arms
// ---------------------------------------------------------------------------

test("unwrapResult reads each arm with its own payload", () => {
  const outcome = unwrapResult(ledger.TransferResult, { tag: "ok", value: 42n });
  assert.deepStrictEqual(outcome, { ok: true, value: 42n });
  // An err arm is a value, not an exception — the whole point.
  const failure: ledger.TransferError = { tag: "too_old" };
  const rejected = unwrapResult(ledger.TransferResult, { tag: "err", value: failure });
  assert.deepStrictEqual(rejected, { ok: false, error: failure });
  // Typed from the arms, not from the value: the err payload is the real
  // variant, so its own arms are reachable without a cast.
  if (!rejected.ok && rejected.issues === undefined) {
    const tag: string = rejected.error.tag;
    assert.strictEqual(tag, "too_old");
  }
  // The capitalized pair reads the same way.
  const Capitalized = c.variant({ Ok: c.nat, Err: c.text });
  assert.deepStrictEqual(unwrapResult(Capitalized, { tag: "Ok", value: 1n }), {
    ok: true,
    value: 1n,
  });
  assert.deepStrictEqual(unwrapResult(Capitalized, { tag: "Err", value: "nope" }), {
    ok: false,
    error: "nope",
  });
});

test("a bare-tag arm unwraps to null, on both sides", () => {
  // The recorded decision: a bare arm's payload type is Candid `null`, whose
  // single JavaScript value is `null`. `undefined` is not a Candid value.
  const Ack = c.variant({ ok: c.null, err: c.text });
  const accepted = unwrapResult(Ack, { tag: "ok" });
  assert.deepStrictEqual(accepted, { ok: true, value: null });
  // The key is present rather than absent, so a consumer never branches on
  // whether a payload slot exists.
  assert.strictEqual("value" in accepted, true);
  assert.strictEqual(accepted.value, null);
  const Failed = c.variant({ ok: c.nat, err: c.null });
  assert.deepStrictEqual(unwrapResult(Failed, { tag: "err" }), { ok: false, error: null });
  // Through `rec`: the arm classification resolves, exactly as the validator
  // does, so a declared alias of `null` is still a bare tag. `NullAlias` is
  // the generated golden for that shape.
  const Aliased = c.variant({ ok: arms.NullAlias, err: c.text });
  assert.deepStrictEqual(unwrapResult(Aliased, { tag: "ok" }), { ok: true, value: null });
});

test("unwrapResult refuses a schema that is not a result, before touching the value", () => {
  const record = c.record({ ok: c.bool, err: c.opt(c.text) });
  assert.throws(
    () => unwrapResult(record, { ok: true, err: null }),
    (error: unknown) =>
      error instanceof TypeError && error.message === "unwrapResult needs a result variant schema",
  );
  assert.throws(
    () => unwrapResult(variants.Status, { tag: "ok" }),
    (error: unknown) =>
      error instanceof TypeError && error.message === "unwrapResult needs a result variant schema",
  );
  assert.throws(
    () => unwrapResult(c.variant({ Ok: c.nat, err: c.text }), { tag: "Ok", value: 1n }),
    (error: unknown) =>
      error instanceof TypeError && error.message === "unwrapResult needs a result variant schema",
  );
  // A non-schema keeps `resolveSchema`'s own wording.
  assert.throws(
    () => unwrapResult({ tag: "ok" } as unknown as AnyFieldSchema, { tag: "ok" }),
    (error: unknown) => error instanceof TypeError && error.message === "not a schema object",
  );
});

// ---------------------------------------------------------------------------
// unwrapResult: values that are not of the schema
// ---------------------------------------------------------------------------

test("a value that is not this result comes back as issues, never as an arm", () => {
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  // A record shaped like the ecosystem's probe target: no tag at all.
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, { ok: true, err: null })), {
    code: "missing_field",
    path: "$.tag",
  });
  // A tag this variant does not declare.
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, { tag: "nope", value: 1n })), {
    code: "unknown_tag",
    path: "$.tag",
  });
  // The arm's payload is missing, or is the other arm's type.
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, { tag: "ok" })), {
    code: "missing_field",
    path: "$.value",
  });
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, { tag: "ok", value: "5" })), {
    code: "invalid_type",
    path: "$.value",
  });
  // A key the variant shape does not admit.
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, { tag: "ok", value: 1n, extra: 1 })), {
    code: "unexpected_field",
    path: "$.extra",
  });
  // A payload on a bare-tag arm.
  assert.deepStrictEqual(
    firstIssue(unwrapResult(c.variant({ ok: c.null, err: c.text }), { tag: "ok", value: null })),
    { code: "unexpected_field", path: "$.value" },
  );
  // And values that are not variant-shaped at all.
  for (const value of [null, undefined, 5n, "ok", [], new Uint8Array(1)]) {
    assert.deepStrictEqual(
      firstIssue(unwrapResult(Transfer, value)),
      { code: "invalid_type", path: "$" },
      `${String(value)} is not a variant value`,
    );
  }
  // The issue list is exactly what `validate` reports for the same value, so
  // a consumer who already reads those reads these.
  const value = { tag: "ok", value: "5" };
  const unwrapped = unwrapResult(Transfer, value);
  const checked = validate(Transfer, value);
  assert.strictEqual(checked.ok, false);
  if (checked.ok) {
    return;
  }
  assert.deepStrictEqual(unwrapped.issues, checked.issues);
});

test("an uninhabited arm admits no value and still reads the other side", () => {
  const Never = c.variant({ ok: c.empty, err: c.text });
  assert.deepStrictEqual(firstIssue(unwrapResult(Never, { tag: "ok", value: 1n })), {
    code: "uninhabited_type",
    path: "$.value",
  });
  assert.deepStrictEqual(unwrapResult(Never, { tag: "err", value: "why" }), {
    ok: false,
    error: "why",
  });
});

test("validate options are forwarded, so a bounded caller stays bounded", () => {
  const Transfer = c.variant({ ok: c.vec(c.nat), err: c.text });
  const outcome = unwrapResult(Transfer, { tag: "ok", value: [1n, 2n, 3n] }, { maxElements: 2 });
  assert.strictEqual(firstIssue(outcome).code, "resource_limit_exceeded");
  assert.strictEqual(outcome.issues?.[0].resource_limit?.resource, "value_elements");
  // The same value inside the budget unwraps normally.
  assert.deepStrictEqual(unwrapResult(Transfer, { tag: "ok", value: [1n, 2n, 3n] }), {
    ok: true,
    value: [1n, 2n, 3n],
  });
});

// ---------------------------------------------------------------------------
// unwrapResult: values that fight inspection
// ---------------------------------------------------------------------------

test("a value that throws while being read comes back as issues, not an exception", () => {
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  // Throwing on the first read: the validator's own fail-closed choke point
  // catches it and the walk never reports ok.
  const throwing = {
    tag: "ok",
    get value(): bigint {
      throw new Error("boom");
    },
  };
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, throwing)), {
    code: "unreadable_value",
    path: "$.value",
  });
  // Throwing on the *second* read is the case this helper has to catch
  // itself: validation passed, and the payload read that follows is outside
  // that walk. It must still be an issue rather than a thrown error.
  let reads = 0;
  const later = {
    tag: "ok",
    get value(): bigint {
      reads += 1;
      if (reads > 1) {
        throw new Error("boom");
      }
      return 5n;
    },
  };
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, later)), {
    code: "unreadable_value",
    path: "$.value",
  });
  assert.strictEqual(reads, 2, "the validator reads the payload once, the unwrapper once");
  // A tag that throws on the second read is reported at its own path.
  let tagReads = 0;
  const throwingTag = {
    get tag(): string {
      tagReads += 1;
      if (tagReads > 1) {
        throw new Error("boom");
      }
      return "err";
    },
    value: "why",
  };
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, throwingTag)), {
    code: "unreadable_value",
    path: "$.tag",
  });
});

test("a tag that changes after validation fails closed instead of misclassifying", () => {
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  let reads = 0;
  const drifting = {
    get tag(): string {
      reads += 1;
      return reads === 1 ? "ok" : "gone";
    },
    value: 5n,
  };
  const outcome = unwrapResult(Transfer, drifting);
  assert.deepStrictEqual(firstIssue(outcome), { code: "unknown_tag", path: "$.tag" });
  assert.strictEqual(
    outcome.issues?.[0].message,
    '"gone" is not this result\'s ok or err arm',
    "the message names the tag that was actually read",
  );
  // A non-string tag on the second read is described rather than quoted.
  let numeric = 0;
  const toNumber = {
    get tag(): string {
      numeric += 1;
      return (numeric === 1 ? "ok" : 7) as string;
    },
    value: 5n,
  };
  assert.strictEqual(
    unwrapResult(Transfer, toNumber).issues?.[0].message,
    "number is not this result's ok or err arm",
  );
});

test("a documented limitation: a value that flips between its own two arms", () => {
  // The one shape a post-validation read cannot defend against: an accessor
  // that answers `ok` to the validator and `err` to the reader, so the
  // payload returned as `error` was validated against the ok arm. The
  // classification still cannot leave the pair, and no decoded value — inert
  // data — can do this. Pinned so the behaviour is recorded, not discovered.
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  let reads = 0;
  const flipping = {
    get tag(): string {
      reads += 1;
      return reads === 1 ? "ok" : "err";
    },
    value: 5n,
  };
  assert.deepStrictEqual(unwrapResult(Transfer, flipping), { ok: false, error: 5n });
});

test("a non-enumerable value property stays out of an unwrapped payload", () => {
  // The validator's presence rule is own *enumerable* — the projection
  // serialization and spread see — so a bare-tag arm carrying a hidden
  // `value` validates, and the payload read must agree with the walk that
  // accepted it rather than reaching past it.
  const Ack = c.variant({ ok: c.null, err: c.text });
  const hidden: Record<string, unknown> = { tag: "ok" };
  Object.defineProperty(hidden, "value", { value: 42n, enumerable: false });
  assert.strictEqual(validate(Ack, hidden).ok, true);
  assert.deepStrictEqual(unwrapResult(Ack, hidden), { ok: true, value: null });
  // On a payload arm the same hidden property does not satisfy the field.
  const Transfer = c.variant({ ok: c.nat, err: c.text });
  const missing: Record<string, unknown> = { tag: "ok" };
  Object.defineProperty(missing, "value", { value: 42n, enumerable: false });
  assert.deepStrictEqual(firstIssue(unwrapResult(Transfer, missing)), {
    code: "missing_field",
    path: "$.value",
  });
});

// ---------------------------------------------------------------------------
// The routes a real consumer arrives by
// ---------------------------------------------------------------------------

test("a result loaded from a Contract document unwraps through its rec arms", () => {
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
  const TransferResult = loaded.schemas.TransferResult;
  assert(TransferResult !== undefined, "the document declares TransferResult");
  // The shape this test exists for: the declaration is a rec, and so is every
  // arm underneath it, so a non-resolving classifier sees "rec" for both.
  assert.strictEqual((TransferResult as { kind: string }).kind, "rec");
  assert.strictEqual(isResultSchema(TransferResult), true);
  assert.deepStrictEqual(unwrapResult(TransferResult, { tag: "ok", value: 7n }), {
    ok: true,
    value: 7n,
  });
  const failure = { tag: "duplicate", value: { duplicate_of: 3n } };
  assert.deepStrictEqual(unwrapResult(TransferResult, { tag: "err", value: failure }), {
    ok: false,
    error: failure,
  });
});

test("a reply that made the round trip through the codec unwraps", () => {
  // The real path: bytes off the wire, decoded against the same schema, then
  // unwrapped. No stage re-validates what the previous one proved, so this is
  // the composition a consumer actually writes.
  const value: ledger.TransferResult = { tag: "err", value: { tag: "too_old" } };
  const bytes = encode(ledger.TransferResult, value);
  assert.strictEqual(bytes.ok, true);
  if (!bytes.ok) {
    return;
  }
  const back = decode(ledger.TransferResult, bytes.bytes);
  assert.strictEqual(back.ok, true);
  if (!back.ok) {
    return;
  }
  const outcome = unwrapResult(ledger.TransferResult, back.value);
  assert.deepStrictEqual(outcome, { ok: false, error: { tag: "too_old" } });
});

// ---------------------------------------------------------------------------
// The type level: payloads come off the arms, and the union narrows
// ---------------------------------------------------------------------------

// Positive compile probes, in the file's own type-check: each annotation is
// the claim, and the file failing to compile is the regression signal.
export const okPayload: ResultOk<typeof ledger.TransferResult> = 5n;
export const errPayload: ResultErr<typeof ledger.TransferResult> = { tag: "too_old" };
export const bareOkPayload: ResultOk<Schema<{ tag: "ok" } | { tag: "err"; value: string }>> = null;
export const capitalPayload: ResultOk<Schema<{ tag: "Ok"; value: bigint } | { tag: "Err" }>> = 7n;

// @ts-expect-error the ledger's ok arm carries a nat, which is a bigint
export const wrongOkPayload: ResultOk<typeof ledger.TransferResult> = "5";
// @ts-expect-error a bare-tag arm's payload is null, not undefined
export const undefinedBarePayload: ResultOk<Schema<{ tag: "ok" } | { tag: "err" }>> = undefined;
// @ts-expect-error a schema with no ok arm has no ok payload type
export const noOkArm: ResultOk<typeof c.nat> = 5n;
// @ts-expect-error nor does a record that merely carries those field names
export const recordOkArm: ResultOk<Schema<{ ok: bigint; err: string }>> = 5n;

test("the outcome narrows to each of its three states by dot access", () => {
  const outcome: UnwrapResult<bigint, ledger.TransferError> = unwrapResult(ledger.TransferResult, {
    tag: "ok",
    value: 5n,
  });
  // Each branch's annotation is the type-level assertion; the runtime checks
  // that the branch taken is the right one.
  if (outcome.issues) {
    const issues: readonly ValidationIssue[] = outcome.issues;
    assert(false, `unexpected issues: ${issues[0].message}`);
  } else if (outcome.ok) {
    const value: bigint = outcome.value;
    assert.strictEqual(value, 5n);
  } else {
    const error: ledger.TransferError = outcome.error;
    assert(false, `unexpected error arm: ${error.tag}`);
  }
  // And the other order: `ok` first, `issues` second.
  const failed = unwrapResult(ledger.TransferResult, { tag: "err", value: { tag: "too_old" } });
  if (failed.ok) {
    assert(false, "the err arm is not the ok arm");
  } else if (failed.issues !== undefined) {
    assert(false, `unexpected issues: ${failed.issues[0].message}`);
  } else {
    const error: ledger.TransferError = failed.error;
    assert.strictEqual(error.tag, "too_old");
  }
});
