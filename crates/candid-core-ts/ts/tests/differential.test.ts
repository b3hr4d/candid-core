// The differential fuzz of issue #196, CI half: replay the committed seeded
// corpus against this runtime and hold every case to the Rust reference.
//
// `tests/goldens/differential/corpus.jsonl` is written by the Rust driver
// (`tests/differential/main.rs`): a fixed count of cases per environment over
// committed seeds, plus the minimized regression vectors, each with the
// reference's verdict. Nothing here measures time (the #39 decision); the
// count is the corpus's, fixed.
//
// The judge (`differential/compare.ts`) gives every case one status: it
// agrees with the reference, it diverges with an exact symptom (both
// verdicts and each refusal's class or code), the reference never judged it
// (`inconclusive`), or the loader omits its declaration (`skip`). Nothing
// attributes a divergence by rule. A divergence is accepted only when the
// reviewed list `tests/goldens/differential/divergences.json` names its case
// id with the issue that explains it and exactly the symptom observed:
//
// - a divergence whose id is not listed, or listed with another symptom,
//   fails;
// - a listed case that no longer diverges (or diverges otherwise) fails, so
//   a fix, upstream or here, updates the list, and a fault that removes an
//   intended difference is caught;
// - an `inconclusive` case fails: the corpus must hold none (the generator
//   is sized so the reference judges everything).
//
// The minimized regression vectors (`r/…`) stay in the corpus for good
// (#62): each is listed, or its `tests/fixtures/differential/regressions.json`
// entry says why it agrees (`agrees`) and it must agree — a boundary vector,
// or a fixed bug's vector kept so the bug stays fixed.
//
// `UPDATE_GOLDENS=1 npm test` rewrites the list's `cases` and `skipped` from
// the current outcomes and stops there: a case whose id and symptom are
// unchanged keeps its entry, and any other divergence is written
// `unclassified` (issue 0), which the next run refuses until it is reviewed
// and given its issue by hand.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

import {
  judge,
  parseCorpus,
  runCorpus,
  symptom,
  valuesDigest,
  type CaseLine,
  type Outcome,
  type Reference,
} from "./differential/compare.ts";
import * as codec from "../codec.ts";
import * as validation from "../validate.ts";

/** Why a listed case diverges: the side that is wrong, or a settled decision. */
type Side = "reference" | "runtime" | "decision" | "owner-call" | "unclassified";

interface Listed {
  readonly issue: number;
  readonly side: Side;
  readonly symptom: string;
}

interface Vector {
  readonly name: string;
  readonly agrees?: string;
}

