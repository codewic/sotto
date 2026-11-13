import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Mic, Pause, Play, Video, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { type ApiError, chooseSavePath, imageUrl, post } from "../api";
import { MarkIndicator } from "../brand/Mark";
import { Button } from "./Button";
import { useToast } from "./toast";

export const isVideoFile = (mime: string) => mime.startsWith("video/");
export const isAudioFile = (mime: string) => mime.startsWith("audio/");

type Kind = "audio" | "video";
type Prep =
  | { state: "ready"; kind: Kind; duration: number; width: number; height: number; peaks: number[]; playable: boolean }
  | { state: "held"; reason: string }
  | { state: "failed"; reason: string }
  | { state: "missing" };

const q = (key: string) => encodeURIComponent(key);

/** A voice note, audio file or video's clean copy, made by the agent from what left the sandbox. */
export function useMedia(key: string | null, kind: Kind, own: boolean) {
  const qc = useQueryClient();
  const prep = useQuery({
    queryKey: ["media", key],
    queryFn: () => post<Prep>("/api/media/prepare", { key, kind, own }),
    enabled: !!key,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 1,
  });
  const again = useCallback(
    async (extra: { show?: boolean; play?: boolean }) => {
      const r = await post<Prep>("/api/media/prepare", { key, kind, own, ...extra });
      qc.setQueryData(["media", key], r);
      return r;
    },
    [qc, key, kind, own],
  );
  const show = useCallback(() => again({ show: true }), [again]);
  const makePlayable = useCallback(() => again({ play: true }), [again]);
  return { prep: prep.data, show, makePlayable };
}

/** Bytes from the agent as an object URL (cached while the conversation is open). */
function useBlob(path: string | null) {
  return useQuery({ queryKey: ["media-blob", path], queryFn: () => imageUrl(path ?? ""), enabled: !!path, staleTime: Number.POSITIVE_INFINITY }).data;
}

