import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Bold, Check, Clock, Code, FileText, Italic, Paperclip, Quote, SendHorizontal, Smile, SquareCode, X } from "lucide-react";
import { type ReactNode, type Ref, useEffect, useImperativeHandle, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { type ApiError, api, del, get, post } from "../api";
import { Avatar } from "./Avatar";
import { IconButton } from "./Button";
import { bytes } from "./bits";
import { EMOJI, type Emoji, findEmoji } from "./emoji";
import { plain } from "./Rich";
import { useToast } from "./toast";

// ---------------------------------------------------------------------------------------------
// Drafts: unsent text per conversation ("c:<id>"), group ("g:<id>") and channel ("h:<id>").
// In memory only, never on disk, so a draft is gone once Sotto locks.

const drafts = new Map<string, string>();
const listeners = new Set<() => void>();
export const getDraft = (key: string) => drafts.get(key) ?? "";
function setDraft(key: string, text: string) {
  if ((drafts.get(key) ?? "") === (text.trim() ? text : "")) return;
  if (text.trim()) drafts.set(key, text);
  else drafts.delete(key);
  listeners.forEach((l) => l());
}
/** A draft, live, for list previews ("Draft: ..."). */
export function useDraft(key: string) {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => drafts.get(key) ?? "",
  );
}

// ---------------------------------------------------------------------------------------------

/** A list row's preview: your unsent draft when there is one, otherwise `children`. */
export function DraftLine({ draftKey, off = false, children }: { draftKey: string; off?: boolean; children: ReactNode }) {
  const draft = useDraft(draftKey);
  if (off || !draft.trim()) return <>{children}</>;
  return (
    <>
      <span className="font-medium text-primary">Draft:</span> {plain(draft)}
    </>
  );
}

/** Upload one file to a conversation, group or channel's `/files` endpoint. */
export function uploadFile(path: string, file: File, caption: string) {
  return file.arrayBuffer().then((buf) =>
    api("POST", path, new Uint8Array(buf), {
      "content-type": file.type || "application/octet-stream",
      "x-file-name": encodeURIComponent(file.name),
      "x-caption": encodeURIComponent(caption),
    }),
  );
}

export type ComposerHandle = { focus(): void; setText(t: string): void; addFiles(f: File[]): void };
export type Command = { name: string; hint: string; run: () => void };
export type Member = { name: string; seed: string; choice?: string | null };

type Suggest =
  | { kind: "slash"; items: Command[]; from: number }
  | { kind: "mention"; items: Member[]; from: number }
  | { kind: "emoji"; items: Emoji[]; from: number };

type Scheduled = { id: string; kind: string; target: string; text: string; at: number; late: boolean };

const when = (sec: number) => new Date(sec * 1000).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit" });

