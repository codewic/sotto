import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronLeft, ChevronRight, Download, FileText, ImageIcon, Minus, Plus, X } from "lucide-react";
import { type ButtonHTMLAttributes, type ReactNode, type Ref, useCallback, useEffect, useRef, useState } from "react";
import { type ApiError, chooseSavePath, imageUrl, post } from "../api";
import type { Message } from "../types";
import { Button } from "./Button";
import { Rich } from "./Rich";
import { useToast } from "./toast";

/** Images Sotto can clean in its sandbox. Others (SVG, HEIC) travel as ordinary files. */
export const isPhotoFile = (mime: string) => mime.startsWith("image/") && !/svg|heic|heif/i.test(mime);

type Prep =
  | { state: "ready"; width: number; height: number; mime: string }
  | { state: "held"; reason: string }
  | { state: "failed"; reason: string }
  | { state: "missing" };

/**
 * A photo's clean copy: the agent cleans it in the sandbox the first time it's asked (unless
 * it's held for Show), then serves only that copy.
 */
export function usePhoto(key: string | null, own: boolean) {
  const qc = useQueryClient();
  const prep = useQuery({
    queryKey: ["photo", key],
    queryFn: () => post<Prep>("/api/photos/prepare", { key, own }),
    enabled: !!key,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 1,
  });
  const ready = prep.data?.state === "ready";
  const img = useQuery({
    queryKey: ["photo-img", key],
    queryFn: () => imageUrl(`/api/photos/image?key=${encodeURIComponent(key ?? "")}`),
    enabled: !!key && ready,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const show = useCallback(async () => {
    const r = await post<Prep>("/api/photos/prepare", { key, own, show: true });
    qc.setQueryData(["photo", key], r);
  }, [qc, key, own]);
  return { prep: prep.data, url: img.data, show };
}

/** On-screen size: the photo's own shape, fitting inside a max width and height. */
function fit(w: number, h: number, maxW: number, maxH: number) {
  const s = Math.min(maxW / w, maxH / h, 1);
  return { width: Math.max(96, Math.round(w * s)), height: Math.max(64, Math.round(h * s)) };
}

/**
 * A photo inside a message. While it's being cleaned it holds its place; when it's held it
 * offers Show; when it can't be cleaned it falls back to `fallback` (the ordinary file card).
 */
export function InlinePhoto({ photoKey, own, name, onOpen, fallback }: { photoKey: string; own: boolean; name: string; onOpen: () => void; fallback: ReactNode }) {
  const { prep, url, show } = usePhoto(photoKey, own);
  const [showing, setShowing] = useState(false);
  if (prep?.state === "failed" || prep?.state === "missing") return <>{fallback}</>;
  if (prep?.state === "held") {
    return (
      <div className="flex min-w-[250px] items-center gap-3 rounded-xl bg-[var(--bubble-inset)] p-2.5">
        <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-current/10">
          <ImageIcon size={18} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px] font-medium">{name}</div>
          <div className="text-[12px] opacity-70">Photo held: {prep.reason.toLowerCase()}</div>
        </div>
        <Button
          size="sm"
          variant="primary"
          loading={showing}
          onClick={async () => {
            setShowing(true);
            await show().finally(() => setShowing(false));
          }}
        >
          Show
        </Button>
      </div>
    );
  }
  const box = prep?.state === "ready" ? fit(prep.width, prep.height, 320, 360) : { width: 240, height: 180 };
  return (
    <button
      type="button"
      onClick={onOpen}
      disabled={!url}
      aria-label={url ? `Open ${name}` : `${name}, cleaning`}
      className="relative block cursor-zoom-in overflow-hidden rounded-xl bg-current/10 disabled:cursor-default"
      style={{ width: box.width, height: box.height, maxWidth: "100%" }}
    >
      {url ? <img src={url} alt={name} draggable={false} className="size-full object-cover" /> : <span className="photo-wait absolute inset-0" aria-hidden="true" />}
    </button>
  );
}

/** A message that can sit in an album: a photo, received or yours, with no reply or reactions. */
const albumable = (m: Message, i: number) =>
  m.kind === "file" && !!m.file && isPhotoFile(m.file.mime) && (m.outgoing || m.file.status === "ready") && !m.reply_to && m.reactions.length === 0 && (i === 0 || !m.file.caption);

export type RunItem = { kind: "one"; m: Message; i: number } | { kind: "album"; ms: Message[]; i: number };

/** Consecutive photos in a run (a caption only on the first) become one album. */
export function withAlbums(run: Message[]): RunItem[] {
  const out: RunItem[] = [];
  let k = 0;
  while (k < run.length) {
    let j = k;
    while (j < run.length && albumable(run[j], j - k)) j++;
    if (j - k >= 2) {
      out.push({ kind: "album", ms: run.slice(k, j), i: k });
      k = j;
    } else {
      out.push({ kind: "one", m: run[k], i: k });
      k++;
    }
  }
  return out;
}

/** Several photos sent together, as one grid. Each tile opens the viewer at that photo. */
export function Album({
  ms,
  keyOf,
  mine,
  meta,
  onOpen,
  onMenu,
}: {
  ms: Message[];
  keyOf: (m: Message) => string;
  mine: boolean;
  meta: ReactNode;
  onOpen: (key: string) => void;
  onMenu: (m: Message, x: number, y: number) => void;
}) {
  const shown = ms.slice(0, 9);
  const more = ms.length - shown.length;
  const cols = ms.length <= 4 ? 2 : 3;
  const caption = ms[0].file?.caption;
  return (
    <div
      data-mid={ms[0].id}
      className={`relative flex w-[min(340px,100%)] flex-col gap-1.5 rounded-2xl p-1.5 pb-5 ${mine ? "self-end bg-[var(--bubble-mine)] text-[var(--bubble-mine-fg)]" : "self-start bg-[var(--bubble-theirs)] text-foreground shadow-[var(--bubble-theirs-shadow)]"}`}
    >
      <div className="grid gap-1 overflow-hidden rounded-xl" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }} aria-label={`${ms.length} photos`}>
        {shown.map((m, i) => (
          <Tile
            key={m.id}
            m={m}
            photoKey={keyOf(m)}
            own={mine}
            wide={ms.length === 3 && i === 0}
            more={i === shown.length - 1 && more > 0 ? more : 0}
            onOpen={() => onOpen(keyOf(m))}
            onMenu={(x, y) => onMenu(m, x, y)}
          />
        ))}
      </div>
      {caption && <div className="px-1.5 text-[length:var(--msg-size)] leading-relaxed break-words whitespace-pre-wrap"><Rich text={caption} /></div>}
      <span className={`absolute right-2.5 bottom-1.5 flex items-center gap-1 text-[11px] tabular ${mine ? "text-[var(--bubble-mine-muted)]" : "text-muted-foreground"}`}>{meta}</span>
    </div>
  );
}

