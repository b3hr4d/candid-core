// Issue #245: a generated module declares each declaration's type under its
// Candid name (`export type Account = …`) and keeps the `$`-prefixed local
// for the value only (`const $Account`), so the compiler errors an app sees
// name the types it imported — `Account`, `Actor` — not the locals. A name
// that cannot be a type in the module (an ambient type a lowering
// references, or a word TypeScript refuses there) keeps the `$` local for its
// type too, and its errors name that.
//
// The tsc equality gate proves every golden compiles; it cannot see what an
// error message says. This suite compiles a consumer that makes deliberate
// mistakes against the goldens and reads the compiler's own diagnostics,
// through its unstable programmatic API (exact-pinned, as in
// `jsdoc.test.ts`). The consumer and its project exist only in memory: the
// API's virtual file system serves them and falls back to the real files for
// everything else, so no file with deliberate errors joins the gate.

import { test } from "node:test";
import assert from "node:assert/strict";

interface TsDiagnostic {
  readonly fileName?: string;
  readonly pos: number;
  readonly code: number;
  readonly text: string;
  readonly messageChain?: readonly TsDiagnostic[];
}
interface TsProject {
  readonly program: {
    getSemanticDiagnostics(file?: string): readonly TsDiagnostic[];
    getSyntacticDiagnostics(file?: string): readonly TsDiagnostic[];
  };
}
interface TsApi {
  updateSnapshot(params: { openProject: string }): { getProjects(): readonly TsProject[] };
  close(): void;
}
interface VirtualFileSystem {
  readFile(fileName: string): string | null | undefined;
  fileExists(fileName: string): boolean | undefined;
}
interface UnstableApi {
  API: new (options: { cwd: string; fs: VirtualFileSystem }) => TsApi;
}

const specifier = "typescript/unstable/sync";

function pathOf(relative: string): string {
  return decodeURIComponent(new URL(relative, import.meta.url).pathname);
}

const LEDGER = pathOf("../../tests/goldens/ledger.ts");
const SHADOWING = pathOf("../../tests/goldens/shadowing.ts");
const TYPENAMES = pathOf("../../tests/goldens/typenames.ts");
const SCHEMA = pathOf("../schema.ts");
// A directory no real file lives in, beside the real project so the
// compiler finds the default library the same way the gate does.
const ROOT = pathOf("../type-names.virtual/");
const CONFIG = `${ROOT}tsconfig.json`;
const CONSUMER = `${ROOT}consumer.ts`;

/** One line per mistake: the consumer's source line for each check. */
const MISTAKES = {
  account: `const account: Account = 42;`,
  field: `const arg: TransferArg = { to: 42, amount: { e8s: 1n }, fee: null, memo: null, from_subaccount: null, created_at_time: null };`,
  result: `const result: TransferResult = 1;`,
  actor: `const ledger: Actor = {};`,
  method: `const fee: Actor["fee"] = async () => 1;`,
  // The fallbacks: names that cannot be a type keep the `$` local, and
  // their errors name it.
  ambient: `const promise: PromiseDecl = "x";`,
  reserved: `const deleted: Deleted = 1;`,
  holder: `const holder: Refused["delete"] = 1;`,
  // A contextual keyword is a type name like any other.
  contextual: `const of: OfDecl = 1;`,
} as const;

type Mistake = keyof typeof MISTAKES;

const CONSUMER_SOURCE = [
  `import type { Account, Actor, TransferArg, TransferResult } from ${JSON.stringify(LEDGER)};`,
  `import type { Promise as PromiseDecl } from ${JSON.stringify(SHADOWING)};`,
  `import type { Refused, delete as Deleted, of as OfDecl } from ${JSON.stringify(TYPENAMES)};`,
  ...Object.values(MISTAKES),
  `export { account, arg, result, ledger, fee, promise, deleted, holder, of };`,
  ``,
].join("\n");

