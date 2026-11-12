import { ConnectionQuality, ExternalE2EEKeyProvider, type LocalAudioTrack, type Participant, type RemoteTrack, Room, RoomEvent, Track } from "livekit-client";
import { Check, Lock, Mic, MicOff, PhoneOff, SlidersHorizontal, UserPlus } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { type ApiError, get, post, subscribe } from "../api";
import { Avatar } from "../ui/Avatar";
import { MarkIndicator } from "../brand/Mark";
import { installSdpPrivacy } from "./sdp";

type JoinInfo = { call_id: string; contact_id: string; name: string; url: string; token: string; key: string; outgoing: boolean; group: boolean; me: string };
type Person = { id: string; name: string; verified: boolean };

type Stage =
  | { kind: "loading" }
  | { kind: "waiting" } // outgoing, in the room, the other person hasn't arrived
  | { kind: "connecting" } // answering, joining the room
  | { kind: "live" }
  | { kind: "reconnecting" }
  | { kind: "ended"; text: string }
  | { kind: "error"; text: string; retry: boolean };

const ENDED: Record<string, string> = {
  finished: "Call ended",
  no_answer: "No answer",
  cancelled: "Call cancelled",
  they_declined: "They couldn't talk",
  missed: "Missed call",
  missed_busy: "Missed call",
  declined: "Call declined",
  off: "Calls are off in Extreme mode",
};

/** More than this many decryption failures in 10 seconds means the keys don't match: end the call. */
const DECRYPT_FAILURES = 25;