function Tile({ m, photoKey, own, wide, more, onOpen, onMenu }: { m: Message; photoKey: string; own: boolean; wide: boolean; more: number; onOpen: () => void; onMenu: (x: number, y: number) => void }) {
  const { prep, url, show } = usePhoto(photoKey, own);
  const name = m.file?.name ?? "Photo";
  const cls = `relative overflow-hidden bg-current/10 ${wide ? "col-span-2 aspect-[2/1]" : "aspect-square"}`;
  const menu = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    onMenu(e.clientX, e.clientY);
  };
  if (prep?.state === "held") {
    return (
      <div className={`${cls} grid place-items-center`} onContextMenu={menu}>
        <button type="button" onClick={() => void show()} className="press flex cursor-pointer flex-col items-center gap-1 rounded-lg px-2 py-1.5 text-[12px] font-medium hover:bg-current/10">
          <ImageIcon size={18} />
          Show
        </button>
      </div>
    );
  }
  if (prep?.state === "failed" || prep?.state === "missing") {
    return (
      <div className={`${cls} flex flex-col items-center justify-center gap-1 p-2 text-center`} onContextMenu={menu} title={prep.state === "failed" ? prep.reason : undefined}>
        <FileText size={18} className="opacity-70" />
        <span className="line-clamp-2 text-[11.5px] opacity-80">{name}</span>
      </div>
    );
  }
  return (
    <button type="button" onClick={onOpen} onContextMenu={menu} disabled={!url} aria-label={url ? `Open ${name}` : `${name}, cleaning`} className={`${cls} block cursor-zoom-in disabled:cursor-default`}>
      {url ? <img src={url} alt={name} draggable={false} className="size-full object-cover" /> : <span className="photo-wait absolute inset-0" aria-hidden="true" />}
      {more > 0 && <span className="absolute inset-0 grid place-items-center bg-[var(--scrim)] text-[18px] font-semibold text-scrim-fg tabular">+{more}</span>}
    </button>
  );
}

export type ViewerItem = { key: string; own: boolean; name: string; caption?: string; who: string; at: number };