const CONFIG_SOURCE = JSON.stringify({
  compilerOptions: {
    strict: true,
    noEmit: true,
    target: "es2020",
    module: "esnext",
    moduleResolution: "bundler",
    allowImportingTsExtensions: true,
    paths: { "@candid-core/schema": [SCHEMA] },
  },
  files: ["consumer.ts"],
});

/** Each diagnostic's text with its whole message chain, one line per link. */
function flatten(diagnostic: TsDiagnostic): string {
  const lines = [diagnostic.text];
  for (const link of diagnostic.messageChain ?? []) {
    lines.push(
      ...flatten(link)
        .split("\n")
        .map((line) => `  ${line}`),
    );
  }
  return lines.join("\n");
}

/** The consumer's diagnostics, keyed by the mistake on the line they start on. */
async function consumerDiagnostics(): Promise<Map<Mistake, string[]>> {
  const { API } = (await import(specifier)) as UnstableApi;
  const virtual: Record<string, string> = { [CONFIG]: CONFIG_SOURCE, [CONSUMER]: CONSUMER_SOURCE };
  const api = new API({
    cwd: pathOf("../"),
    fs: {
      readFile: (fileName) => virtual[fileName],
      fileExists: (fileName) => (fileName in virtual ? true : undefined),
    },
  });
  try {
    const snapshot = api.updateSnapshot({ openProject: CONFIG });
    const { program } = snapshot.getProjects()[0];
    const syntactic = program.getSyntacticDiagnostics(CONSUMER);
    assert.deepStrictEqual(syntactic.map(flatten), [], "the consumer parses");
    const lineStarts = [0];
    for (let i = 0; i < CONSUMER_SOURCE.length; i++) {
      if (CONSUMER_SOURCE[i] === "\n") lineStarts.push(i + 1);
    }
    const lines = CONSUMER_SOURCE.split("\n");
    const byMistake = new Map<Mistake, string[]>();
    for (const diagnostic of program.getSemanticDiagnostics(CONSUMER)) {
      let line = 0;
      while (line + 1 < lineStarts.length && lineStarts[line + 1] <= diagnostic.pos) line++;
      const mistake = (Object.keys(MISTAKES) as Mistake[]).find(
        (key) => MISTAKES[key] === lines[line],
      );
      if (mistake === undefined) {
        throw new Error(
          `a diagnostic on an unexpected line: ${lines[line]}\n${flatten(diagnostic)}`,
        );
      }
      byMistake.set(mistake, [...(byMistake.get(mistake) ?? []), flatten(diagnostic)]);
    }
    return byMistake;
  } finally {
    api.close();
  }
}

test("each mistake against a generated type reports the name the app imported", async () => {
  const diagnostics = await consumerDiagnostics();
  // The whole text of every diagnostic, as the pinned compiler writes it.
  // Before, each `Account`, `Tokens`, `TransferResult` and `Actor` here read
  // `$Account`, `$Tokens`, `$TransferResult` and `$Actor`.
  assert.deepStrictEqual(Object.fromEntries(diagnostics), {
    account: ["Type 'number' is not assignable to type 'Account'."],
    field: ["Type 'number' is not assignable to type 'Account'."],
    result: ["Type 'number' is not assignable to type 'TransferResult'."],
    actor: [
      "Type '{}' is missing the following properties from type 'Actor': fee, decimals, name, balance_of, and 4 more.",
    ],
    method: [
      "Type 'Promise<number>' is not assignable to type 'Promise<Tokens>'.\n" +
        "  Type 'number' is not assignable to type 'Tokens'.",
    ],
    ambient: ["Type 'string' is not assignable to type '$Promise'."],
    reserved: ["Type 'number' is not assignable to type '$delete'."],
    holder: ["Type 'number' is not assignable to type '$delete'."],
    contextual: ["Type 'number' is not assignable to type 'of'."],
  });
  // Stated generally: no message about an ordinary name mentions a `$`.
  for (const mistake of ["account", "field", "result", "actor", "method"] as const) {
    for (const text of diagnostics.get(mistake) ?? []) {
      assert(!text.includes("$"), `${mistake}: ${text}`);
    }
  }
});
