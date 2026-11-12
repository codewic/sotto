import { Check, Copy } from "lucide-react";
import { Fragment, type ReactNode, useState } from "react";
import { type Kind, segments, tokenize } from "./highlight";

const tone: Record<Kind, string> = {
  kw: "text-[var(--code-kw)] font-medium",
  str: "text-[var(--code-str)]",
  num: "text-[var(--code-num)]",
  com: "text-[var(--code-com)] italic",
  fn: "text-[var(--code-fn)]",
  type: "text-[var(--code-type)]",
  text: "",
};

/**
 * Safe, minimal formatting: ``` fenced code with highlighting, *bold*, _italic_, `code`, and
 * "> " quote lines. Links are shown, never fetched or opened.
 */
export function Rich({ text }: { text: string }) {
  return (
    <>
      {segments(text).map((s, i) => (s.kind === "code" ? <CodeBlock key={i} lang={s.lang} code={s.code} /> : <Prose key={i} text={s.text} />))}
    </>
  );
}

function Prose({ text }: { text: string }) {
  const lines = text.split("\n");
  return (
    <>
      {lines.map((line, i) => {
        const quote = line.startsWith("> ");
        const content = inline(quote ? line.slice(2) : line);
        return (
          <Fragment key={i}>
            {quote ? <span className="my-0.5 block border-l-2 border-current/40 pl-2 opacity-80">{content}</span> : content}
            {i < lines.length - 1 && !quote && "\n"}
          </Fragment>
        );
      })}
    </>
  );
}

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="my-1.5 overflow-hidden rounded-xl bg-[var(--code-bg)] text-left">
      <div className="flex h-7 items-center justify-between pr-1 pl-3 text-[11.5px] opacity-70">
        <span className="font-mono">{lang || "code"}</span>
        <button
          type="button"
          onClick={() =>
            void navigator.clipboard.writeText(code).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            })
          }
          className="press flex h-6 cursor-pointer items-center gap-1 rounded-md px-1.5 hover:bg-current/10"
          aria-label="Copy code"
        >
          {copied ? <Check size={12} /> : <Copy size={12} />}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="scroll-pane overflow-x-auto px-3 pb-2.5 font-mono text-[12.5px] leading-[1.6] whitespace-pre">
        <code>
          {tokenize(code, lang).map((t, i) =>
            t.k === "text" ? (
              <Fragment key={i}>{t.v}</Fragment>
            ) : (
              <span key={i} className={tone[t.k]}>
                {t.v}
              </span>
            ),
          )}
        </code>
      </pre>
    </div>
  );
}

/** Message text as one plain line for list previews: no fences, markers or line breaks. */
export function plain(s: string) {
  return s
    // A fence and its one-word language label (previews may already have lost their line breaks).
    .replace(/```[\w+#.-]*/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/_([^_]+)_/g, "$1")
    .replace(/(^|\s)> /g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}

function inline(s: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(`[^`\n]+`)|(\*[^*\n]+\*)|(_[^_\n]+_)|(https?:\/\/[^\s]+)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(s))) {
    if (m.index > last) out.push(s.slice(last, m.index));
    const t = m[0];
    if (m[1]) out.push(<code key={k++} className="rounded bg-current/10 px-1 py-0.5 font-mono text-[0.88em]">{t.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={k++}>{t.slice(1, -1)}</strong>);
    else if (m[3]) out.push(<em key={k++}>{t.slice(1, -1)}</em>);
    else out.push(<span key={k++} className="underline decoration-current/50 underline-offset-2" title="Links aren't opened from Sotto. Copy it into Tor Browser if you trust it.">{t}</span>);
    last = m.index + t.length;
  }
  if (last < s.length) out.push(s.slice(last));
  return out;
}
