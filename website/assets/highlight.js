/* A small, dependency-free syntax highlighter.
 *
 * Each language is an ordered list of rules. The rules are joined into one
 * alternation and applied left to right with a sticky regex, so the first rule
 * that matches at a position wins — which is why comments and strings are
 * listed before keywords and punctuation. Anything unmatched is emitted as
 * escaped plain text, so a language this file does not know still renders.
 */

const ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;" };

function escapeHtml(text) {
  return text.replace(/[&<>]/g, (character) => ESCAPES[character]);
}

const RUST_KEYWORDS =
  "as|async|await|break|const|continue|crate|dyn|else|enum|extern|false|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|true|type|unsafe|use|where|while";

const TS_KEYWORDS =
  "as|async|await|break|case|catch|class|const|continue|declare|default|delete|do|else|enum|export|extends|finally|for|from|function|get|if|implements|import|in|infer|instanceof|interface|keyof|let|new|of|readonly|return|satisfies|set|static|switch|this|throw|try|type|typeof|var|void|while|yield|abstract|public|private|protected|out";

const TS_LITERALS =
  "true|false|null|undefined|never|unknown|any|void|string|number|boolean|bigint|symbol|object";

const CANDID_KEYWORDS =
  "type|service|func|import|opt|vec|record|variant|blob|principal|query|composite_query|oneway|null|reserved|empty";

const CANDID_PRIMITIVES =
  "bool|text|nat8|nat16|nat32|nat64|nat|int8|int16|int32|int64|int|float32|float64";

const SHELL_BUILTINS =
  "cargo|npm|npx|node|git|rustup|wasm-pack|python3|cd|export|echo|curl|mkdir|rm|cp|mv|cat|set|source|dfx|icp";