export function Composer({
  draftKey,
  placeholder,
  banner,
  editing = false,
  onSendText,
  onSendFile,
  onArrowUpEmpty,
  onEscape,
  allowFiles = true,
  members,
  commands,
  extra,
  schedule,
  autoFocus,
  handle,
}: {
  draftKey: string;
  placeholder: string;
  /** Reply or edit banner, drawn at the top of the box. */
  banner?: ReactNode;
  editing?: boolean;
  /** Send (or save the edit of) a text message; throw to keep the text. */
  onSendText: (text: string) => Promise<unknown>;
  /** Send one file; the caption goes with the first file of a set. */
  onSendFile?: (file: File, caption: string) => Promise<unknown>;
  onArrowUpEmpty?: () => void;
  onEscape?: () => void;
  allowFiles?: boolean;
  /** Group members, for @mentions. */
  members?: Member[];
  /** Slash commands from the place this composer is in. */
  commands?: Command[];
  /** Extra buttons beside attach (for example, one-time secret). */
  extra?: ReactNode;
  /** Where "send later" sends to. */
  schedule?: { kind: "c" | "g" | "h"; target: string };
  autoFocus?: boolean;
  handle?: Ref<ComposerHandle>;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const [text, setTextState] = useState(() => getDraft(draftKey));
  const [files, setFiles] = useState<File[]>([]);
  const [sel, setSel] = useState<[number, number]>([0, 0]);
  const [pop, setPop] = useState<null | "emoji" | "schedule" | "scheduled">(null);
  const [active, setActive] = useState(0);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [focused, setFocused] = useState(false);
  const canAttach = allowFiles && !!onSendFile;
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const setText = (t: string) => {
    setTextState(t);
    // An edit isn't a draft: leaving mid-edit shouldn't turn the old message into one.
    if (!editing) setDraft(draftKey, t);
  };
  useEffect(() => {
    setTextState(getDraft(draftKey));
    setFiles([]);
    setPop(null);
  }, [draftKey]);

  // When an edit ends (saved or cancelled), the box goes back to your draft.
  const wasEditing = useRef(editing);
  useEffect(() => {
    if (wasEditing.current && !editing) setTextState(getDraft(draftKey));
    wasEditing.current = editing;
  }, [editing, draftKey]);

  const addFiles = (f: File[]) => {
    if (!canAttach || !f.length) return;
    setFiles((cur) => [...cur, ...f].slice(0, 10));
    input.current?.focus();
  };
  useImperativeHandle(handle, () => ({ focus: () => input.current?.focus(), setText, addFiles }));

  // Put the caret back after React has updated the box.
  const place = (start: number, end = start) =>
    requestAnimationFrame(() => {
      const el = input.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(start, end);
      setSel([start, end]);
    });

  // ---- formatting ----
  const wrap = (before: string, after: string) => {
    const [s, e] = sel;
    const inner = text.slice(s, e);
    setText(text.slice(0, s) + before + inner + after + text.slice(e));
    place(s + before.length, s + before.length + inner.length);
  };
  const codeBlock = () => {
    const [s, e] = sel;
    const inner = text.slice(s, e);
    const lead = s > 0 && text[s - 1] !== "\n" ? "\n" : "";
    setText(`${text.slice(0, s)}${lead}\`\`\`\n${inner}\n\`\`\`${text.slice(e)}`);
    place(s + lead.length + 4, s + lead.length + 4 + inner.length);
  };
  const quote = () => {
    const [s, e] = sel;
    const start = text.lastIndexOf("\n", s - 1) + 1;
    const block = text
      .slice(start, e)
      .split("\n")
      .map((l) => (l.startsWith("> ") ? l : `> ${l}`))
      .join("\n");
    setText(text.slice(0, start) + block + text.slice(e));
    place(start + block.length);
  };
  const insert = (str: string, from = sel[0], to = sel[1]) => {
    setText(text.slice(0, from) + str + text.slice(to));
    place(from + str.length);
  };

  // ---- slash commands, mentions, emoji shortcodes ----
  const allCommands: Command[] = [
    {
      name: "code",
      hint: "Start a code block",
      run: () => {
        setText("```\n\n```");
        place(4);
      },
    },
    ...(schedule && !editing ? [{ name: "later", hint: "Pick a time to send this message", run: () => setPop("schedule") }] : []),
    { name: "emoji", hint: "Pick an emoji", run: () => setPop("emoji") },
    ...(editing ? [] : (commands ?? [])),
  ];
  const before = text.slice(0, sel[1]);
  const suggest: Suggest | null = useMemo(() => {
    if (sel[0] !== sel[1] || before === dismissed) return null;
    if (/^\/[a-z]*$/.test(before)) {
      const q = before.slice(1);
      const items = allCommands.filter((c) => c.name.startsWith(q));
      return items.length ? { kind: "slash", items, from: 0 } : null;
    }
    const m = /(^|\s)@([^\s@]{0,30})$/.exec(before);
    if (m && members?.length) {
      const q = m[2].toLowerCase();
      const items = members.filter((x) => x.name.toLowerCase().includes(q)).slice(0, 6);
      return items.length ? { kind: "mention", items, from: sel[1] - m[2].length - 1 } : null;
    }
    const e = /(^|\s):([a-z0-9_+-]{2,})$/.exec(before);
    if (e) {
      const items = findEmoji(e[2], 6);
      return items.length ? { kind: "emoji", items, from: sel[1] - e[2].length - 1 } : null;
    }
    return null;
    // allCommands is rebuilt each render; its names only change with these inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [before, sel, members, dismissed, editing, commands?.length, !!schedule]);
  const sugKey = suggest ? `${suggest.kind}:${suggest.items.length}` : "";
  useEffect(() => setActive(0), [sugKey]);

  const choose = (i: number) => {
    if (!suggest) return;
    if (suggest.kind === "slash") {
      setText("");
      suggest.items[i].run();
    } else if (suggest.kind === "mention") {
      insert(`@${suggest.items[i].name} `, suggest.from, sel[1]);
    } else {
      insert(`${suggest.items[i].c} `, suggest.from, sel[1]);
    }
  };

  // ---- sending ----
  async function send() {
    const body = text.trim();
    if (sending || (!body && !files.length)) return;
    // A typed command ("/code") runs instead of being sent.
    const cmd = /^\/([a-z]+)$/.exec(body);
    const c = cmd && !files.length ? allCommands.find((x) => x.name === cmd[1]) : undefined;
    if (c) {
      setText("");
      c.run();
      return;
    }
    setSending(true);
    let sent = 0;
    try {
      if (files.length && onSendFile) {
        for (; sent < files.length; sent++) await onSendFile(files[sent], sent === 0 ? body : "");
      } else {
        await onSendText(body);
      }
      setText("");
      setFiles([]);
    } catch (e) {
      const why = (e as ApiError).message;
      // Keep only what didn't go, so nothing is sent twice when you try again.
      if (sent > 0) {
        setText("");
        setFiles(files.slice(sent));
        toast("error", `Sent ${sent} of ${files.length} files. ${files[sent].name} didn't send: ${why}`);
      } else toast("error", why);
    } finally {
      setSending(false);
      input.current?.focus();
    }
  }

  const scheduled = useQuery({
    queryKey: ["scheduled", schedule?.kind, schedule?.target],
    queryFn: () => get<Scheduled[]>(`/api/scheduled?kind=${schedule?.kind}&target=${schedule?.target}`),
    enabled: !!schedule,
  });
  const queued = scheduled.data ?? [];
  const later = async (at: number) => {
    if (!schedule) return;
    try {
      await post("/api/scheduled", { kind: schedule.kind, target: schedule.target, text: text.trim(), at });
      setText("");
      setPop(null);
      await qc.invalidateQueries({ queryKey: ["scheduled"] });
      toast("info", `Scheduled for ${when(at)}. It's sent while Sotto is open and unlocked.`);
    } catch (e) {
      toast("error", (e as ApiError).message);
    }
  };
  const cancel = async (id: string) => {
    try {
      await del(`/api/scheduled/${id}`);
      await qc.invalidateQueries({ queryKey: ["scheduled"] });
    } catch (e) {
      toast("error", (e as ApiError).message);
    }
  };
  useEffect(() => {
    if (pop === "scheduled" && queued.length === 0) setPop(null);
  }, [pop, queued.length]);

  const hasSel = focused && sel[0] !== sel[1];
  const rows = Math.min(10, Math.max(1, text.split("\n").length));
  const sugLabel = suggest?.kind === "slash" ? "Commands" : suggest?.kind === "mention" ? "Members" : "Emoji";

  return (
    <div className="relative">
      {suggest && !pop && (
        <ul id="composer-suggest" role="listbox" aria-label={sugLabel} className="absolute right-0 bottom-full left-0 z-20 mb-2 flex max-h-64 flex-col overflow-auto rounded-xl bg-pane p-1 shadow-float">
          {suggest.items.map((it, i) => (
            <li key={i} role="option" aria-selected={i === active}>
              <button
                type="button"
                tabIndex={-1}
                onMouseDown={(e) => {
                  e.preventDefault();
                  choose(i);
                }}
                onMouseEnter={() => setActive(i)}
                className={`flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13.5px] ${i === active ? "bg-[color-mix(in_srgb,var(--primary)_14%,transparent)]" : ""}`}
              >
                {suggest.kind === "slash" && (
                  <>
                    <span className="font-mono text-[13px] text-primary">/{(it as Command).name}</span>
                    <span className="truncate text-muted-foreground">{(it as Command).hint}</span>
                  </>
                )}
                {suggest.kind === "mention" && (
                  <>
                    <Avatar seed={(it as Member).seed} choice={(it as Member).choice} size={22} />
                    <span className="truncate">{(it as Member).name}</span>
                  </>
                )}
                {suggest.kind === "emoji" && (
                  <>
                    <span className="text-[18px]">{(it as Emoji).c}</span>
                    <span className="font-mono text-[12.5px] text-muted-foreground">:{(it as Emoji).n}:</span>
                  </>
                )}
              </button>
            </li>
          ))}
          <li className="px-2.5 pt-1 pb-0.5 text-[11.5px] text-muted-foreground" aria-hidden>
            ↑ ↓ to choose, Enter or Tab to pick, Esc to close
          </li>
        </ul>
      )}
      {pop === "emoji" && (
        <EmojiPicker
          onPick={(c) => insert(c)}
          onClose={() => {
            setPop(null);
            input.current?.focus();
          }}
        />
      )}
      {pop === "schedule" && <SchedulePicker onPick={(at) => void later(at)} onClose={() => setPop(null)} empty={!text.trim()} withFiles={files.length > 0} />}
      {pop === "scheduled" && (
        <div role="dialog" aria-label="Scheduled messages" className="absolute bottom-full left-0 z-20 mb-2 w-[min(420px,100%)] rounded-xl bg-pane p-2 shadow-float">
          <div className="flex items-center justify-between px-2 pt-1 pb-1.5">
            <span className="text-[12.5px] font-semibold">Scheduled here</span>
            <IconButton label="Close" onClick={() => setPop(null)}>
              <X size={14} />
            </IconButton>
          </div>
          <ul className="flex flex-col">
            {queued.map((q) => (
              <li key={q.id} className="flex items-center gap-2.5 rounded-lg px-2 py-1.5">
                <Clock size={14} className="shrink-0 text-muted-foreground" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px]">{q.text}</div>
                  <div className="text-[11.5px] text-muted-foreground tabular">{q.late ? "Couldn't send yet, trying again" : when(q.at)}</div>
                </div>
                <IconButton label={`Don't send "${q.text.length > 30 ? `${q.text.slice(0, 30).trimEnd()}…` : q.text}"`} onClick={() => void cancel(q.id)}>
                  <X size={14} />
                </IconButton>
              </li>
            ))}
          </ul>
        </div>
      )}
      {queued.length > 0 && pop !== "scheduled" && (
        <button
          type="button"
          onClick={() => setPop("scheduled")}
          className="press mb-2 flex h-7 cursor-pointer items-center gap-1.5 rounded-full bg-muted px-3 text-[12px] font-medium text-muted-foreground transition-colors hover:text-foreground"
        >
          <Clock size={13} />
          <span className="tabular">{queued.length === 1 ? `1 message scheduled for ${when(queued[0].at)}` : `${queued.length} messages scheduled`}</span>
        </button>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
        className="rounded-2xl bg-[var(--composer)] shadow-[var(--composer-shadow)]"
      >
        {banner}
        {files.length > 0 && <Tray files={files} onRemove={(i) => setFiles((f) => f.filter((_, j) => j !== i))} />}
        {hasSel && (
          <div role="toolbar" aria-label="Formatting" className="flex items-center gap-0.5 border-b border-line px-2 py-1">
            <Fmt label="Bold (Ctrl+B)" onClick={() => wrap("*", "*")}>
              <Bold size={15} />
            </Fmt>
            <Fmt label="Italic (Ctrl+I)" onClick={() => wrap("_", "_")}>
              <Italic size={15} />
            </Fmt>
            <Fmt label="Code (Ctrl+E)" onClick={() => wrap("`", "`")}>
              <Code size={15} />
            </Fmt>
            <Fmt label="Code block (Ctrl+Shift+C)" onClick={codeBlock}>
              <SquareCode size={15} />
            </Fmt>
            <Fmt label="Quote" onClick={quote}>
              <Quote size={15} />
            </Fmt>
          </div>
        )}
        <div className="flex items-end gap-1 p-2">
          {canAttach && !editing && (
            <>
              <IconButton label="Attach files" type="button" onClick={() => fileInput.current?.click()}>
                <Paperclip size={17} />
              </IconButton>
              <input
                ref={fileInput}
                type="file"
                multiple
                className="hidden"
                onChange={(e) => {
                  addFiles(Array.from(e.target.files ?? []));
                  e.target.value = "";
                }}
              />
            </>
          )}
          {!editing && extra}
          <textarea
            ref={input}
            value={text}
            rows={rows}
            autoFocus={autoFocus}
            onChange={(e) => {
              setText(e.target.value);
              setSel([e.target.selectionStart, e.target.selectionEnd]);
              setDismissed(null);
            }}
            onSelect={(e) => setSel([e.currentTarget.selectionStart, e.currentTarget.selectionEnd])}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            onKeyDown={(e) => {
              const mod = e.ctrlKey || e.metaKey;
              const k = e.key.toLowerCase();
              if (suggest && !pop) {
                const n = suggest.items.length;
                if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                  e.preventDefault();
                  setActive((a) => (a + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
                  return;
                }
                if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
                  e.preventDefault();
                  choose(Math.min(active, n - 1));
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setDismissed(before);
                  return;
                }
              }
              if (mod && e.shiftKey && k === "c") {
                e.preventDefault();
                e.stopPropagation();
                codeBlock();
              } else if (mod && !e.shiftKey && k === "b") {
                e.preventDefault();
                e.stopPropagation();
                wrap("*", "*");
              } else if (mod && !e.shiftKey && k === "i") {
                e.preventDefault();
                e.stopPropagation();
                wrap("_", "_");
              } else if (mod && !e.shiftKey && k === "e") {
                e.preventDefault();
                e.stopPropagation();
                wrap("`", "`");
              } else if (e.key === "Enter" && !e.shiftKey) {
                // Inside an open code block Enter is a new line; Ctrl+Enter still sends.
                const open = (before.match(/```/g) ?? []).length % 2 === 1;
                if (open && !mod) return;
                e.preventDefault();
                void send();
              } else if (e.key === "Escape") {
                if (pop) setPop(null);
                else {
                  setFiles([]);
                  onEscape?.();
                }
              } else if (e.key === "ArrowUp" && !text && onArrowUpEmpty) {
                e.preventDefault();
                onArrowUpEmpty();
              }
            }}
            onPaste={(e) => {
              const f = Array.from(e.clipboardData.files);
              if (f.length && canAttach && !editing) {
                e.preventDefault();
                addFiles(f);
              }
            }}
            placeholder={files.length ? "Add a caption (optional)" : placeholder}
            aria-label="Message"
            aria-autocomplete="list"
            aria-controls={suggest ? "composer-suggest" : undefined}
            aria-expanded={!!suggest}
            className="max-h-56 min-h-9 min-w-0 flex-1 resize-none bg-transparent px-1.5 py-2 text-[length:var(--msg-size)] leading-relaxed outline-none placeholder:truncate placeholder:text-muted-foreground"
          />
          <IconButton label="Emoji" type="button" aria-pressed={pop === "emoji"} onClick={() => setPop((p) => (p === "emoji" ? null : "emoji"))}>
            <Smile size={17} />
          </IconButton>
          {schedule && !editing && (
            <IconButton label="Send later" type="button" aria-pressed={pop === "schedule"} onClick={() => setPop((p) => (p === "schedule" ? null : "schedule"))}>
              <Clock size={17} />
            </IconButton>
          )}
          <button
            type="submit"
            disabled={sending || (!text.trim() && !files.length)}
            aria-label={editing ? "Save edit" : "Send"}
            className="press grid size-9 shrink-0 cursor-pointer place-items-center rounded-full bg-primary text-primary-foreground disabled:cursor-not-allowed disabled:opacity-40"
          >
            {editing ? <Check size={17} /> : <SendHorizontal size={17} />}
          </button>
        </div>
      </form>
    </div>
  );
}

/** A toolbar button that keeps the text selected while you click it. */
function Fmt({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <IconButton label={label} type="button" className="size-7" onMouseDown={(e) => e.preventDefault()} onClick={onClick}>
      {children}
    </IconButton>
  );
}

/** Files waiting to go, with a thumbnail for images (drawn from your own file, here). */
function Tray({ files, onRemove }: { files: File[]; onRemove: (i: number) => void }) {
  const urls = useMemo(() => files.map((f) => (f.type.startsWith("image/") ? URL.createObjectURL(f) : null)), [files]);
  useEffect(() => () => urls.forEach((u) => u && URL.revokeObjectURL(u)), [urls]);
  const total = files.reduce((n, f) => n + f.size, 0);
  return (
    <div className="border-b border-line px-3 pt-2.5 pb-2" aria-label="Files to send">
      <div className="flex gap-2 overflow-x-auto pb-1">
        {files.map((f, i) => (
          <div key={`${f.name}-${f.size}-${i}`} className="relative flex w-44 shrink-0 items-center gap-2 rounded-xl bg-pane p-1.5 pr-7">
            {urls[i] ? (
              <img src={urls[i] as string} alt="" className="size-10 shrink-0 rounded-lg object-cover" />
            ) : (
              <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
                <FileText size={17} />
              </span>
            )}
            <div className="min-w-0">
              <div className="truncate text-[12.5px] font-medium">{f.name}</div>
              <div className="text-[11.5px] text-muted-foreground tabular">{bytes(f.size)}</div>
            </div>
            <button
              type="button"
              onClick={() => onRemove(i)}
              aria-label={`Remove ${f.name}`}
              className="absolute top-1 right-1 grid size-5 cursor-pointer place-items-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <X size={12} />
            </button>
          </div>
        ))}
      </div>
      <div className="pt-1 text-[11.5px] text-muted-foreground tabular">
        {files.length === 1 ? "1 file" : `${files.length} files`}, {bytes(total)}, encrypted before it leaves this computer
      </div>
    </div>
  );
}

function usePopoverClose(onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const out = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", out);
    return () => window.removeEventListener("mousedown", out);
  }, [onClose]);
  return ref;
}

function EmojiPicker({ onPick, onClose }: { onPick: (c: string) => void; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [cat, setCat] = useState(0);
  const ref = usePopoverClose(onClose);
  const shown = q.trim() ? findEmoji(q.trim().toLowerCase(), 64) : EMOJI[cat].items;
  return (
    <div ref={ref} role="dialog" aria-label="Emoji" className="absolute right-0 bottom-full z-20 mb-2 w-[min(344px,100%)] rounded-xl bg-pane p-2 shadow-float">
      <input
        autoFocus
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          }
          if (e.key === "Enter" && shown[0]) {
            e.preventDefault();
            onPick(shown[0].c);
            onClose();
          }
        }}
        placeholder="Search emoji"
        aria-label="Search emoji"
        spellCheck={false}
        className="mb-2 h-8 w-full rounded-lg bg-muted px-2.5 text-[13px] outline-none placeholder:text-muted-foreground"
      />
      {!q.trim() && (
        <div role="tablist" aria-label="Emoji groups" className="mb-1 flex gap-0.5 overflow-x-auto">
          {EMOJI.map((g, i) => (
            <button
              key={g.name}
              type="button"
              role="tab"
              aria-selected={cat === i}
              aria-label={g.name}
              title={g.name}
              onClick={() => setCat(i)}
              className={`grid size-8 shrink-0 cursor-pointer place-items-center rounded-lg text-[16px] transition-colors ${cat === i ? "bg-muted" : "hover:bg-muted/60"}`}
            >
              {g.items[0].c}
            </button>
          ))}
        </div>
      )}
      <div className="px-1 pb-1 text-[11.5px] font-medium text-muted-foreground">{q.trim() ? "Results" : EMOJI[cat].name}</div>
      <div className="grid max-h-52 grid-cols-8 gap-0.5 overflow-y-auto">
        {shown.map((e) => (
          <button
            key={e.n}
            type="button"
            title={`:${e.n}:`}
            aria-label={e.n.replace(/_/g, " ")}
            onMouseDown={(ev) => ev.preventDefault()}
            onClick={() => {
              onPick(e.c);
              onClose();
            }}
            className="grid size-9 cursor-pointer place-items-center rounded-lg text-[19px] hover:bg-muted"
          >
            {e.c}
          </button>
        ))}
        {shown.length === 0 && <p className="col-span-8 py-5 text-center text-[12.5px] text-muted-foreground">No emoji called "{q.trim()}".</p>}
      </div>
    </div>
  );
}

function SchedulePicker({ onPick, onClose, empty, withFiles }: { onPick: (at: number) => void; onClose: () => void; empty: boolean; withFiles: boolean }) {
  const [custom, setCustom] = useState("");
  const ref = usePopoverClose(onClose);
  const now = new Date();
  const nowSec = Math.floor(now.getTime() / 1000);
  const atHour = (days: number, hour: number) => {
    const d = new Date(now);
    d.setDate(d.getDate() + days);
    d.setHours(hour, 0, 0, 0);
    return Math.floor(d.getTime() / 1000);
  };
  const options: [string, number][] = [
    ["In 1 hour", nowSec + 3600],
    ...(now.getHours() < 17 ? [["This evening, 18:00", atHour(0, 18)] as [string, number]] : []),
    ["Tomorrow, 09:00", atHour(1, 9)],
    ["Monday, 09:00", atHour(((8 - now.getDay()) % 7) || 7, 9)],
  ];
  const customAt = custom ? Math.floor(new Date(custom).getTime() / 1000) : 0;
  const customOk = customAt >= nowSec + 60 && customAt <= nowSec + 30 * 86_400;
  return (
    <div ref={ref} role="dialog" aria-label="Send later" className="absolute right-0 bottom-full z-20 mb-2 w-[min(304px,100%)] rounded-xl bg-pane p-2 shadow-float">
      <div className="flex items-center justify-between px-2 pt-1 pb-1">
        <span className="text-[12.5px] font-semibold">Send later</span>
        <IconButton label="Close" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </div>
      {empty || withFiles ? (
        <p className="px-2 pb-2 text-[12.5px] leading-relaxed text-muted-foreground">
          {withFiles ? "Files can't be scheduled yet. Send them now, or remove them to schedule the text." : "Write the message first, then pick when it goes."}
        </p>
      ) : (
        <>
          {options.map(([label, at]) => (
            <button key={label} type="button" onClick={() => onPick(at)} className="flex h-9 w-full cursor-pointer items-center justify-between rounded-lg px-2 text-left text-[13.5px] transition-colors hover:bg-muted">
              {label}
              <span className="text-[12px] text-muted-foreground tabular">{new Date(at * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" })}</span>
            </button>
          ))}
          <div className="mt-1 flex items-center gap-2 border-t border-line px-2 pt-2">
            <input
              type="datetime-local"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              aria-label="Pick a date and time"
              className="h-8 min-w-0 flex-1 rounded-lg bg-muted px-2 text-[12.5px] outline-none"
            />
            <button
              type="button"
              disabled={!customOk}
              onClick={() => onPick(customAt)}
              className="press h-8 cursor-pointer rounded-full bg-primary px-3 text-[12.5px] font-medium text-primary-foreground disabled:cursor-not-allowed disabled:opacity-40"
            >
              Schedule
            </button>
          </div>
          {custom && !customOk && <p className="px-2 pt-1.5 text-[11.5px] text-destructive">Pick a time from a minute to 30 days from now.</p>}
          <p className="px-2 pt-2 pb-1 text-[11.5px] leading-relaxed text-muted-foreground">Kept encrypted on this device and sent when it's due, while Sotto is open and unlocked.</p>
        </>
      )}
    </div>
  );
}
