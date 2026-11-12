/**
 * A small, safe syntax highlighter for code in messages. It only splits text into tokens; it
 * never builds HTML, so nothing in a message can become markup or script.
 */

export type Kind = "kw" | "str" | "num" | "com" | "fn" | "type" | "text";
export type Token = { k: Kind; v: string };

type Lang = { kw: Set<string>; line: string[]; block?: [string, string]; quotes: string; caps: boolean };

const words = (s: string) => new Set(s.split(/\s+/).filter(Boolean));

const JS = "break case catch class const continue debugger default delete do else export extends finally for function if import in instanceof let new of return super switch this throw try typeof var void while with yield async await static get set from as true false null undefined";
const TS = `${JS} interface type enum implements namespace declare readonly private public protected abstract keyof infer satisfies never unknown any string number boolean`;

const LANGS: Record<string, Lang> = {
  js: { kw: words(JS), line: ["//"], block: ["/*", "*/"], quotes: "\"'`", caps: true },
  ts: { kw: words(TS), line: ["//"], block: ["/*", "*/"], quotes: "\"'`", caps: true },
  py: { kw: words("and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield True False None self match case"), line: ["#"], quotes: "\"'", caps: true },
  rust: { kw: words("as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while Some None Ok Err"), line: ["//"], block: ["/*", "*/"], quotes: "\"", caps: true },
  go: { kw: words("break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil"), line: ["//"], block: ["/*", "*/"], quotes: "\"'`", caps: true },
  sh: { kw: words("if then else elif fi for while until do done case esac in function return export local readonly echo exit set unset source sudo cd"), line: ["#"], quotes: "\"'", caps: false },
  sql: { kw: words("select from where and or not insert into values update set delete create table alter drop index join left right inner outer on group by order having limit offset as distinct union all null is in like between primary key references default begin commit rollback with returning"), line: ["--"], block: ["/*", "*/"], quotes: "'\"", caps: false },
  json: { kw: words("true false null"), line: [], quotes: "\"", caps: false },
  yaml: { kw: words("true false null yes no on off"), line: ["#"], quotes: "\"'", caps: false },
  css: { kw: words("important media supports keyframes import from to"), line: [], block: ["/*", "*/"], quotes: "\"'", caps: false },
  // C, C++, Java, Kotlin, Swift, C# and anything unlabelled: the common C-family words.
  c: { kw: words("auto break case catch char class const continue default do double else enum extends final finally float for fun func guard if implements import int interface let long namespace new null nullptr override package private protected public return short static struct switch this throw throws try typedef union unsigned using val var void volatile when where while true false"), line: ["//"], block: ["/*", "*/"], quotes: "\"'", caps: true },
};

const ALIAS: Record<string, string> = {
  js: "js", jsx: "js", javascript: "js", mjs: "js", cjs: "js",
  ts: "ts", tsx: "ts", typescript: "ts",
  py: "py", python: "py",
  rs: "rust", rust: "rust",
  go: "go", golang: "go",
  sh: "sh", bash: "sh", shell: "sh", zsh: "sh", console: "sh",
  sql: "sql", postgres: "sql", psql: "sql",
  json: "json", jsonc: "json",
  yaml: "yaml", yml: "yaml", toml: "yaml", ini: "yaml",
  css: "css", scss: "css",
};

/** The language a fence label names, or the C-family default. */
export function languageOf(label: string): string {
  return ALIAS[label.trim().toLowerCase()] ?? "c";
}

const isIdStart = (c: string) => /[A-Za-z_$]/.test(c);
const isId = (c: string) => /[A-Za-z0-9_$]/.test(c);

export function tokenize(code: string, label = ""): Token[] {
  const L = LANGS[languageOf(label)];
  const out: Token[] = [];
  const push = (k: Kind, v: string) => {
    const last = out[out.length - 1];
    if (last && last.k === k && k === "text") last.v += v;
    else out.push({ k, v });
  };
  let i = 0;
  while (i < code.length) {
    const rest = code.slice(i);
    const line = L.line.find((p) => rest.startsWith(p));
    if (line) {
      const end = code.indexOf("\n", i);
      const stop = end === -1 ? code.length : end;
      push("com", code.slice(i, stop));
      i = stop;
      continue;
    }
    if (L.block && rest.startsWith(L.block[0])) {
      const end = code.indexOf(L.block[1], i + L.block[0].length);
      const stop = end === -1 ? code.length : end + L.block[1].length;
      push("com", code.slice(i, stop));
      i = stop;
      continue;
    }
    const c = code[i];
    if (L.quotes.includes(c)) {
      let j = i + 1;
      while (j < code.length && code[j] !== c) {
        if (code[j] === "\\") j++;
        if (code[j] === "\n" && c !== "`") break;
        j++;
      }
      push("str", code.slice(i, Math.min(j + 1, code.length)));
      i = Math.min(j + 1, code.length);
      continue;
    }
    if (/[0-9]/.test(c) && !isId(code[i - 1] ?? "")) {
      const m = /^(0x[0-9a-fA-F_]+|[0-9][0-9_]*(\.[0-9_]+)?([eE][+-]?[0-9]+)?)/.exec(rest);
      const v = m ? m[0] : c;
      push("num", v);
      i += v.length;
      continue;
    }
    if (isIdStart(c)) {
      let j = i + 1;
      while (j < code.length && isId(code[j])) j++;
      const w = code.slice(i, j);
      const kw = L.kw.has(w) || (!L.caps && L.kw.has(w.toLowerCase()));
      const next = code.slice(j).match(/^\s*\(/);
      push(kw ? "kw" : next ? "fn" : L.caps && /^[A-Z]/.test(w) ? "type" : "text", w);
      i = j;
      continue;
    }
    push("text", c);
    i++;
  }
  return out;
}

export type Segment = { kind: "text"; text: string } | { kind: "code"; lang: string; code: string };

/** Split a message into prose and ``` fenced code. An unclosed fence runs to the end. */
export function segments(text: string): Segment[] {
  const out: Segment[] = [];
  const re = /```([^\n`]*)\n([\s\S]*?)(?:\n?```|$)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push({ kind: "text", text: text.slice(last, m.index).replace(/\n$/, "") });
    out.push({ kind: "code", lang: m[1].trim(), code: m[2] });
    last = m.index + m[0].length;
    if (text[last] === "\n") last++;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out.filter((s) => s.kind === "code" || s.text.length > 0);
}