/** @type {Record<string, Array<[RegExp, string]>>} */
const GRAMMARS = {
  rust: [
    [/\/\/[^\n]*/, "comment"],
    [/\/\*[\s\S]*?\*\//, "comment"],
    [/#!?\[[^\]]*\]/, "attr"],
    [/r#*"[\s\S]*?"#*/, "string"],
    [/b?"(?:\\.|[^"\\])*"/, "string"],
    [/'(?:\\.|[^'\\])'/, "string"],
    [/'[a-z_][a-zA-Z0-9_]*\b/, "attr"],
    [
      /\b(?:0x[0-9a-fA-F_]+|0b[01_]+|\d[\d_]*(?:\.\d[\d_]*)?)(?:[iuf](?:8|16|32|64|128|size))?\b/,
      "number",
    ],
    [new RegExp(`\\b(?:${RUST_KEYWORDS})\\b`), "keyword"],
    [
      /\b(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64|bool|char|str|String|Vec|Option|Result|Box|Arc|Rc|HashMap|BTreeMap)\b/,
      "type",
    ],
    [/\b[A-Z][A-Za-z0-9_]*\b/, "type"],
    [/\b[a-z_][a-zA-Z0-9_]*!(?=\s*[([{])/, "attr"],
    [/\b[a-z_][a-zA-Z0-9_]*(?=\s*\()/, "fn"],
    [/[{}()[\];,.:<>|&!?=+\-*/%]/, "punct"],
  ],
  ts: [
    [/\/\/[^\n]*/, "comment"],
    [/\/\*[\s\S]*?\*\//, "comment"],
    [/`(?:\\.|\$\{[^}]*\}|[^`\\])*`/, "string"],
    [/"(?:\\.|[^"\\])*"/, "string"],
    [/'(?:\\.|[^'\\])'/, "string"],
    [/\b\d[\d_]*n?\b/, "number"],
    [/@[A-Za-z_][A-Za-z0-9_]*/, "attr"],
    [new RegExp(`\\b(?:${TS_KEYWORDS})\\b`), "keyword"],
    [new RegExp(`\\b(?:${TS_LITERALS})\\b`), "type"],
    [/\b[A-Z][A-Za-z0-9_]*\b/, "type"],
    [/\b[a-zA-Z_$][\w$]*(?=\s*\()/, "fn"],
    [/[{}()[\];,.:<>|&!?=+\-*/%]/, "punct"],
  ],
  json: [
    [/"(?:\\.|[^"\\])*"(?=\s*:)/, "type"],
    [/"(?:\\.|[^"\\])*"/, "string"],
    [/\b(?:true|false|null)\b/, "keyword"],
    [/-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/, "number"],
    [/[{}[\],:]/, "punct"],
  ],
  candid: [
    [/\/\/[^\n]*/, "comment"],
    [/\/\*[\s\S]*?\*\//, "comment"],
    [/"(?:\\.|[^"\\])*"/, "string"],
    [/\b\d[\d_]*\b/, "number"],
    [new RegExp(`\\b(?:${CANDID_KEYWORDS})\\b`), "keyword"],
    [new RegExp(`\\b(?:${CANDID_PRIMITIVES})\\b`), "type"],
    [/\b[A-Za-z_][A-Za-z0-9_]*(?=\s*:)/, "fn"],
    [/[{}()[\];,.:<>|=]|->/, "punct"],
  ],
  bash: [
    [/#[^\n]*/, "comment"],
    [/"(?:\\.|[^"\\])*"/, "string"],
    [/'[^']*'/, "string"],
    [/\$\{[^}]*\}|\$[A-Za-z_][A-Za-z0-9_]*/, "attr"],
    [/(?:^|\s)--?[A-Za-z][\w-]*/, "attr"],
    [new RegExp(`(?:^|\\n|\\|\\s|&&\\s)\\s*(?:${SHELL_BUILTINS})\\b`), "keyword"],
    [/[|&;<>()]/, "punct"],
  ],
  toml: [
    [/#[^\n]*/, "comment"],
    [/^\s*\[[^\]\n]*\]/m, "type"],
    [/"(?:\\.|[^"\\])*"/, "string"],
    [/'[^']*'/, "string"],
    [/\b(?:true|false)\b/, "keyword"],
    [/\b\d[\d_.]*\b/, "number"],
    [/^\s*[A-Za-z_][\w.-]*(?=\s*=)/m, "fn"],
    [/[=[\],]/, "punct"],
  ],
};

GRAMMARS.rs = GRAMMARS.rust;
GRAMMARS.tsx = GRAMMARS.ts;
GRAMMARS.js = GRAMMARS.ts;
GRAMMARS.javascript = GRAMMARS.ts;
GRAMMARS.typescript = GRAMMARS.ts;
GRAMMARS.did = GRAMMARS.candid;
GRAMMARS.sh = GRAMMARS.bash;
GRAMMARS.shell = GRAMMARS.bash;
GRAMMARS.console = GRAMMARS.bash;
GRAMMARS.text = [];
GRAMMARS.txt = [];

const COMPILED = new Map();

function compile(language) {
  if (COMPILED.has(language)) return COMPILED.get(language);
  const rules = GRAMMARS[language];
  if (!rules || rules.length === 0) {
    COMPILED.set(language, null);
    return null;
  }
  const pattern = new RegExp(rules.map(([re]) => `(${re.source})`).join("|"), "y");
  const classes = rules.map(([, cls]) => cls);
  // A rule's own source may contain capture groups, so map each top-level
  // alternative to its first group index rather than assuming index === rule.
  const groupOffsets = [];
  let index = 1;
  for (const [re] of rules) {
    groupOffsets.push(index);
    index += 1 + countGroups(re.source);
  }
  const compiled = { pattern, classes, groupOffsets };
  COMPILED.set(language, compiled);
  return compiled;
}

function countGroups(source) {
  // Counts capturing groups: "(" not followed by "?" and not escaped, and not
  // inside a character class.
  let count = 0;
  let inClass = false;
  for (let i = 0; i < source.length; i += 1) {
    const character = source[i];
    if (character === "\\") {
      i += 1;
      continue;
    }
    if (inClass) {
      if (character === "]") inClass = false;
      continue;
    }
    if (character === "[") inClass = true;
    else if (character === "(" && source[i + 1] !== "?") count += 1;
  }
  return count;
}

export function highlight(source, language) {
  const grammar = compile(String(language || "").toLowerCase());
  if (!grammar) return escapeHtml(source);

  const { pattern, classes, groupOffsets } = grammar;
  let out = "";
  let position = 0;

  while (position < source.length) {
    pattern.lastIndex = position;
    const match = pattern.exec(source);
    if (match) {
      let ruleIndex = -1;
      for (let r = 0; r < groupOffsets.length; r += 1) {
        if (match[groupOffsets[r]] !== undefined) {
          ruleIndex = r;
          break;
        }
      }
      if (ruleIndex >= 0 && match[0].length > 0) {
        out += `<span class="tok-${classes[ruleIndex]}">${escapeHtml(match[0])}</span>`;
        position += match[0].length;
        continue;
      }
    }
    out += escapeHtml(source[position]);
    position += 1;
  }

  return out;
}

export function highlightAll(root = document) {
  for (const code of root.querySelectorAll("pre > code[data-lang]")) {
    if (code.dataset.highlighted === "true") continue;
    const language = code.dataset.lang;
    code.innerHTML = highlight(code.textContent, language);
    code.dataset.highlighted = "true";
  }
}