export function clock(sec: number) {
  const s = Math.max(0, Math.round(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function Held({ icon, label, reason, onShow }: { icon: ReactNode; label: string; reason: string; onShow: () => Promise<unknown> }) {
  const [busy, setBusy] = useState(false);
  return (
    <div className="flex min-w-[250px] items-center gap-3 rounded-xl bg-[var(--bubble-inset)] p-2.5">
      <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-current/10">{icon}</span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13.5px] font-medium">{label}</div>
        <div className="text-[12px] opacity-70">Held: {reason.toLowerCase()}</div>
      </div>
      <Button
        size="sm"
        variant="primary"
        loading={busy}
        onClick={async () => {
          setBusy(true);
          await onShow().finally(() => setBusy(false));
        }}
      >
        Show
      </Button>
    </div>
  );
}

/** A voice note or audio file: play, a waveform you can click to seek, and playback speed. */
export function VoicePlayer({ mediaKey, own, name, fallback }: { mediaKey: string; own: boolean; name: string; fallback: ReactNode }) {
  const { prep, show } = useMedia(mediaKey, "audio", own);
  const [want, setWant] = useState(false);
  const src = useBlob(want && prep?.state === "ready" ? `/api/media/play?key=${q(mediaKey)}` : null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [t, setT] = useState(0);
  const [rate, setRate] = useState(1);
  const voice = /^voice note/i.test(name);

  useEffect(() => {
    if (!src) return;
    const a = new Audio(src);
    a.ontimeupdate = () => setT(a.currentTime);
    a.onended = () => {
      setPlaying(false);
      setT(0);
    };
    a.onpause = () => setPlaying(false);
    a.onplay = () => setPlaying(true);
    audio.current = a;
    void a.play().catch(() => setPlaying(false));
    return () => {
      a.pause();
      audio.current = null;
    };
  }, [src]);
  useEffect(() => {
    if (audio.current) audio.current.playbackRate = rate;
  }, [rate, src]);

  if (prep?.state === "failed" || prep?.state === "missing") return <>{fallback}</>;
  if (prep?.state === "held") return <Held icon={<Mic size={18} />} label={voice ? "Voice note" : name} reason={prep.reason} onShow={show} />;
  const ready = prep?.state === "ready" ? prep : null;
  const duration = ready?.duration ?? 0;
  const peaks = ready?.peaks.length ? ready.peaks : Array.from({ length: 64 }, () => 40);
  const progress = duration ? t / duration : 0;
  const toggle = () => {
    if (!ready) return;
    if (!want) return setWant(true);
    const a = audio.current;
    if (!a) return;
    if (a.paused) void a.play();
    else a.pause();
  };
  const seek = (fraction: number) => {
    const a = audio.current;
    if (!a || !duration) return;
    a.currentTime = Math.min(duration, Math.max(0, fraction * duration));
    setT(a.currentTime);
  };
  return (
    <div className="flex w-[min(300px,100%)] flex-col gap-1">
      {!voice && <div className="truncate text-[13px] font-medium">{name}</div>}
      <div className="flex items-center gap-2.5">
        <button
          type="button"
          onClick={toggle}
          disabled={!ready}
          aria-label={`${playing ? "Pause" : "Play"} ${voice ? "voice note" : name}, ${clock(duration)}`}
          className="press grid size-10 shrink-0 cursor-pointer place-items-center rounded-full bg-current/15 disabled:cursor-default disabled:opacity-60"
        >
          {playing ? <Pause size={17} fill="currentColor" /> : <Play size={17} fill="currentColor" className="translate-x-px" />}
        </button>
        <div
          role="slider"
          tabIndex={ready ? 0 : -1}
          aria-label="Position"
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(t)}
          aria-valuetext={`${clock(t)} of ${clock(duration)}`}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            seek((e.clientX - r.left) / r.width);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowRight") seek((t + 5) / (duration || 1));
            else if (e.key === "ArrowLeft") seek((t - 5) / (duration || 1));
            else if (e.key === " " || e.key === "Enter") toggle();
            else return;
            e.preventDefault();
          }}
          className={`flex h-8 min-w-0 flex-1 cursor-pointer items-center gap-[2px] ${ready ? "" : "photo-wait rounded"}`}
        >
          {peaks.map((p, i) => (
            <span
              key={i}
              aria-hidden="true"
              className="min-w-[2px] flex-1 rounded-full bg-current"
              style={{ height: `${Math.max(12, (p / 255) * 100)}%`, opacity: i / peaks.length < progress ? 1 : 0.35 }}
            />
          ))}
        </div>
      </div>
      <div className="flex items-center gap-2 pl-[50px] text-[11.5px] tabular opacity-75">
        <span>{playing || t > 0 ? clock(t) : clock(duration)}</span>
        {(playing || t > 0) && (
          <button
            type="button"
            onClick={() => setRate((r) => (r === 1 ? 1.5 : r === 1.5 ? 2 : 1))}
            aria-label={`Playback speed ${rate} times`}
            className="press cursor-pointer rounded-full bg-current/15 px-1.5 font-semibold"
          >
            {rate}×
          </button>
        )}
      </div>
    </div>
  );
}

/** On-screen size for a poster: its own shape inside a max width and height. */
function fit(w: number, h: number, maxW: number, maxH: number) {
  const s = Math.min(maxW / w, maxH / h, 1);
  return { width: Math.max(120, Math.round(w * s)), height: Math.max(80, Math.round(h * s)) };
}

/** A video inside a message: its clean poster frame, a play button and its length. */
export function InlineVideo({ mediaKey, own, name, onOpen, fallback }: { mediaKey: string; own: boolean; name: string; onOpen: () => void; fallback: ReactNode }) {
  const { prep, show } = useMedia(mediaKey, "video", own);
  const poster = useBlob(prep?.state === "ready" ? `/api/media/poster?key=${q(mediaKey)}` : null);
  if (prep?.state === "failed" || prep?.state === "missing") return <>{fallback}</>;
  if (prep?.state === "held") return <Held icon={<Video size={18} />} label={name} reason={prep.reason} onShow={show} />;
  const box = prep?.state === "ready" ? fit(prep.width, prep.height, 320, 360) : { width: 240, height: 160 };
  return (
    <button
      type="button"
      onClick={onOpen}
      disabled={!poster}
      aria-label={prep?.state === "ready" ? `Play ${name}, ${clock(prep.duration)}` : `${name}, getting ready`}
      className="group/video relative block cursor-pointer overflow-hidden rounded-xl bg-current/10 disabled:cursor-default"
      style={{ width: box.width, height: box.height, maxWidth: "100%" }}
    >
      {poster ? <img src={poster} alt="" draggable={false} className="size-full object-cover" /> : <span className="photo-wait absolute inset-0" aria-hidden="true" />}
      {poster && (
        <>
          <span className="absolute inset-0 grid place-items-center">
            <span className="grid size-12 place-items-center rounded-full bg-[var(--scrim)] text-scrim-fg transition-transform duration-150 group-hover/video:scale-105">
              <Play size={20} fill="currentColor" className="translate-x-0.5" />
            </span>
          </span>
          {prep?.state === "ready" && <span className="absolute bottom-2 left-2 rounded-full bg-[var(--scrim)] px-2 py-0.5 text-[11.5px] font-medium text-scrim-fg tabular">{clock(prep.duration)}</span>}
        </>
      )}
    </button>
  );
}

export type VideoItem = { key: string; own: boolean; name: string; who: string; at: number };

/** Full-screen video. The first time, a safe copy is made from frames that left the sandbox. */
export function VideoViewer({ item, onClose }: { item: VideoItem; onClose: () => void }) {
  const toast = useToast();
  const { prep, makePlayable } = useMedia(item.key, "video", item.own);
  const [error, setError] = useState<string | null>(null);
  const playable = prep?.state === "ready" && prep.playable;
  const src = useBlob(playable ? `/api/media/play?key=${q(item.key)}` : null);
  const closeBtn = useRef<HTMLButtonElement>(null);
  const asked = useRef(false);

  useEffect(() => {
    if (prep?.state !== "ready" || prep.playable || asked.current) return;
    asked.current = true;
    makePlayable()
      .then((r) => setError(r.state === "failed" ? r.reason : null))
      .catch((e: ApiError) => setError(e.message));
  }, [prep, makePlayable]);
  useEffect(() => {
    const back = document.activeElement as HTMLElement | null;
    closeBtn.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      back?.focus();
    };
  }, [onClose]);

  async function save() {
    const path = await chooseSavePath(`${item.name.replace(/\.[^.]+$/, "")}.mp4`);
    if (!path) return;
    try {
      await post("/api/media/save", { key: item.key, path });
      toast("info", item.own ? "Saved a copy. It's outside Sotto's encrypted storage now." : "Saved a clean copy. It's outside Sotto's encrypted storage now.");
    } catch (e) {
      toast("error", (e as ApiError).message);
    }
  }

  const when = new Date(item.at).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  const btn = "press grid size-10 cursor-pointer place-items-center rounded-full text-scrim-fg/85 hover:bg-scrim-fg/15 hover:text-scrim-fg disabled:cursor-default disabled:opacity-35";
  return (
    <div role="dialog" aria-modal="true" aria-label={`Video: ${item.name}`} className="viewer-in fixed inset-0 flex flex-col bg-[var(--scrim)] text-scrim-fg" style={{ zIndex: "var(--z-modal)" }}>
      <div className="flex h-14 shrink-0 items-center gap-1.5 px-4">
        <div className="mr-2 min-w-0 flex-1">
          <div className="truncate text-[14px] font-medium">{item.name}</div>
          <div className="truncate text-[12px] text-scrim-fg/65 tabular">
            {item.who} · {when}
          </div>
        </div>
        <button type="button" onClick={() => void save()} disabled={!src} aria-label="Save a copy" title="Save a copy" className={btn}>
          <Download size={18} />
        </button>
        <button ref={closeBtn} type="button" onClick={onClose} aria-label="Close (Esc)" title="Close (Esc)" className={btn}>
          <X size={19} />
        </button>
      </div>
      <div className="relative grid min-h-0 flex-1 place-items-center px-6 pb-6">
        {src ? (
          <video src={src} controls autoPlay playsInline className="max-h-full max-w-full rounded-lg">
            <track kind="captions" />
          </video>
        ) : error ? (
          <div className="max-w-[420px] text-center">
            <p className="text-[15px] font-medium">This video can't be played safely.</p>
            <p className="mt-1.5 text-[13px] text-scrim-fg/70">{error}</p>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-3 text-center">
            <MarkIndicator size={28} inherit />
            <p className="text-[14px] font-medium">Preparing a safe copy to play…</p>
            <p className="max-w-[360px] text-[12.5px] text-scrim-fg/65">It's opened in the sandbox and rebuilt frame by frame, so nothing hidden in the file can reach this computer. Longer videos take a little while.</p>
          </div>
        )}
      </div>
    </div>
  );
}
