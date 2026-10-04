// Campaign replay for the differential fuzz (issue #196): classify every case
// of one corpus batch the Rust driver wrote in campaign mode, and print a
// JSON summary — counts per target and verdict, and per divergence category
// the count plus the first few case ids. Not a test: nothing here asserts,
// and no timing is measured (the #39 decision); the campaign's time box is
// the operator's loop around it.
//
//   DIFF_CORPUS=batch.jsonl DIFF_REPORT=summary.json \
//     node --import ./tests/register.ts tests/differential/campaign.ts
//
// Run from crates/candid-core-ts/ts. `docs/verification.md` describes the
// whole loop.

import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

import { parseCorpus, runCorpus } from "./compare.ts";

const corpusPath = process.env.DIFF_CORPUS;
const reportPath = process.env.DIFF_REPORT;
if (corpusPath === undefined || reportPath === undefined) {
  throw new Error("set DIFF_CORPUS (the batch to replay) and DIFF_REPORT (the summary to write)");
}

const corpus = parseCorpus(readFileSync(corpusPath, "utf8"));
const outcomes = runCorpus(corpus);
const byId = new Map(corpus.cases.map((kase) => [kase.id, kase]));

const totals: Record<string, number> = {};
const categories: Record<string, { count: number; examples: unknown[] }> = {};
for (const outcome of outcomes) {
  const kase = byId.get(outcome.id);
  const key =
    outcome.ours.verdict === "skip"
      ? `${kase?.kind ?? "?"}:skip:${outcome.ours.reason}`
      : `${kase?.kind ?? "?"}:${outcome.ours.verdict}`;
  totals[key] = (totals[key] ?? 0) + 1;
  if (outcome.category === null) {
    continue;
  }
  const entry = (categories[outcome.category] ??= { count: 0, examples: [] });
  entry.count += 1;
  if (entry.examples.length < 3) {
    entry.examples.push({ case: kase, ours: outcome.ours });
  }
}

writeFileSync(
  reportPath,
  `${JSON.stringify({ header: corpus.header, cases: outcomes.length, totals, categories }, null, 2)}\n`,
);