interface DivergenceList {
  readonly about: string;
  /** Issue number → what it is and why its cases diverge. */
  readonly issues: { readonly [issue: string]: string };
  readonly cases: { readonly [id: string]: Listed };
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

// The header states the runtime budgets the corpus pins (generated cases
// stay below each; exact vectors sit at each bound and one past it), so it
// must name this runtime's defaults: a changed default regenerates the
// header and its vectors with it.
test("the corpus header states this runtime's default budgets", () => {
  assert.deepStrictEqual(
    (corpus.header as { readonly runtime_budgets?: unknown }).runtime_budgets,
    {
      decode: {
        maxBytes: codec.DEFAULT_MAX_BYTES,
        maxTypeTableEntries: codec.DEFAULT_MAX_TYPE_TABLE_ENTRIES,
        maxDepth: codec.DEFAULT_MAX_DEPTH,
        maxElements: codec.DEFAULT_MAX_ELEMENTS,
        maxNumericBytes: codec.DEFAULT_MAX_NUMERIC_BYTES,
      },
      validate: {
        maxDepth: validation.DEFAULT_MAX_DEPTH,
        maxElements: validation.DEFAULT_MAX_ELEMENTS,
      },
    },
  );
});

test("the reference judges every case of the corpus", () => {
  const unjudged = outcomes()
    .filter((outcome) => outcome.status === "inconclusive")
    .map((outcome) => outcome.id);
  assert.deepStrictEqual(unjudged, []);
});

test("every divergence is listed with its issue and its exact symptom, and only those", () => {
  const actual = new Map<string, string>();
  const skipped = new Map<string, string>();
  for (const outcome of outcomes()) {
    if (outcome.status === "diverge" && outcome.symptom !== null) {
      actual.set(outcome.id, outcome.symptom);
    }
    if (outcome.ours.verdict === "skip") {
      skipped.set(outcome.id, outcome.ours.reason);
    }
  }
  if (updating) {
    const cases = [...actual].map(([id, observed]): [string, Listed] => {
      const listed = list.cases[id];
      return listed !== undefined && listed.symptom === observed
        ? [id, listed]
        : [id, { issue: 0, side: "unclassified", symptom: observed }];
    });
    const updated = {
      about: list.about,
      issues: list.issues,
      cases: sorted(cases),
      skipped: sorted(skipped),
    };
    writeFileSync(listUrl, `${JSON.stringify(updated, null, 2)}\n`);
    return;
  }
  const unlisted: string[] = [];
  for (const [id, observed] of actual) {
    const listed = list.cases[id];
    if (listed === undefined || listed.symptom !== observed) {
      unlisted.push(`${id}: ${observed} (listed: ${listed?.symptom ?? "none"})`);
    }
  }
  const disappeared = Object.entries(list.cases)
    .filter(([id]) => !actual.has(id))
    .map(([id, listed]) => `${id}: listed ${listed.symptom}, no longer diverges`);
  assert.deepStrictEqual({ unlisted, disappeared }, { unlisted: [], disappeared: [] });
  // Skips (a declaration the loader omits) are pinned too, so a change in
  // what the runner can exercise is reviewed rather than silent.
  assert.deepStrictEqual(sorted(skipped), list.skipped);
});

test("every listed divergence names its issue, and every issue is described and used", () => {
  if (updating) {
    return;
  }
  const used = new Set<string>();
  for (const [id, listed] of Object.entries(list.cases)) {
    assert(
      Number.isInteger(listed.issue) && listed.issue > 0,
      `${id}: no issue number (unclassified: review it and name its issue)`,
    );
    assert(
      ["reference", "runtime", "decision", "owner-call"].includes(listed.side),
      `${id}: side ${listed.side}`,
    );
    assert(
      Object.prototype.hasOwnProperty.call(list.issues, String(listed.issue)),
      `${id}: issue ${listed.issue} is not described under \`issues\``,
    );
    used.add(String(listed.issue));
  }
  for (const [issue, text] of Object.entries(list.issues)) {
    assert(text.length > 0, `issue ${issue}: no description`);
    assert(used.has(issue), `issue ${issue}: no listed case cites it`);
  }
});

test("every minimized regression vector is listed or agrees as its entry says", () => {
  if (updating) {
    return;
  }
  const regressions = corpus.cases.filter((kase) => kase.id.startsWith("r/"));
  assert.strictEqual(regressions.length, vectors.length);
  const status = new Map(outcomes().map((outcome) => [outcome.id, outcome.status]));
  for (const vector of vectors) {
    const id = `r/${vector.name}`;
    if (vector.agrees === undefined) {
      assert(id in list.cases, `${id} is not in the divergence list and says no \`agrees\``);
    } else {
      assert(vector.agrees.length > 0, `${id}: agrees needs a reason`);
      assert(!(id in list.cases), `${id} says it agrees but is listed`);
      assert.strictEqual(status.get(id), "agree", `${id} must agree with the reference`);
    }
  }
});

/** The structural type-table edits the generator applies (`wire::mutate_table`, `contract::structural`). */
const STRUCTURAL = [
  "duplicate_field_id",
  "unsorted_field_ids",
  "duplicate_variant_id",
  "unsorted_variant_ids",
  "duplicate_method_name",
  "unsorted_method_names",
];

test("the generated corpus applies every structural type-table edit on both targets", () => {
  const decode = new Set<string>();
  const contract = new Set<string>();
  for (const kase of corpus.cases) {
    if (kase.id.startsWith("r/")) {
      continue;
    }
    if (kase.kind === "decode") {
      for (const applied of kase.mutation.split("+")) {
        decode.add(applied);
      }
    } else if (kase.kind === "contract") {
      for (const op of kase.ops) {
        if (op.edit !== undefined) {
          contract.add(op.edit);
        }
      }
    }
  }
  assert.deepStrictEqual(
    STRUCTURAL.filter((edit) => !decode.has(edit)),
    [],
    "decode: structural table edits the corpus never applies",
  );
  assert.deepStrictEqual(
    STRUCTURAL.filter((edit) => !contract.has(edit)),
    [],
    "contract: structural node edits the corpus never applies",
  );
});

/** The structural edits that are a swap of two neighbours: two ops, the first tagged. */
const SWAPS = new Set(["unsorted_field_ids", "unsorted_variant_ids", "unsorted_method_names"]);

/**
 * The structural edit a case applies alone, or null: a generated decode
 * case whose only mutation is the edit, a decode vector `r/table_<edit>`
 * that both sides refuse in the type table (the reference while reading
 * the header, this runtime with `malformed_type_table`: its name alone is
 * no evidence that its bytes carry the edit), or a contract case (generated
 * or the vector `r/contract_<edit>`) whose ops are exactly the edit's (one
 * op, or a swap's two).
 */
function editAlone(kase: CaseLine, ours: Outcome["ours"] | undefined): string | null {
  if (kase.kind === "decode") {
    if (kase.id.startsWith("r/table_")) {
      const edit = kase.id.slice("r/table_".length);
      const refused =
        kase.ref.verdict === "reject" &&
        kase.ref.class === "header" &&
        ours?.verdict === "reject" &&
        ours.code === "malformed_type_table";
      return STRUCTURAL.includes(edit) && refused ? edit : null;
    }
    return STRUCTURAL.includes(kase.mutation) ? kase.mutation : null;
  }
  if (kase.kind === "contract") {
    const edit = kase.ops[0]?.edit;
    if (edit === undefined || kase.ops.slice(1).some((op) => op.edit !== undefined)) {
      return null;
    }
    return kase.ops.length === (SWAPS.has(edit) ? 2 : 1) ? edit : null;
  }
  return null;
}

// A structural edit that only ever occurs beside another edit can be
// decided by that other edit (the review found the corpus's one generated
// duplicate method name refused for another op's sake), so each kind must
// also occur alone on each target, where its own check decides the verdict.
test("every structural edit occurs alone in some case of each target", () => {
  const alone = { decode: new Set<string>(), contract: new Set<string>() };
  const ours = new Map(outcomes().map((outcome) => [outcome.id, outcome.ours]));
  for (const kase of corpus.cases) {
    const edit = editAlone(kase, ours.get(kase.id));
    if (edit !== null && (kase.kind === "decode" || kase.kind === "contract")) {
      alone[kase.kind].add(edit);
    }
  }
  for (const target of ["decode", "contract"] as const) {
    assert.deepStrictEqual(
      STRUCTURAL.filter((edit) => !alone[target].has(edit)),
      [],
      `${target}: structural edits that never occur alone`,
    );
  }
});

// The judge's own rules, on synthetic answers (issue #196 redesign): no
// input property makes a divergence agree, and a limit refusal agrees only
// with the reference's refusal on the same budget.
test("the judge accepts nothing by rule: a limit refusal agrees only with the same limit", () => {
  const decode = (ref: Reference): CaseLine => ({
    kind: "decode",
    id: "d",
    env: "e",
    wire: null,
    expected: [],
    mutation: "none",
    hex: "",
    ref,
  });
  const validateCase = (ref: Reference): CaseLine => ({
    kind: "validate",
    id: "v",
    env: "e",
    type: "T",
    value: ["n"],
    ref,
  });
  const depth = {
    verdict: "reject",
    code: "resource_limit_exceeded",
    path: "$",
    resource: "value_depth",
  } as const;
  // decode: the reference has no budget; any limit refusal diverges, against
  // an acceptance or against any class of refusal.
  const refs: readonly Reference[] = [
    { verdict: "accept", values: [] },
    { verdict: "reject", class: "malformed" },
    { verdict: "reject", class: "header" },
    { verdict: "reject", class: "coercion" },
  ];
  for (const ref of refs) {
    assert.strictEqual(judge(decode(ref), depth).status, "diverge", JSON.stringify(ref));
  }
  assert.strictEqual(
    symptom({ verdict: "accept", values: [] }, depth),
    "ts=reject:resource_limit_exceeded/value_depth ref=accept",
  );
  // decode refusals agree by class only.
  const truncated = { verdict: "reject", code: "truncated", path: "$" } as const;
  const mismatch = { verdict: "reject", code: "type_mismatch", path: "$" } as const;
  assert.strictEqual(
    judge(decode({ verdict: "reject", class: "header" }), truncated).status,
    "agree",
  );
  assert.strictEqual(
    judge(decode({ verdict: "reject", class: "header" }), mismatch).status,
    "diverge",
  );
  assert.strictEqual(
    judge(decode({ verdict: "reject", class: "coercion" }), truncated).status,
    "diverge",
  );
  // A value mismatch carries our values' digest.
  const ours = { verdict: "accept", values: [{ $int: "1" }] } as const;
  const outcome = judge(decode({ verdict: "accept", values: [{ $int: "2" }] }), ours);
  assert.strictEqual(outcome.status, "diverge");
  assert(/^ts=accept#[0-9a-f]{8} ref=accept$/.test(outcome.symptom ?? ""), outcome.symptom ?? "");
  // validate: refusals agree whatever their codes, except a limit, which
  // agrees only with the reference's refusal on the same resource.
  const missing = { verdict: "reject", code: "missing_field", path: "$" } as const;
  assert.strictEqual(
    judge(validateCase({ verdict: "reject", class: "record_field_set_mismatch" }), missing).status,
    "agree",
  );
  assert.strictEqual(
    judge(validateCase({ verdict: "reject", class: "record_field_set_mismatch" }), depth).status,
    "diverge",
  );
  assert.strictEqual(
    judge(validateCase({ verdict: "reject", class: "resource_limit_exceeded/value_depth" }), depth)
      .status,
    "agree",
  );
  assert.strictEqual(
    judge(
      validateCase({ verdict: "reject", class: "resource_limit_exceeded/value_depth" }),
      missing,
    ).status,
    "diverge",
  );
  // A reference that did not judge is never an agreement.
  assert.strictEqual(
    judge(validateCase({ verdict: "inconclusive", budget: "host_value_limit" }), depth).status,
    "inconclusive",
  );
  assert.strictEqual(
    judge(decode({ verdict: "inconclusive", budget: "quota" }), { verdict: "accept", values: [] })
      .status,
    "inconclusive",
  );
  // ... even where this runtime skips the case: a campaign counts it.
  assert.strictEqual(
    judge(decode({ verdict: "inconclusive", budget: "quota" }), {
      verdict: "skip",
      reason: "omitted",
    }).status,
    "inconclusive",
  );
  // decode: the type-table budget is configured alike on both sides, so its
  // refusal agrees with the runtime's refusal on that resource and nothing else.
  const table = { verdict: "reject", class: "resource_limit_exceeded/type_table_entries" } as const;
  const ourTable = {
    verdict: "reject",
    code: "resource_limit_exceeded",
    path: "$",
    resource: "type_table_entries",
  } as const;
  assert.strictEqual(judge(decode(table), ourTable).status, "agree");
  assert.strictEqual(judge(decode(table), truncated).status, "diverge");
  assert.strictEqual(judge(decode(table), depth).status, "diverge");
  assert.strictEqual(
    judge(decode({ verdict: "reject", class: "header" }), ourTable).status,
    "diverge",
  );
  // Values recorded as a digest compare by our values' canonical digest.
  const digested = {
    verdict: "accept",
    values_digest: valuesDigest([{ $int: "1" }]),
  } as const;
  assert.strictEqual(judge(decode(digested), ours).status, "agree");
  assert.strictEqual(
    judge(decode(digested), { verdict: "accept", values: [{ $int: "2" }] }).status,
    "diverge",
  );
  // validate: the reference's element budget is a verdict like its depth one.
  const elements = { ...depth, resource: "value_elements" } as const;
  assert.strictEqual(
    judge(
      validateCase({ verdict: "reject", class: "resource_limit_exceeded/value_elements" }),
      elements,
    ).status,
    "agree",
  );
  assert.strictEqual(
    judge(
      validateCase({ verdict: "reject", class: "resource_limit_exceeded/value_elements" }),
      depth,
    ).status,
    "diverge",
  );
});
