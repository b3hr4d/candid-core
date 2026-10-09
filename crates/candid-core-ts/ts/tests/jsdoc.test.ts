// Issue #191: `.did` doc comments and argument names reach TypeScript users as
// JSDoc. The tsc equality gate proves the `docs` golden compiles; this suite
// asks the compiler what a hover would show, which is the part of the
// acceptance a compile cannot see:
//
// - the doc on a declaration is visible on both meanings a consumer imports:
//   the type declared under its own name (`export type X`) and the value
//   behind the `export { $X as X }` alias;
// - field, arm and `Actor` method docs read back as written, with the two
//   escapes the generator applies (`*\/` and `\@`) showing as text;
// - a hostile doc — a terminator, a forged `@param`/`@deprecated`, an
//   unclosed `{@link`, a code fence — neither ends the comment nor forges a
//   tag nor swallows the generated `@param` tags, which name exactly the
//   parameters of the method's signature;
// - a name that is not a usable TypeScript parameter falls back to `arg{n}`
//   in the signature and earns no tag.
//
// The reader is the TypeScript compiler's own JSDoc parser, reached through
// its unstable programmatic API (the `typescript` devDependency is exact-
// pinned, so "unstable" means a deliberate bump, not drift). The API is
// imported through a computed specifier and typed by the minimal shapes below,
// because its declaration files need a newer `lib` than this project's gate
// and the gate is deliberately not loosened for a test dependency.

import { test } from "node:test";
import assert from "node:assert/strict";

interface TsSymbol {
  readonly name: string;
  readonly flags: number;
}
interface TsType {
  getTypes(): readonly TsType[] | undefined;
}
interface TsSignature {
  getParameters(): readonly TsSymbol[];
}
interface TsChecker {
  getSymbolAtPosition(file: string, position: number): TsSymbol | undefined;
  getAliasedSymbol(symbol: TsSymbol): TsSymbol;
  getDeclaredTypeOfSymbol(symbol: TsSymbol): TsType;
  getTypeOfSymbol(symbol: TsSymbol): TsType | undefined;
  getPropertyOfType(type: TsType, name: string): TsSymbol | undefined;
  getPropertiesOfType(type: TsType): readonly TsSymbol[];
  getSignaturesOfType(type: TsType, kind: number): readonly TsSignature[];
  getDocumentationCommentOfSymbol(symbol: TsSymbol): string;
  getJsDocTagsOfSymbol(symbol: TsSymbol): readonly { name: string; text?: string }[];
}
interface TsProject {
  readonly checker: TsChecker;
  readonly program: { getSourceFile(file: string): { readonly text: string } | undefined };
}
interface TsApi {
  updateSnapshot(params: { openProject: string }): { getProjects(): readonly TsProject[] };
  close(): void;
}
interface UnstableApi {
  API: new (options: { cwd: string }) => TsApi;
  SymbolFlags: { Alias: number };
  SignatureKind: { Call: number };
}

const specifier = "typescript/unstable/sync";

/** Narrow a possibly-missing query result, failing the test when absent. */
function present<T>(value: T | undefined | null, what: string): T {
  if (value === undefined || value === null) {
    throw new Error(`missing ${what}`);
  }
  return value;
}

function pathOf(relative: string): string {
  return decodeURIComponent(new URL(relative, import.meta.url).pathname);
}

const GOLDEN = pathOf("../../tests/goldens/docs.ts");

/** What a consumer of the `docs` golden can ask the compiler. */
interface Session {
  readonly checker: TsChecker;
  readonly aliasFlag: number;
  readonly callKind: number;
  /** The value behind `export { $name as name }`, alias followed. */
  exported(name: string): TsSymbol;
  /** The alias symbol itself, as written at the export specifier. */
  exportAlias(name: string): TsSymbol;
  /** The type declared under the export name, `export type name = …`. */
  exportedType(name: string): TsSymbol;
  declared(name: string): TsType;
  property(type: TsType, name: string): TsSymbol;
  /** The type of a property, e.g. the anonymous record a field holds. */
  typeOf(symbol: TsSymbol): TsType;
  /** The members of a union type. */
  arms(type: TsType): readonly TsType[];
  docs(symbol: TsSymbol): string;
  tags(symbol: TsSymbol): string[];
  parameters(method: TsSymbol): string[];
}

