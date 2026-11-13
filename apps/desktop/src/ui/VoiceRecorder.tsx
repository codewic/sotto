import { Pause, Play, SendHorizontal, Square, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { RiskMode } from "../types";
import { Button, IconButton } from "./Button";
import { Dialog } from "./Dialog";
import { clock } from "./Media";
import { useToast } from "./toast";

/** Longest voice note: 10 minutes. */
const MAX_MS = 10 * 60 * 1000;
const ACK = "sotto.voice-note-ack";

type State =
  | { phase: "idle" }
  | { phase: "recording"; started: number; elapsed: number; levels: number[] }
  | { phase: "review"; file: File; url: string; duration: number };

/**
 * Record a voice note in the app. The microphone is opened only while recording, audio only.
 * The note leaves as an ordinary file; the other side plays a copy rebuilt in its sandbox.
 */
export function useVoiceRecorder(risk: RiskMode | undefined) {
  const toast = useToast();
  const [state, setState] = useState<State>({ phase: "idle" });
  const [notice, setNotice] = useState<null | "high" | "extreme">(null);
  const live = useRef<{ mr: MediaRecorder; stream: MediaStream; ctx: AudioContext; timer: number; keep: boolean } | null>(null);

  const release = useCallback(() => {
    const l = live.current;
    if (!l) return;
    window.clearInterval(l.timer);
    l.stream.getTracks().forEach((t) => t.stop());
    void l.ctx.close().catch(() => {});
    live.current = null;
  }, []);
  useEffect(() => () => release(), [release]);
  useEffect(() => {
    return () => {
      if (state.phase === "review") URL.revokeObjectURL(state.url);
    };
  }, [state]);

  const begin = useCallback(async () => {
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    } catch (e) {
      const name = (e as DOMException).name;
      toast("error", name === "NotFoundError" ? "No microphone was found. Plug one in and try again." : "Sotto can't use the microphone. Allow it in your system settings, then try again.");
      return;
    }
    const type = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";
    const mr = new MediaRecorder(stream, { mimeType: type, audioBitsPerSecond: 32_000 });
    const chunks: Blob[] = [];
    const ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    ctx.createMediaStreamSource(stream).connect(analyser);
    const buf = new Uint8Array(analyser.fftSize);
    const started = Date.now();
    mr.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    mr.onstop = () => {
      const keep = live.current?.keep ?? false;
      const elapsed = Date.now() - started;
      release();
      if (!keep || elapsed < 600 || chunks.length === 0) {
        setState({ phase: "idle" });
        return;
      }
      const file = new File(chunks, "Voice note.webm", { type: "audio/webm" });
      setState({ phase: "review", file, url: URL.createObjectURL(file), duration: elapsed / 1000 });
    };
    const timer = window.setInterval(() => {
      analyser.getByteTimeDomainData(buf);
      let sum = 0;
      for (const v of buf) sum += ((v - 128) / 128) ** 2;
      const level = Math.min(1, Math.sqrt(sum / buf.length) * 4);
      const elapsed = Date.now() - started;
      setState((s) => (s.phase === "recording" ? { ...s, elapsed, levels: [...s.levels.slice(-47), level] } : s));
      if (elapsed >= MAX_MS && live.current) {
        live.current.keep = true;
        live.current.mr.stop();
      }
    }, 100);
    live.current = { mr, stream, ctx, timer, keep: false };
    mr.start(250);
    setState({ phase: "recording", started, elapsed: 0, levels: [] });
  }, [release, toast]);

  /** Press the microphone: a note about voices first in High risk (once) and Extreme (always). */
  const start = useCallback(() => {
    let acked = false;
    try {
      acked = localStorage.getItem(ACK) === "1";
    } catch {
      acked = false;
    }
    if (risk === "extreme") setNotice("extreme");
    else if (risk === "high" && !acked) setNotice("high");
    else void begin();
  }, [risk, begin]);

  const stop = useCallback((keep: boolean) => {
    const l = live.current;
    if (!l) return;
    l.keep = keep;
    l.mr.stop();
  }, []);
  const discard = useCallback(() => {
    if (live.current) stop(false);
    else setState({ phase: "idle" });
  }, [stop]);

  const dialog = (
    <Dialog
      open={!!notice}
      onClose={() => setNotice(null)}
      title={notice === "extreme" ? "Record a voice note in Extreme mode?" : "Voices can identify people"}
      description="The note travels end-to-end encrypted, but anyone who hears it could recognise your voice. If a typed message would do, it reveals less."
      width={440}
    >
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={() => setNotice(null)}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            if (notice === "high") {
              try {
                localStorage.setItem(ACK, "1");
              } catch {
                // Not remembered: the note shows again next time.
              }
            }
            setNotice(null);
            void begin();
          }}
        >
          Record voice note
        </Button>
      </div>
    </Dialog>
  );
  return { state, start, stop, discard, clear: () => setState({ phase: "idle" }), dialog };
}

