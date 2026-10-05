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
// A category is attributed in one of two ways. An attribution the verdict
// mapping makes from an input property the reference side flagged
// (`…:intended:…`, `…:reference:…`, `env:label-collision`) holds for every
// case it names. A plain symptom (`…:ts-rejects:<code>`, `…:ts-accepts:<class>`,
// `decode:value-mismatch`, `decode:class:…`) names only what differed; it may
// be classified (b) or (c) only `per-case` — for the listed cases, each
// minimized and attributed by hand — since the same symptom on another input
// may have another cause. (A plain symptom classified (a) says no more than
// the symptom itself: the runtime answers differently from the reference.)
//
// The minimized regression vectors (`r/…`) stay in the corpus for good
// (#62): each either shows its listed divergence or, when its
// `tests/fixtures/differential/regressions.json` entry says why (`agrees`),
// agrees with the reference — a boundary vector, or a fixed bug's vector
// kept so the bug stays fixed.
//
// `UPDATE_GOLDENS=1 npm test` rewrites the list's `cases` and `skipped` from
// the current outcomes (categories and their reasons are edited by hand) and
// stops there; review the diff like any golden, then run the suite again.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

import { parseCorpus, runCorpus, type Outcome } from "./differential/compare.ts";

interface Category {
  readonly class: "a" | "b" | "c";
  readonly reason: string;
  readonly issue?: string;
  /** Set when the class holds for the listed cases only (see above). */
  readonly attributed?: "per-case";
}

interface Vector {
  readonly name: string;
  readonly agrees?: string;
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
const vectors = (
  JSON.parse(
    readFileSync(
      new URL("../../tests/fixtures/differential/regressions.json", import.meta.url),
      "utf8",
    ),
  ) as { readonly vectors: readonly Vector[] }
).vectors;

// Rewriting the list (see above): the tests that read it pass that run
// without checking.
const updating = process.env.UPDATE_GOLDENS !== undefined;

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
    readonly seeds: { readonly start: number; readonly envs: number; readonly deep_envs: number };
    readonly per_env: {
      readonly decode: number;
      readonly validate: number;
      readonly contract: number;
    };
    readonly per_deep_env: { readonly decode: number; readonly validate: number };
  };
  const generated = corpus.cases.filter((kase) => !kase.id.startsWith("r/"));
  const kinds: { [kind: string]: number } = {};
  for (const kase of generated) {
    const deep = kase.id.startsWith("x") ? "deep " : "";
    kinds[deep + kase.kind] = (kinds[deep + kase.kind] ?? 0) + 1;
  }
  const { envs, deep_envs: deepEnvs } = header.seeds;
  assert.deepStrictEqual(kinds, {
    decode: envs * header.per_env.decode,
    validate: envs * header.per_env.validate,
    contract: envs * header.per_env.contract,
    "deep decode": deepEnvs * header.per_deep_env.decode,
    "deep validate": deepEnvs * header.per_deep_env.validate,
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
  if (updating) {
    const updated = {
      about: list.about,
      categories: list.categories,
      cases: sorted(actual),
      skipped: sorted(skipped),
    };
    writeFileSync(listUrl, `${JSON.stringify(updated, null, 2)}\n`);
    return;
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

/** Whether a category name only describes what differed (see above). */
function plainSymptom(name: string): boolean {
  return /:(ts-rejects|ts-accepts|class):|:value-mismatch$/.test(name);
}

test("every listed category is classified, reasoned, and shown by a case", () => {
  if (updating) {
    return;
  }
  const shown = new Set(Object.values(list.cases));
  for (const [name, category] of Object.entries(list.categories)) {
    assert(["a", "b", "c"].includes(category.class), `${name}: class`);
    assert(category.reason.length > 0, `${name}: reason`);
    assert(shown.has(name), `${name}: no case shows it any more`);
    if (plainSymptom(name) && category.class !== "a") {
      assert.strictEqual(
        category.attributed,
        "per-case",
        `${name} is a plain symptom: class ${category.class} holds per case only`,
      );
    }
  }
});

test("every minimized regression vector diverges as listed or agrees as its entry says", () => {
  if (updating) {
    return;
  }
  const regressions = corpus.cases.filter((kase) => kase.id.startsWith("r/"));
  assert.strictEqual(regressions.length, vectors.length);
  for (const vector of vectors) {
    const id = `r/${vector.name}`;
    if (vector.agrees === undefined) {
      assert(id in list.cases, `${id} is not in the divergence list and says no \`agrees\``);
    } else {
      // Its agreement is held by the main test (an unlisted case must
      // agree); here: an agreeing vector is never also listed.
      assert(vector.agrees.length > 0, `${id}: agrees needs a reason`);
      assert(!(id in list.cases), `${id} says it agrees but is listed`);
    }
  }
});