async function withDocsGolden(run: (session: Session) => void): Promise<void> {
  const { API, SymbolFlags, SignatureKind } = (await import(specifier)) as UnstableApi;
  const api = new API({ cwd: pathOf("../") });
  try {
    const snapshot = api.updateSnapshot({ openProject: pathOf("../tsconfig.json") });
    const project = snapshot.getProjects()[0];
    const source = present(
      project.program.getSourceFile(GOLDEN),
      "docs golden (it must be part of the tsc project)",
    );
    const { checker } = project;
    const exportAlias = (name: string): TsSymbol => {
      const found = present(
        new RegExp(`export \\{[^}]*\\bas ${name}\\b`).exec(source.text),
        `export specifier for ${name}`,
      );
      const position = found.index + found[0].length - name.length;
      return present(checker.getSymbolAtPosition(GOLDEN, position), `export of ${name}`);
    };
    const exported = (name: string): TsSymbol => {
      const alias = exportAlias(name);
      assert((alias.flags & SymbolFlags.Alias) !== 0, `${name} must be an export alias`);
      return checker.getAliasedSymbol(alias);
    };
    const exportedType = (name: string): TsSymbol => {
      const found = present(
        new RegExp(`export type ${name} =`).exec(source.text),
        `export type ${name}`,
      );
      const position = found.index + "export type ".length;
      return present(checker.getSymbolAtPosition(GOLDEN, position), `type ${name}`);
    };
    const property = (type: TsType, name: string): TsSymbol =>
      present(checker.getPropertyOfType(type, name), `property ${name}`);
    run({
      checker,
      aliasFlag: SymbolFlags.Alias,
      callKind: SignatureKind.Call,
      exported,
      exportAlias,
      exportedType,
      declared: (name) => checker.getDeclaredTypeOfSymbol(exportedType(name)),
      property,
      typeOf: (symbol) => present(checker.getTypeOfSymbol(symbol), `type of ${symbol.name}`),
      arms: (type) => present(type.getTypes(), "union members"),
      docs: (symbol) => checker.getDocumentationCommentOfSymbol(symbol),
      tags: (symbol) =>
        checker.getJsDocTagsOfSymbol(symbol).map((tag) => `${tag.name} ${tag.text ?? ""}`.trim()),
      parameters: (method) => {
        const type = present(checker.getTypeOfSymbol(method), `type of ${method.name}`);
        const signatures = checker.getSignaturesOfType(type, SignatureKind.Call);
        assert.strictEqual(signatures.length, 1, `${method.name} has one signature`);
        return signatures[0].getParameters().map((parameter) => parameter.name);
      },
    });
  } finally {
    api.close();
  }
}

test("a declaration's docs are visible through the export alias, type and value", async () => {
  await withDocsGolden((s) => {
    const expected =
      "An account. Two `///` lines make two doc lines.\n" +
      "\n" +
      "A blank `///` line keeps its paragraph break; `backticks` and an email\n" +
      "a@b.c pass through, while an inline {\\@link Tokens} is escaped like any tag.";
    // The symbol a consumer's `import { Account }` binds is the module's
    // export `Account`: the type declared under that name (`export type
    // Account`), merged with the alias that adds the value meaning. It
    // carries the type's docs itself, and the editor's hover reads them.
    const alias = s.exportAlias("Account");
    assert((alias.flags & s.aliasFlag) !== 0);
    assert.strictEqual(s.docs(alias), expected);
    // Followed, the alias reaches the value, `$Account`, documented alike.
    const account = s.exported("Account");
    assert.strictEqual(account.name, "$Account");
    assert.strictEqual(s.docs(account), expected);
    assert.deepStrictEqual(s.tags(account), []);
    // The type is its own symbol, named `Account`, with the same docs.
    const accountType = s.exportedType("Account");
    assert.strictEqual(accountType.name, "Account");
    assert.strictEqual(s.docs(accountType), expected);
    assert.deepStrictEqual(s.tags(accountType), []);
    for (const meaning of [s.exported, s.exportedType]) {
      assert.strictEqual(s.docs(meaning("Tokens")), "The amount of a transfer, in e8s.");
      assert.strictEqual(s.docs(meaning("Second")), "Second is documented on its own.");
      // A block comment above a declaration is not a doc, and the file's own
      // header is detached from it by a blank line.
      assert.strictEqual(s.docs(meaning("Plain")), "");
    }
  });
});