export function RecordingRow({ elapsed, levels, onStop, onDiscard }: { elapsed: number; levels: number[]; onStop: () => void; onDiscard: () => void }) {
  const bars = [...Array.from({ length: Math.max(0, 48 - levels.length) }, () => 0), ...levels];
  return (
    <div className="flex items-center gap-2 p-2" role="group" aria-label="Recording a voice note">
      <IconButton label="Discard recording" type="button" onClick={onDiscard}>
        <Trash2 size={17} />
      </IconButton>
      <span className="flex shrink-0 items-center gap-2 px-1 text-[13.5px] font-medium tabular" aria-live="off">
        <span className="rec-dot size-2.5 rounded-full bg-destructive" aria-hidden="true" />
        {clock(elapsed / 1000)}
      </span>
      <div className="flex h-8 min-w-0 flex-1 items-center gap-[2px] overflow-hidden text-primary" aria-hidden="true">
        {bars.map((l, i) => (
          <span key={i} className="min-w-[2px] flex-1 rounded-full bg-current" style={{ height: `${Math.max(10, l * 100)}%`, opacity: l ? 0.9 : 0.25 }} />
        ))}
      </div>
      <button type="button" onClick={onStop} aria-label="Stop recording" className="press grid size-9 shrink-0 cursor-pointer place-items-center rounded-full bg-primary text-primary-foreground">
        <Square size={14} fill="currentColor" />
      </button>
    </div>
  );
}

export function ReviewRow({ url, duration, sending, onSend, onDiscard }: { url: string; duration: number; sending: boolean; onSend: () => void; onDiscard: () => void }) {
  const audio = useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = useState(false);
  const [t, setT] = useState(0);
  return (
    <div className="flex items-center gap-2 p-2" role="group" aria-label="Voice note, ready to send">
      <IconButton label="Discard voice note" type="button" onClick={onDiscard} disabled={sending}>
        <Trash2 size={17} />
      </IconButton>
      {/* Your own recording, so it plays as is. */}
      <audio ref={audio} src={url} onTimeUpdate={(e) => setT(e.currentTarget.currentTime)} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setT(0)} preload="auto">
        <track kind="captions" />
      </audio>
      <button
        type="button"
        onClick={() => (audio.current?.paused ? void audio.current.play() : audio.current?.pause())}
        aria-label={playing ? "Pause" : "Listen before sending"}
        className="press grid size-9 shrink-0 cursor-pointer place-items-center rounded-full bg-muted"
      >
        {playing ? <Pause size={15} fill="currentColor" /> : <Play size={15} fill="currentColor" className="translate-x-px" />}
      </button>
      <div className="relative h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-muted" aria-hidden="true">
        <span className="absolute inset-y-0 left-0 rounded-full bg-primary" style={{ width: `${duration ? Math.min(100, (t / duration) * 100) : 0}%` }} />
      </div>
      <span className="shrink-0 px-1 text-[12.5px] text-muted-foreground tabular">{clock(playing || t ? t : duration)}</span>
      <button
        type="button"
        onClick={onSend}
        disabled={sending}
        aria-label="Send voice note"
        className="press grid size-9 shrink-0 cursor-pointer place-items-center rounded-full bg-primary text-primary-foreground disabled:cursor-not-allowed disabled:opacity-40"
      >
        <SendHorizontal size={17} />
      </button>
    </div>
  );
}