function clock(ms: number) {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

function bytesOf(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

/**
 * Who's speaking, worked out here from the decrypted audio. The server can't tell: the
 * audio-level header that would have told it is never negotiated (see sdp.ts).
 */
function watchSpeaking(track: MediaStreamTrack, onChange: (speaking: boolean) => void): () => void {
  let ctx: AudioContext;
  try {
    ctx = new AudioContext();
  } catch {
    return () => {};
  }
  const analyser = ctx.createAnalyser();
  analyser.fftSize = 512;
  ctx.createMediaStreamSource(new MediaStream([track])).connect(analyser);
  const buf = new Float32Array(analyser.fftSize);
  let speaking = false;
  let changedAt = 0;
  const timer = setInterval(() => {
    analyser.getFloatTimeDomainData(buf);
    let sum = 0;
    for (const v of buf) sum += v * v;
    const loud = Math.sqrt(sum / buf.length) > 0.02;
    const t = performance.now();
    // Quick to light up, slower to fade, so the ring doesn't flicker between words.
    if (loud !== speaking && t - changedAt > (loud ? 80 : 450)) {
      speaking = loud;
      onChange(loud);
    }
    if (loud === speaking) changedAt = t;
  }, 60);
  return () => {
    clearInterval(timer);
    void ctx.close();
  };
}

const QUALITY: Record<string, string> = {
  [ConnectionQuality.Excellent]: "Strong connection",
  [ConnectionQuality.Good]: "Good connection",
  [ConnectionQuality.Poor]: "Weak connection",
  [ConnectionQuality.Lost]: "Connection lost",
};

export function CallScreen({ callId }: { callId: string }) {
  const [info, setInfo] = useState<JoinInfo | null>(null);
  const [stage, setStage] = useState<Stage>({ kind: "loading" });
  const [muted, setMuted] = useState(false);
  const [encrypted, setEncrypted] = useState(false);
  // Everyone else in the room, by their random server identity; their names arrive end-to-end
  // encrypted inside the call; who's speaking is worked out from the decrypted audio.
  const [people, setPeople] = useState<string[]>([]);
  const [names, setNames] = useState<Record<string, string>>({});
  const [speakingIds, setSpeakingIds] = useState<Set<string>>(new Set());
  const [adding, setAdding] = useState(false);
  const [ringingNames, setRingingNames] = useState<string[]>([]);
  const markedGroup = useRef(false);
  const [quality, setQuality] = useState<string | null>(null);
  const [since, setSince] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now());
  const [devices, setDevices] = useState(false);
  const room = useRef<Room | null>(null);
  const audio = useRef<HTMLDivElement>(null);
  const done = useRef(false);
  const failures = useRef<number[]>([]);
  const watchers = useRef(new Map<string, () => void>());
  const stopAllWatchers = () => {
    watchers.current.forEach((stop) => stop());
    watchers.current.clear();
  };

  /** Leave the room, tell the agent (once) and close the window shortly after. */
  const finish = useCallback(
    async (text: string, tellAgent: boolean) => {
      if (done.current) return;
      done.current = true;
      setStage({ kind: "ended", text });
      const r = room.current;
      room.current = null;
      stopAllWatchers();
      await r?.disconnect().catch(() => {});
      if (tellAgent) await post(`/api/calls/${callId}/end`).catch(() => {});
      setTimeout(() => window.close(), 2200);
    },
    [callId],
  );

  const join = useCallback(async () => {
    done.current = false;
    setStage({ kind: "loading" });
    let j: JoinInfo;
    try {
      j = await get<JoinInfo>(`/api/calls/${callId}/join`);
    } catch (e) {
      setStage({ kind: "error", text: (e as ApiError).message, retry: false });
      return;
    }
    setInfo(j);
    document.title = `Call with ${j.name || "contact"}`;
    setStage(j.outgoing ? { kind: "waiting" } : { kind: "connecting" });

    // Development only: ?nopriv leaves descriptions as the browser makes them, to compare.
    if (!(import.meta.env.DEV && new URLSearchParams(location.search).has("nopriv"))) installSdpPrivacy();
    const keyProvider = new ExternalE2EEKeyProvider();
    const worker = new Worker(new URL("livekit-client/e2ee-worker", import.meta.url), { type: "module" });
    const r = new Room({
      encryption: { keyProvider, worker },
      adaptiveStream: false,
      dynacast: false,
      audioCaptureDefaults: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
    room.current = r;
    if (import.meta.env.DEV) (window as unknown as { __sottoCall: unknown }).__sottoCall = { room: r };

    // Events from a room we've since left (a retry, or the screen unmounting) mean nothing.
    const current = () => room.current === r;
    const peerArrived = () => {
      if (!current()) return;
      setStage((s) => (s.kind === "ended" || s.kind === "error" ? s : { kind: "live" }));
      setSince((t) => t ?? Date.now());
      if (j.outgoing) post(`/api/calls/${callId}/joined`).catch(() => {});
    };
    const refreshPeople = () => {
      if (!current()) return;
      const ids = [...r.remoteParticipants.values()].map((p) => p.identity);
      setPeople(ids);
      // Two or more others: a group call. Tell the agent, so one person leaving doesn't end it.
      if (ids.length >= 2 && !markedGroup.current) {
        markedGroup.current = true;
        post(`/api/calls/${callId}/group`).catch(() => {});
      }
    };
    // Share my chosen name with everyone in the call (room data is end-to-end encrypted too).
    const sayName = () => {
      if (!current()) return;
      void r.localParticipant.publishData(new TextEncoder().encode(JSON.stringify({ t: "name", name: j.me })), { reliable: true, topic: "sotto" }).catch(() => {});
    };
    const refreshEncryption = () => {
      const remote = [...r.remoteParticipants.values()];
      setEncrypted(r.isE2EEEnabled && r.localParticipant.isEncrypted && remote.length > 0 && remote.every((p) => p.isEncrypted));
    };

    r.on(RoomEvent.ParticipantConnected, () => {
      peerArrived();
      refreshPeople();
      refreshEncryption();
      sayName();
      setRingingNames((n) => n.slice(1));
    })
      .on(RoomEvent.ParticipantDisconnected, (p: Participant) => {
        if (!current()) return;
        watchers.current.get(p.identity)?.();
        watchers.current.delete(p.identity);
        refreshPeople();
        // The call ends when everyone else has left.
        if (r.remoteParticipants.size === 0) void finish("Call ended", true);
      })
      .on(RoomEvent.DataReceived, (payload: Uint8Array, p?: Participant, _kind?: unknown, topic?: string) => {
        if (!current() || !p || topic !== "sotto") return;
        try {
          const m = JSON.parse(new TextDecoder().decode(payload)) as { t?: string; name?: unknown };
          if (m.t === "name" && typeof m.name === "string") setNames((n) => ({ ...n, [p.identity]: (m.name as string).slice(0, 60) }));
        } catch {
          /* not ours */
        }
      })
      .on(RoomEvent.TrackSubscribed, (track: RemoteTrack, _pub: unknown, p: Participant) => {
        if (!current()) return;
        if (track.kind === Track.Kind.Audio && audio.current) {
          audio.current.appendChild(track.attach());
          watchers.current.get(p.identity)?.();
          watchers.current.set(
            p.identity,
            watchSpeaking(track.mediaStreamTrack, (on) =>
              setSpeakingIds((set) => {
                const next = new Set(set);
                if (on) next.add(p.identity);
                else next.delete(p.identity);
                return next;
              }),
            ),
          );
        }
        refreshEncryption();
      })
      .on(RoomEvent.TrackUnsubscribed, (track: RemoteTrack) => track.detach().forEach((el) => el.remove()))
      .on(RoomEvent.ParticipantEncryptionStatusChanged, refreshEncryption)
      .on(RoomEvent.EncryptionError, () => {
        const t = Date.now();
        failures.current = [...failures.current.filter((x) => t - x < 10_000), t];
        if (current() && failures.current.length > DECRYPT_FAILURES) void finish("The call couldn't be decrypted, so Sotto ended it to keep it private.", true);
      })
      .on(RoomEvent.ConnectionQualityChanged, (q: ConnectionQuality, p: Participant) => {
        if (p.identity === r.localParticipant.identity) setQuality(QUALITY[q] ?? null);
      })
      .on(RoomEvent.Reconnecting, () => setStage((s) => (s.kind === "ended" ? s : { kind: "reconnecting" })))
      .on(RoomEvent.Reconnected, () => setStage((s) => (s.kind === "ended" ? s : r.remoteParticipants.size ? { kind: "live" } : { kind: "waiting" })))
      .on(RoomEvent.Disconnected, () => current() && void finish("The call dropped. Check your connection and call again.", true));

    try {
      await keyProvider.setKey(bytesOf(j.key));
      await r.setE2EEEnabled(true);
      await r.connect(j.url, j.token, { autoSubscribe: true });
      if (!current()) {
        void r.disconnect();
        return;
      }
    } catch {
      if (!current()) return;
      room.current = null;
      void r.disconnect().catch(() => {});
      setStage({ kind: "error", text: "The call server can't be reached. Check your connection and try again.", retry: true });
      return;
    }
    try {
      // Development only: a test tone instead of the microphone, so calls can be checked without one.
      // ?tone=pulse switches it on and off every two seconds, like speech and silence.
      const tone = import.meta.env.DEV ? new URLSearchParams(location.search).get("tone") : null;
      if (tone !== null) {
        const ctx = new AudioContext();
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        const dest = ctx.createMediaStreamDestination();
        osc.frequency.value = 440;
        osc.connect(gain).connect(dest);
        osc.start();
        if (tone === "pulse") setInterval(() => (gain.gain.value = gain.gain.value > 0 ? 0 : 1), 2000);
        await r.localParticipant.publishTrack(dest.stream.getAudioTracks()[0], { source: Track.Source.Microphone, dtx: false, red: false });
      } else {
        await r.localParticipant.setMicrophoneEnabled(true, undefined, { dtx: false, red: false });
      }
    } catch (e) {
      if (!current()) return;
      const denied = e instanceof Error && (e.name === "NotAllowedError" || e.name === "SecurityError");
      setStage({
        kind: "error",
        text: denied
          ? "Sotto can't use your microphone. Allow microphone access for Sotto in your system settings, then try again."
          : "Your microphone couldn't be started. Check that one is connected, then try again.",
        retry: true,
      });
      room.current = null;
      await r.disconnect().catch(() => {});
      return;
    }
    if (!current()) {
      void r.disconnect();
      return;
    }
    if (r.remoteParticipants.size > 0) peerArrived();
    if (j.group) markedGroup.current = true;
    refreshPeople();
    refreshEncryption();
    sayName();
  }, [callId, finish]);

  useEffect(() => {
    void join();
    return () => {
      // Leaving on purpose (unmount): detach first so the disconnect isn't read as a dropped call.
      const r = room.current;
      room.current = null;
      stopAllWatchers();
      void r?.disconnect();
    };
  }, [join]);

  // The agent says when the call ends elsewhere (the other side hung up, it rang out, Sotto locked).
  useEffect(
    () =>
      subscribe((e) => {
        if (e.type === "call") {
          const c = e.call as { id: string; phase: string; reason?: string };
          if (c.id === callId && c.phase === "ended") void finish(ENDED[c.reason ?? "finished"] ?? "Call ended", false);
        }
        if (e.type === "state" && e.state === "locked") void finish("Sotto locked, so the call ended.", false);
      }),
    [callId, finish],
  );

  // In a browser preview the window can close without the app noticing; tell the agent.
  useEffect(() => {
    if (window.sotto) return;
    const bye = () => {
      const desk = new URLSearchParams(location.search).get("desk");
      const base = import.meta.env.DEV && (desk === "peer" || desk === "third") ? `/desk/${desk}` : "";
      if (!done.current) void fetch(`${base}/api/calls/${callId}/end`, { method: "POST", keepalive: true });
    };
    window.addEventListener("pagehide", bye);
    return () => window.removeEventListener("pagehide", bye);
  }, [callId]);

  useEffect(() => {
    if (stage.kind !== "live") return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [stage.kind]);

  const toggleMute = useCallback(async () => {
    const r = room.current;
    if (!r) return;
    const next = !muted;
    setMuted(next);
    // Mute here, without telling the server: the microphone track sends silence at the same size
    // and rate as speech, so muting looks like nothing at all from outside.
    const track = r.localParticipant.getTrackPublication(Track.Source.Microphone)?.track as LocalAudioTrack | undefined;
    if (track) track.mediaStreamTrack.enabled = !next;
  }, [muted]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "m" && !e.metaKey && !e.ctrlKey && !e.altKey) void toggleMute();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleMute]);

  const name = info?.name || "Contact";
  const group = people.length >= 2 || ringingNames.length > 0 || (info?.group ?? false);
  const nameOf = (id: string) => names[id] ?? (people.length === 1 && !info?.group ? name : "Joining…");
  const status =
    stage.kind === "loading"
      ? "Starting the call…"
      : stage.kind === "waiting"
        ? "Calling…"
        : stage.kind === "connecting"
          ? "Connecting…"
          : stage.kind === "reconnecting"
            ? "Reconnecting…"
            : stage.kind === "live"
              ? clock(now - (since ?? now))
              : stage.text;
  const inCall = stage.kind === "waiting" || stage.kind === "connecting" || stage.kind === "live" || stage.kind === "reconnecting";

  return (
    <div className="flex h-svh flex-col bg-background p-2">
      <div className="relative flex flex-1 flex-col items-center overflow-hidden rounded-2xl bg-pane px-6 pt-5 pb-7 shadow-pane">
        <div className="flex h-7 items-center">
          {encrypted && stage.kind === "live" ? (
            <span className="inline-flex h-7 items-center gap-1.5 rounded-full bg-[color-mix(in_srgb,var(--primary)_12%,transparent)] px-3 text-[12px] font-medium text-primary">
              <Lock size={12} aria-hidden="true" /> End-to-end encrypted
            </span>
          ) : inCall ? (
            <span className="text-[12px] text-muted-foreground">{stage.kind === "live" ? "Securing the call…" : "Encrypted with a key only you two have"}</span>
          ) : null}
        </div>

        <div className="flex flex-1 flex-col items-center justify-center">
          {group && stage.kind !== "ended" ? (
            <>
              <div className="grid w-full max-w-[320px] grid-cols-2 gap-x-3 gap-y-5">
                {people.map((id) => (
                  <Tile key={id} name={nameOf(id)} speaking={speakingIds.has(id)} />
                ))}
                {ringingNames.map((n, i) => (
                  <Tile key={`ringing-${i}`} name={n} ringing />
                ))}
              </div>
              <h1 className="mt-6 text-[17px] font-semibold tracking-tight">Group call · {people.length + 1} people</h1>
            </>
          ) : (
            <>
              <div className="relative grid size-[120px] place-items-center">
                <span
                  aria-hidden="true"
                  className={`call-ring absolute inset-0 rounded-full bg-[color-mix(in_srgb,var(--primary)_18%,transparent)] ${people[0] && speakingIds.has(people[0]) ? "is-speaking" : ""} ${stage.kind === "waiting" ? "is-calling" : ""}`}
                />
                <Avatar seed={`call:${name}`} size={104} />
              </div>
              <h1 className="mt-5 max-w-full truncate text-[21px] font-semibold tracking-tight">{name}</h1>
            </>
          )}
          <div className={`mt-1 text-[14px] ${stage.kind === "live" ? "text-foreground tabular" : "text-center text-muted-foreground"}`} aria-live="polite">
            {stage.kind === "loading" ? <MarkIndicator size={18} label="Starting the call…" /> : status}
          </div>
          {stage.kind === "live" && quality && <p className="mt-1 text-[12px] text-muted-foreground">{quality}</p>}
          {stage.kind === "error" && (
            <div className="mt-4 flex gap-2">
              {stage.retry && (
                <button type="button" onClick={() => void join()} className="press h-9 cursor-pointer rounded-full bg-muted px-4 text-[13px] font-medium hover:bg-muted-strong">
                  Try again
                </button>
              )}
              <button type="button" onClick={() => void finish("Call ended", true)} className="press h-9 cursor-pointer rounded-full px-4 text-[13px] font-medium text-muted-foreground hover:bg-muted">
                Close
              </button>
            </div>
          )}
        </div>

        {inCall && (
          <div className="flex items-center gap-5">
            <RoundButton label={muted ? "Unmute (M)" : "Mute (M)"} pressed={muted} onClick={() => void toggleMute()}>
              {muted ? <MicOff size={22} /> : <Mic size={22} />}
            </RoundButton>
            <button
              type="button"
              aria-label={stage.kind === "waiting" ? "Cancel call" : "End call"}
              title={stage.kind === "waiting" ? "Cancel call" : "End call"}
              onClick={() => void finish(stage.kind === "waiting" ? "Call cancelled" : "Call ended", true)}
              className="press grid size-16 cursor-pointer place-items-center rounded-full bg-destructive text-destructive-foreground hover:opacity-90"
            >
              <PhoneOff size={24} />
            </button>
            <div className="relative">
              <RoundButton label="Add people" pressed={adding} onClick={() => setAdding((v) => !v)}>
                <UserPlus size={21} />
              </RoundButton>
              {adding && (
                <AddPeople
                  callId={callId}
                  onAdded={(n) => {
                    setRingingNames((r) => [...r, n]);
                    setTimeout(() => setRingingNames((r) => r.filter((x) => x !== n)), 45_000);
                  }}
                  onClose={() => setAdding(false)}
                />
              )}
            </div>
            <div className="relative">
              <RoundButton label="Microphone and speaker" pressed={devices} onClick={() => setDevices((v) => !v)}>
                <SlidersHorizontal size={21} />
              </RoundButton>
              {devices && room.current && <DeviceMenu room={room.current} onClose={() => setDevices(false)} />}
            </div>
          </div>
        )}
        <div ref={audio} hidden />
      </div>
    </div>
  );
}

function Tile({ name, speaking = false, ringing = false }: { name: string; speaking?: boolean; ringing?: boolean }) {
  return (
    <div className={`flex min-w-0 flex-col items-center ${ringing ? "opacity-60" : ""}`}>
      <div className="relative grid size-[76px] place-items-center">
        <span
          aria-hidden="true"
          className={`call-ring absolute inset-0 rounded-full bg-[color-mix(in_srgb,var(--primary)_18%,transparent)] ${speaking ? "is-speaking" : ""} ${ringing ? "is-calling" : ""}`}
        />
        <Avatar seed={`call:${name}`} size={64} />
      </div>
      <div className="mt-2 max-w-full truncate text-[13px] font-medium">{name}</div>
      <div className="text-[11.5px] text-muted-foreground">{ringing ? "Ringing…" : speaking ? "Speaking" : " "}</div>
    </div>
  );
}

/** Pick a conversation to add to this call. They get an ordinary call invitation. */
function AddPeople({ callId, onAdded, onClose }: { callId: string; onAdded: (name: string) => void; onClose: () => void }) {
  const [list, setList] = useState<Person[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    get<{ people: Person[] }>(`/api/calls/${callId}/people`)
      .then((r) => setList(r.people))
      .catch((e: ApiError) => setError(e.message));
    const close = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [callId, onClose]);
  const add = async (p: Person) => {
    setBusy(p.id);
    setError(null);
    try {
      await post(`/api/calls/${callId}/invite`, { contact_id: p.id });
      onAdded(p.name);
      setList((l) => l?.filter((x) => x.id !== p.id) ?? null);
    } catch (e) {
      setError((e as ApiError).message);
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="absolute bottom-full left-1/2 mb-2 w-64 -translate-x-1/2 rounded-xl bg-pane p-1 shadow-float" style={{ zIndex: "var(--z-dropdown)" }} role="menu" aria-label="Add people">
      <div className="px-2.5 pt-1.5 pb-1 text-[11.5px] font-medium text-muted-foreground">Add to this call</div>
      {list === null && !error && <p className="px-2.5 py-2 text-[12.5px] text-muted-foreground">Loading…</p>}
      {list?.length === 0 && <p className="px-2.5 py-2 text-[12.5px] text-muted-foreground">Everyone you talk to is already here.</p>}
      {list?.map((p) => (
        <button
          key={p.id}
          type="button"
          disabled={busy !== null}
          onClick={() => void add(p)}
          className="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] hover:bg-muted disabled:opacity-60"
        >
          <Avatar seed={`call:${p.name}`} size={24} />
          <span className="flex-1 truncate">{p.name}</span>
          {busy === p.id && <span className="text-[11.5px] text-muted-foreground">Calling…</span>}
        </button>
      ))}
      {error && <p className="px-2.5 py-2 text-[12.5px] text-destructive">{error}</p>}
    </div>
  );
}

function RoundButton({ label, pressed, onClick, children }: { label: string; pressed: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      title={label}
      onClick={onClick}
      className={`press grid size-14 cursor-pointer place-items-center rounded-full ${pressed ? "bg-foreground text-pane" : "bg-muted text-foreground hover:bg-muted-strong"}`}
    >
      {children}
    </button>
  );
}

function DeviceMenu({ room, onClose }: { room: Room; onClose: () => void }) {
  const [inputs, setInputs] = useState<MediaDeviceInfo[]>([]);
  const [outputs, setOutputs] = useState<MediaDeviceInfo[]>([]);
  const [active, setActive] = useState<{ in?: string; out?: string }>({});
  useEffect(() => {
    void Room.getLocalDevices("audioinput").then(setInputs);
    void Room.getLocalDevices("audiooutput").then(setOutputs);
    setActive({ in: room.getActiveDevice("audioinput"), out: room.getActiveDevice("audiooutput") });
    const close = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [room, onClose]);
  const pick = async (kind: "audioinput" | "audiooutput", id: string) => {
    await room.switchActiveDevice(kind, id).catch(() => {});
    setActive((a) => (kind === "audioinput" ? { ...a, in: id } : { ...a, out: id }));
  };
  const group = (title: string, list: MediaDeviceInfo[], kind: "audioinput" | "audiooutput", current?: string) =>
    list.length > 0 && (
      <div className="py-1">
        <div className="px-2.5 pt-1 pb-1 text-[11.5px] font-medium text-muted-foreground">{title}</div>
        {list.map((d, i) => (
          <button
            key={d.deviceId || i}
            type="button"
            onClick={() => void pick(kind, d.deviceId)}
            className="flex h-8 w-full cursor-pointer items-center gap-2 rounded-lg px-2.5 text-left text-[13px] hover:bg-muted"
          >
            <span className="w-3.5">{(current ?? "default") === d.deviceId && <Check size={14} className="text-primary" />}</span>
            <span className="truncate">{d.label || `${title} ${i + 1}`}</span>
          </button>
        ))}
      </div>
    );
  return (
    <div className="absolute right-0 bottom-full mb-2 w-64 rounded-xl bg-pane p-1 shadow-float" style={{ zIndex: "var(--z-dropdown)" }} role="menu">
      {group("Microphone", inputs, "audioinput", active.in)}
      {group("Speaker", outputs, "audiooutput", active.out)}
      {inputs.length + outputs.length === 0 && <p className="px-2.5 py-2 text-[12.5px] text-muted-foreground">No audio devices found.</p>}
    </div>
  );
}