test("field docs read back as written, with only the two escapes", async () => {
  await withDocsGolden((s) => {
    const account = s.declared("Account");
    const doc = (name: string) => s.docs(s.property(account, name));
    assert.strictEqual(
      doc("owner"),
      "The owner. It says `*\\/` mid-line, which would end a naive comment,\n" +
        "and /* a nested opener *\\/ that is inert.",
    );
    assert.strictEqual(doc("memo"), "Documented despite the block comment above.");
    assert.strictEqual(doc("note"), "A plain line comment is a doc line too.");
    // A blank line detaches a comment; a field with no comment has none; a
    // block comment is never a doc.
    assert.strictEqual(doc("detached"), "");
    assert.strictEqual(doc("undocumented"), "");
    // A doc that begins with a forged tag is text, and no tag comes of it.
    const subaccount = s.property(account, "subaccount");
    assert.strictEqual(
      s.docs(subaccount),
      "\\@param forged tag at the start of a doc line\n\\@deprecated also forged",
    );
    assert.deepStrictEqual(s.tags(subaccount), []);

    // The first-emitted declaration owns a shared node's field docs.
    assert.strictEqual(s.docs(s.property(s.declared("First"), "x")), "x, as First documents it");
    // Numeric labels keep theirs; disagreeing occurrences drop them.
    assert.strictEqual(
      s.docs(s.property(s.declared("Sparse"), "_0_")),
      "Numeric labels render by id and keep their docs.",
    );
    const twiceA = s.typeOf(s.property(s.declared("Twice"), "a"));
    assert.strictEqual(s.docs(s.property(twiceA, "x")), "");

    // Nested anonymous records carry their own.
    const items = s.property(s.declared("Nested"), "items");
    assert.strictEqual(s.docs(items), "A list of documented records.");
  });
});

test("union arms carry their docs on the discriminant", async () => {
  await withDocsGolden((s) => {
    const arms = s.arms(s.declared("Event"));
    assert.strictEqual(arms.length, 3, "Event is a three-arm union");
    assert.deepStrictEqual(
      arms.map((arm) => s.docs(s.property(arm, "tag"))).sort(),
      ["", "A transfer, carrying a documented anonymous record.", "Nothing happened."],
      "two arms are documented, one is not",
    );
    // A documented anonymous record inside an arm keeps its field docs.
    const sent = present(
      arms.find((arm) => s.docs(s.property(arm, "tag")).startsWith("A transfer")),
      "the Sent arm",
    );
    const value = s.typeOf(s.property(sent, "value"));
    assert.strictEqual(s.docs(s.property(value, "to")), "Who received it.");
    assert.strictEqual(s.docs(s.property(value, "amount")), "How much.");
    assert.strictEqual(s.docs(s.property(value, "plain")), "");
    // Only the documented arm of `Choice` has docs.
    assert.deepStrictEqual(
      s
        .arms(s.declared("Choice"))
        .map((arm) => s.docs(s.property(arm, "tag")))
        .sort(),
      ["", "The first arm."],
    );
  });
});

