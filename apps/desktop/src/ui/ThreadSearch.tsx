import { ChevronDown, ChevronUp, Search, X } from "lucide-react";
import { type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { IconButton } from "./Button";

/**
 * Find inside one thread (a group or a channel): matches are the items whose text contains the
 * query, newest first, and the current one is scrolled into view. `attr` is the data attribute
 * each item carries (`data-mid` or `data-pid`).
 */
export function useThreadSearch<T>(items: T[], text: (t: T) => string, idOf: (t: T) => string, scroller: RefObject<HTMLElement | null>, attr: string) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!open || q.length < 2) return [];
    return items
      .filter((t) => text(t).toLowerCase().includes(q))
      .map(idOf)
      .reverse();
  }, [items, query, open, text, idOf]);
  const current = matches.length ? matches[Math.min(index, matches.length - 1)] : null;

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    if (!current) return;
    const el = scroller.current?.querySelector(`[${attr}="${current}"]`);
    el?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [current, scroller, attr]);

  // Ctrl/Cmd+F opens it while the thread is on screen.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return {
    open,
    query,
    matches,
    index: Math.min(index, Math.max(0, matches.length - 1)),
    current,
    setQuery,
    show: () => setOpen(true),
    close: () => {
      setOpen(false);
      setQuery("");
    },
    // Up goes to older matches, down to newer ones.
    older: () => setIndex((i) => (matches.length ? (i + 1) % matches.length : 0)),
    newer: () => setIndex((i) => (matches.length ? (i - 1 + matches.length) % matches.length : 0)),
  };
}

export function SearchBar({ search, placeholder }: { search: ReturnType<typeof useThreadSearch>; placeholder: string }) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const q = search.query.trim();
  const status = q.length < 2 ? "" : search.matches.length ? `${search.index + 1} of ${search.matches.length}` : "No matches";
  return (
    <div className="flex h-12 shrink-0 items-center gap-2 px-4" role="search">
      <div className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-full bg-muted px-3.5">
        <Search size={15} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <input
          ref={input}
          value={search.query}
          onChange={(e) => search.setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") search.close();
            if (e.key === "Enter") {
              e.preventDefault();
              if (e.shiftKey) search.newer();
              else search.older();
            }
          }}
          placeholder={placeholder}
          aria-label={placeholder}
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent text-[13.5px] outline-none placeholder:text-muted-foreground"
        />
        <span className="shrink-0 text-[12px] text-muted-foreground tabular" aria-live="polite">
          {status}
        </span>
      </div>
      <IconButton label="Older match" onClick={search.older} disabled={search.matches.length < 2}>
        <ChevronUp size={17} />
      </IconButton>
      <IconButton label="Newer match" onClick={search.newer} disabled={search.matches.length < 2}>
        <ChevronDown size={17} />
      </IconButton>
      <IconButton label="Close search" onClick={search.close}>
        <X size={17} />
      </IconButton>
    </div>
  );
}
