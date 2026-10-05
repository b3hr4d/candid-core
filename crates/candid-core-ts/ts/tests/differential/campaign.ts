// Campaign replay for the differential fuzz (issue #196): judge every case of
// one corpus batch the Rust driver wrote in campaign mode, and print a JSON
// summary: counts per target and status, every divergence grouped by its
// exact symptom (with the mutations its cases carried and the first few case
// ids), and every case the reference did not judge, by the budget that
// stopped it. Not a test: nothing here asserts or accepts anything, and no
// timing is measured (the #39 decision); the campaign's time box is the
// operator's loop around it. A campaign's divergences are triaged by hand:
// none is accepted by a rule, and the expected-divergence list of the CI
// corpus is keyed by case id, so it says nothing about a campaign case.
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

interface Group {
  count: number;
  mutations: Record<string, number>;
  examples: unknown[];
}

const totals: Record<string, number> = {};
const divergences: Record<string, Group> = {};
const inconclusive: Record<string, Group> = {};
for (const outcome of outcomes) {
  const kase = byId.get(outcome.id);
  const key = `${outcome.kind}:${outcome.status}`;
  totals[key] = (totals[key] ?? 0) + 1;
  if (outcome.status === "agree" || outcome.status === "skip" || kase === undefined) {
    continue;
  }
  const name =
    outcome.status === "diverge"
      ? `${outcome.kind} ${outcome.symptom ?? "?"}`
      : `${outcome.kind} ${kase.ref.verdict}:${kase.ref.budget ?? "-"}`;
  const groups = outcome.status === "diverge" ? divergences : inconclusive;
  const group = (groups[name] ??= { count: 0, mutations: {}, examples: [] });
  group.count += 1;
  const mutation =
    kase.kind === "decode"
      ? kase.mutation
      : kase.kind === "contract"
        ? kase.ops.map((op) => op.edit ?? op.op).join("+")
        : "value";
  group.mutations[mutation] = (group.mutations[mutation] ?? 0) + 1;
  if (group.examples.length < 3) {
    group.examples.push({ case: kase, ours: outcome.ours });
  }
}

writeFileSync(
  reportPath,
  `${JSON.stringify({ header: corpus.header, cases: outcomes.length, totals, divergences, inconclusive }, null, 2)}\n`,
);
