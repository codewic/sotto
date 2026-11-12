import { describe, expect, test } from "bun:test";
import { languageOf, segments, tokenize } from "./highlight";

describe("segments", () => {
  test("splits prose and fenced code", () => {
    const s = segments("Try this:\n```ts\nconst a = 1;\n```\nThen restart.");
    expect(s).toEqual([
      { kind: "text", text: "Try this:" },
      { kind: "code", lang: "ts", code: "const a = 1;" },
      { kind: "text", text: "Then restart." },
    ]);
  });
  test("an unclosed fence runs to the end", () => {
    expect(segments("```\nls -la")).toEqual([{ kind: "code", lang: "", code: "ls -la" }]);
  });
  test("plain text is one segment", () => {
    expect(segments("no code here")).toEqual([{ kind: "text", text: "no code here" }]);
  });
});

describe("tokenize", () => {
  const kinds = (code: string, lang: string) => tokenize(code, lang).filter((t) => t.k !== "text").map((t) => `${t.k}:${t.v}`);
  test("typescript", () => {
    expect(kinds('const n = parse("42"); // done', "ts")).toEqual(["kw:const", "fn:parse", 'str:"42"', "com:// done"]);
  });
  test("python comments and keywords", () => {
    expect(kinds("def run(x):\n    return x * 2  # double", "py")).toEqual(["kw:def", "fn:run", "kw:return", "num:2", "com:# double"]);
  });
  test("sql is case-insensitive", () => {
    expect(kinds("SELECT id FROM users -- all", "sql")).toEqual(["kw:SELECT", "kw:FROM", "com:-- all"]);
  });
  test("text joins back to the original", () => {
    const code = 'fn main() { let s = "a\\"b"; }';
    expect(tokenize(code, "rust").map((t) => t.v).join("")).toBe(code);
  });
  test("aliases", () => {
    expect(languageOf("TSX")).toBe("ts");
    expect(languageOf("bash")).toBe("sh");
    expect(languageOf("cobol")).toBe("c");
  });
});

import { plain } from "./Rich";

describe("plain previews", () => {
  test("drop fences but keep the code", () => {
    expect(plain("Deploy bot (via Ada): ```rust fn main() { let n = 42; } ```")).toBe("Deploy bot (via Ada): fn main() { let n = 42; }");
    expect(plain("Try:\n```ts\nconst a = 1;\n```")).toBe("Try: const a = 1;");
    expect(plain("*bold* and `code`")).toBe("bold and code");
  });
});