/** Full-screen photos: zoom, pan, next and previous, and save a clean copy. */
export function PhotoViewer({ items, start, onClose }: { items: ViewerItem[]; start: number; onClose: () => void }) {
  const toast = useToast();
  const [i, setI] = useState(start);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ x: number; y: number; px: number; py: number } | null>(null);
  const closeBtn = useRef<HTMLButtonElement>(null);
  const item = items[Math.min(i, items.length - 1)];
  const { prep, url } = usePhoto(item?.key ?? null, item?.own ?? false);

  const go = useCallback(
    (d: number) => {
      setI((n) => Math.min(items.length - 1, Math.max(0, n + d)));
      setZoom(1);
      setPan({ x: 0, y: 0 });
    },
    [items.length],
  );
  const zoomTo = useCallback((z: number) => {
    const next = Math.min(5, Math.max(1, z));
    setZoom(next < 1.01 ? 1 : next);
    if (next < 1.01) setPan({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    const back = document.activeElement as HTMLElement | null;
    closeBtn.current?.focus();
    return () => back?.focus();
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowLeft") go(-1);
      else if (e.key === "ArrowRight") go(1);
      else if (e.key === "+" || e.key === "=") zoomTo(zoom * 1.5);
      else if (e.key === "-") zoomTo(zoom / 1.5);
      else if (e.key === "0") zoomTo(1);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [go, onClose, zoom, zoomTo]);

  async function save() {
    if (!item) return;
    const path = await chooseSavePath(item.name);
    if (!path) return;
    try {
      await post("/api/photos/save", { key: item.key, path });
      toast("info", item.own ? "Saved a copy. It's outside Sotto's encrypted storage now." : "Saved a clean copy. It's outside Sotto's encrypted storage now.");
    } catch (e) {
      toast("error", (e as ApiError).message);
    }
  }

  if (!item) return null;
  const when = new Date(item.at).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  return (
    <div role="dialog" aria-modal="true" aria-label={`Photo: ${item.name}`} className="viewer-in fixed inset-0 flex flex-col bg-[var(--scrim)] text-scrim-fg" style={{ zIndex: "var(--z-modal)" }}>
      <div className="flex h-14 shrink-0 items-center gap-1.5 px-4">
        <div className="mr-2 min-w-0 flex-1">
          <div className="truncate text-[14px] font-medium">{item.name}</div>
          <div className="truncate text-[12px] text-scrim-fg/65 tabular">
            {item.who} · {when}
            {items.length > 1 && ` · ${i + 1} of ${items.length}`}
          </div>
        </div>
        <ViewerButton label="Zoom out (−)" onClick={() => zoomTo(zoom / 1.5)} disabled={zoom <= 1}>
          <Minus size={18} />
        </ViewerButton>
        <span className="w-11 text-center text-[12.5px] text-scrim-fg/70 tabular">{Math.round(zoom * 100)}%</span>
        <ViewerButton label="Zoom in (+)" onClick={() => zoomTo(zoom * 1.5)} disabled={zoom >= 5}>
          <Plus size={18} />
        </ViewerButton>
        <ViewerButton label="Save a copy" onClick={() => void save()} disabled={!url}>
          <Download size={18} />
        </ViewerButton>
        <ViewerButton label="Close (Esc)" onClick={onClose} ref={closeBtn}>
          <X size={19} />
        </ViewerButton>
      </div>
      <div
        className={`relative min-h-0 flex-1 touch-none overflow-hidden select-none ${zoom > 1 ? (dragging ? "cursor-grabbing" : "cursor-grab") : ""}`}
        onWheel={(e) => zoomTo(zoom * (e.deltaY < 0 ? 1.15 : 1 / 1.15))}
        onDoubleClick={() => zoomTo(zoom > 1 ? 1 : 2.5)}
        onPointerDown={(e) => {
          if (zoom <= 1) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { x: e.clientX, y: e.clientY, px: pan.x, py: pan.y };
          setDragging(true);
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (d) setPan({ x: d.px + (e.clientX - d.x) / zoom, y: d.py + (e.clientY - d.y) / zoom });
        }}
        onPointerUp={() => {
          drag.current = null;
          setDragging(false);
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget && zoom === 1) onClose();
        }}
      >
        {url ? (
          <img
            src={url}
            alt={item.caption || item.name}
            draggable={false}
            className="pointer-events-none absolute inset-0 m-auto max-h-[calc(100%-32px)] max-w-[calc(100%-128px)] object-contain"
            style={{ transform: `scale(${zoom}) translate(${pan.x}px, ${pan.y}px)`, transition: dragging ? "none" : "transform 180ms var(--ease-smooth)" }}
          />
        ) : (
          <div className="absolute inset-0 grid place-items-center text-[13px] text-scrim-fg/70">{prep?.state === "held" ? "This photo is held. Close the viewer and choose Show." : "Cleaning this photo…"}</div>
        )}
        {i > 0 && (
          <ViewerButton label="Previous photo (←)" onClick={() => go(-1)} className="absolute top-1/2 left-4 -translate-y-1/2 bg-scrim-fg/10">
            <ChevronLeft size={22} />
          </ViewerButton>
        )}
        {i < items.length - 1 && (
          <ViewerButton label="Next photo (→)" onClick={() => go(1)} className="absolute top-1/2 right-4 -translate-y-1/2 bg-scrim-fg/10">
            <ChevronRight size={22} />
          </ViewerButton>
        )}
      </div>
      {item.caption && <p className="mx-auto max-w-[720px] shrink-0 px-6 pb-5 text-center text-[14px] leading-relaxed text-scrim-fg/85">{item.caption}</p>}
    </div>
  );
}

function ViewerButton({ label, children, className = "", ref, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; ref?: Ref<HTMLButtonElement> }) {
  return (
    <button
      ref={ref}
      type="button"
      aria-label={label}
      title={label}
      className={`press grid size-10 shrink-0 cursor-pointer place-items-center rounded-full text-scrim-fg/85 transition-colors hover:bg-scrim-fg/15 hover:text-scrim-fg disabled:cursor-default disabled:opacity-35 ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}
