// The differential fuzz of issue #196, CI half: replay the committed seeded
// corpus against this runtime and hold every case to the Rust reference.
//
// `tests/goldens/differential/corpus.jsonl` is written by the Rust driver
// (`tests/differential/main.rs`): a fixed count of cases per environment over
// committed seeds, plus the minimized regression vectors, each with the
// reference's verdict. Nothing here measures time (the #39 decision); the
// count is the corpus's, fixed.
//
// Every case must either agree with the reference under the verdict mapping
// (`differential/compare.ts`), or show exactly the divergence category the
// reviewed list `tests/goldens/differential/divergences.json` gives for it.
// The list fails both ways: a new divergence fails, and so does a listed one
// that disappears — which is how a fault that removes an intended difference
// (a decoder that starts accepting overlong LEB128) is caught, and how a bug
// that gets fixed forces its vectors to be reviewed out of the list. Every
// category in the list carries its classification and reason: (a) a bug in
// this runtime, (b) a bug in the reference, (c) an intended difference.
//
// `UPDATE_GOLDENS=1 npm test` rewrites the list's `cases` and `skipped` from
// the current outcomes (categories and their reasons are edited by hand);
// review the diff like any golden.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

import { parseCorpus, runCorpus, type Outcome } from "./differential/compare.ts";

interface Category {
  readonly class: "a" | "b" | "c";
  readonly reason: string;
  readonly issue?: string;
}

interface DivergenceList {
  readonly about: string;
  readonly categories: { readonly [name: string]: Category };
  readonly cases: { readonly [id: string]: string };
  readonly skipped: { readonly [id: string]: string };
}

const corpus = parseCorpus(
  readFileSync(new URL("../../tests/goldens/differential/corpus.jsonl", import.meta.url), "utf8"),
);
const listUrl = new URL("../../tests/goldens/differential/divergences.json", import.meta.url);
const list = JSON.parse(readFileSync(listUrl, "utf8")) as DivergenceList;

let memo: readonly Outcome[] | undefined;
function outcomes(): readonly Outcome[] {
  memo ??= runCorpus(corpus);
  return memo;
}

function sorted<T>(entries: Iterable<readonly [string, T]>): { [key: string]: T } {
  const out: { [key: string]: T } = {};
  for (const [key, value] of [...entries].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    out[key] = value;
  }
  return out;
}

test("the committed corpus is the fixed seeded count, over every target", () => {
  const header = corpus.header as {
    readonly seeds: { readonly start: number; readonly envs: number };
    readonly per_env: {
      readonly decode: number;
      readonly validate: number;
      readonly contract: number;
    };
  };
  const generated = corpus.cases.filter((kase) => !kase.id.startsWith("r/"));
  const perEnv = header.per_env.decode + header.per_env.validate + header.per_env.contract;
  assert.strictEqual(generated.length, header.seeds.envs * perEnv);
  const kinds: { [kind: string]: number } = {};
  for (const kase of generated) {
    kinds[kase.kind] = (kinds[kase.kind] ?? 0) + 1;
  }
  assert.deepStrictEqual(kinds, {
    decode: header.seeds.envs * header.per_env.decode,
    validate: header.seeds.envs * header.per_env.validate,
    contract: header.seeds.envs * header.per_env.contract,
  });
  // Both verdicts occur for every target, so neither side can pass by
  // answering one way.
  for (const kind of ["decode", "validate", "contract"]) {
    const verdicts = new Set(
      generated.filter((kase) => kase.kind === kind).map((kase) => kase.ref.verdict),
    );
    assert.deepStrictEqual([...verdicts].sort(), ["accept", "reject"], kind);
  }
});

test("every case agrees with the reference or shows exactly its listed divergence", () => {
  const actual = new Map<string, string>();
  const skipped = new Map<string, string>();
  for (const outcome of outcomes()) {
    if (outcome.category !== null) {
      actual.set(outcome.id, outcome.category);
    }
    if (outcome.ours.verdict === "skip") {
      skipped.set(outcome.id, outcome.ours.reason);
    }
  }
  if (process.env.UPDATE_GOLDENS !== undefined) {
    const updated = {
      about: list.about,
      categories: list.categories,
      cases: sorted(actual),
      skipped: sorted(skipped),
    };
    writeFileSync(listUrl, `${JSON.stringify(updated, null, 2)}\n`);
  }
  const undescribed = [...new Set(actual.values())].filter(
    (category) => !Object.prototype.hasOwnProperty.call(list.categories, category),
  );
  const appeared: string[] = [];
  for (const [id, category] of actual) {
    if (list.cases[id] !== category) {
      appeared.push(`${id}: ${category} (listed: ${list.cases[id] ?? "none"})`);
    }
  }
  const disappeared = Object.keys(list.cases)
    .filter((id) => !actual.has(id))
    .map((id) => `${id}: ${list.cases[id]} no longer diverges`);
  assert.deepStrictEqual(
    { undescribed, appeared, disappeared },
    { undescribed: [], appeared: [], disappeared: [] },
  );
  // Skips (a declaration the loader omits) are pinned too, so a change in
  // what the runner can exercise is reviewed rather than silent.
  assert.deepStrictEqual(sorted(skipped), list.skipped);
});

test("every listed category is classified, reasoned, and shown by a case", () => {
  const shown = new Set(Object.values(list.cases));
  for (const [name, category] of Object.entries(list.categories)) {
    assert(["a", "b", "c"].includes(category.class), `${name}: class`);
    assert(category.reason.length > 0, `${name}: reason`);
    assert(shown.has(name), `${name}: no case shows it any more`);
  }
});

test("every minimized regression vector shows a listed divergence", () => {
  const regressions = corpus.cases.filter((kase) => kase.id.startsWith("r/"));
  assert(regressions.length > 0);
  for (const kase of regressions) {
    assert(kase.id in list.cases, `${kase.id} is not in the divergence list`);
  }
});