/** Method name -> the `@param` names the `.did` wrote, and the signature's. */
const METHODS: Record<string, { tags: string[]; parameters: string[]; docs: string }> = {
  send: {
    tags: ["to", "amount"],
    parameters: ["to", "amount", "arg2"],
    docs: "",
  },
  same_a: { tags: ["x"], parameters: ["x"], docs: "" },
  same_b: {
    tags: ["y"],
    parameters: ["y"],
    docs: "Same signature as same_a, different names.",
  },
  reserved: {
    tags: ["type", "$"],
    parameters: ["arg0", "arg1", "arg2", "arg3", "arg4", "arg5", "arg6", "type", "$"],
    docs: "Every reserved or unusable name falls back to arg{n}.",
  },
  collide: {
    tags: ["arg1", "arg0_"],
    parameters: ["arg1", "arg1_", "arg0_"],
    docs: "A fallback never steals a declared name.",
  },
  handle: { tags: ["payload", "count"], parameters: ["payload", "count"], docs: "" },
  inline: {
    tags: ["doc"],
    parameters: ["doc"],
    docs: "Anonymous types in a signature are the method's own occurrence.",
  },
  hostile: { tags: ["survivor"], parameters: ["survivor"], docs: "" },
  noargs: { tags: [], parameters: [], docs: "" },
};

test("Actor methods: docs, and @param names that match the signature", async () => {
  await withDocsGolden((s) => {
    const actor = s.declared("Actor");
    assert.strictEqual(s.docs(s.exported("actor")), "The service.");
    assert.strictEqual(s.docs(s.exportedType("Actor")), "The service.");
    for (const [name, expected] of Object.entries(METHODS)) {
      const method = s.property(actor, name);
      assert.deepStrictEqual(s.parameters(method), expected.parameters, `${name} parameters`);
      // Every tag is a `param` naming a real parameter of the signature —
      // and nothing but `param` appears: no forged `deprecated`, no `link`.
      const tags = s.checker.getJsDocTagsOfSymbol(method);
      assert.deepStrictEqual(
        tags.map((tag) => tag.name),
        expected.tags.map(() => "param"),
        `${name} tag names`,
      );
      assert.deepStrictEqual(
        tags.map((tag) => tag.text ?? ""),
        expected.tags,
        `${name} tag texts`,
      );
      for (const tag of expected.tags) {
        assert(expected.parameters.includes(tag), `${name}: @param ${tag} names a parameter`);
      }
      if (expected.docs !== "") {
        assert.strictEqual(s.docs(method), expected.docs, `${name} docs`);
      }
    }
  });
});

test("hostile doc text neither ends the comment, forges a tag, nor swallows the real ones", async () => {
  await withDocsGolden((s) => {
    const actor = s.declared("Actor");
    // `send`: an unmatched backtick, a fence and a trailing backslash, a
    // `*/` and a very long line — and both @param tags still parse.
    const send = s.property(actor, "send");
    const sendDocs = s.docs(send);
    assert(sendDocs.startsWith("Sends tokens.\n\nAn unmatched ` backtick, a fence"));
    assert(sendDocs.includes("\\`\\`\\`"), "the fence is escaped, not opened");
    assert(sendDocs.includes("trailing backslash \\\n"));
    assert(sendDocs.includes("closing with `*\\/` and long text: Lorem ipsum"));
    assert(sendDocs.includes("pariatur."), "the long line survives whole");
    assert(!sendDocs.includes("@param"), "no tag leaked into the description");
    assert.deepStrictEqual(s.tags(send), ["param to", "param amount"]);

    // `hostile`: an opened fence, an unclosed inline tag, a mid-line tag, a
    // bare `@`, and a line that is a forged tag — all text; one real tag.
    const hostile = s.property(actor, "hostile");
    assert.strictEqual(
      s.docs(hostile),
      "A fence \\`\\`\\`ts and an unclosed {\\@link Inline tag, mid-line \\@deprecated text,\n" +
        "a line of only\n" +
        "\\@\n" +
        "and a tag line \\@param forged, none of which may swallow or forge tags.",
    );
    assert.deepStrictEqual(s.tags(hostile), ["param survivor"]);
    assert.deepStrictEqual(s.parameters(hostile), ["survivor"]);
  });
});
