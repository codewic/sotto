import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Phone, PhoneOff } from "lucide-react";
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { type ApiError, get, patch, post } from "../api";
import type { CallSettings, CallView, Contact } from "../types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { useToast } from "../ui/toast";
import { Avatar, seedOf } from "../ui/Avatar";
import { contactName } from "./ConversationDialogs";

const nameOf = (c?: Contact) => (c ? contactName(c) : "Contact");

/** The call window: the app's own window, or a popup in the browser preview. */
export function openCallWindow(callId: string) {
  if (window.sotto?.openCall) {
    void window.sotto.openCall(callId);
    return;
  }
  // Development switches (?tone, ?tone=pulse, ?nopriv) carry over to the call window.
  const dev = new URLSearchParams();
  if (import.meta.env.DEV) {
    const here = new URLSearchParams(location.search);
    for (const k of ["tone", "nopriv"]) if (here.has(k)) dev.set(k, here.get(k) ?? "");
  }
  const params = dev.size ? `?${dev.toString()}` : "";
  window.open(`./call.html${params}#${callId}`, `sotto-call-${callId}`, "popup,width=400,height=620");
}

type CallCtx = { call: (c: Contact, more?: Contact[]) => Promise<void>; calls: CallView[] };
const Ctx = createContext<CallCtx>({ call: async () => {}, calls: [] });
export const useCalls = () => useContext(Ctx);

/** Calls for the main window: starting one, the incoming sheet, and the in-call bar. */
export function CallLayer({ contacts, children, onSettings }: { contacts: Contact[]; children: ReactNode; onSettings: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const calls = useQuery({ queryKey: ["calls"], queryFn: () => get<CallView[]>("/api/calls") });
  const [notice, setNotice] = useState<Contact | null>(null);
  const [setup, setSetup] = useState(false);
  const byId = useMemo(() => new Map(contacts.map((c) => [c.id, c])), [contacts]);
  const list = useMemo(() => calls.data ?? [], [calls.data]);

  const begin = useCallback(
    async (c: Contact, more: Contact[] = []) => {
      try {
        const v = await post<CallView>(`/api/contacts/${c.id}/call`);
        await qc.invalidateQueries({ queryKey: ["calls"] });
        openCallWindow(v.id);
        // A group call: everyone else gets an invitation to the same call.
        for (const m of more) await post(`/api/calls/${v.id}/invite`, { contact_id: m.id }).catch(() => {});
      } catch (e) {
        toast("error", (e as ApiError).message);
      }
    },
    [qc, toast],
  );

  const [pendingMore, setPendingMore] = useState<Contact[]>([]);
  const call = useCallback(
    async (c: Contact, more: Contact[] = []) => {
      if (list.some((x) => x.phase !== "ended")) {
        toast("info", "You're already on a call. End it first.");
        return;
      }
      let s: CallSettings;
      try {
        s = await get<CallSettings>("/api/calls/settings");
      } catch (e) {
        toast("error", (e as ApiError).message);
        return;
      }
      if (!s.server) return setSetup(true);
      if (!s.allowed) {
        toast("info", "Calls are off in Extreme mode, because the call server would see your network address. Send a message instead.");
        return;
      }
      if (s.needs_notice) {
        setPendingMore(more);
        return setNotice(c);
      }
      await begin(c, more);
    },
    [list, begin, toast],
  );

  const ringing = list.find((c) => c.phase === "ringing");
  const current = list.find((c) => c.phase === "calling" || c.phase === "active");

  return (
    <Ctx.Provider value={{ call, calls: list }}>
      {current && <InCallBar call={current} name={nameOf(byId.get(current.contact_id))} />}
      {children}
      {ringing && <IncomingCall key={ringing.id} call={ringing} contact={byId.get(ringing.contact_id)} />}
      <Dialog open={setup} onClose={() => setSetup(false)} title="Set up calls first" description="Calls go through a call server your organization runs. Add its address in Settings, then call again.">
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setSetup(false)}>
            Not now
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              setSetup(false);
              onSettings();
            }}
          >
            Open call settings
          </Button>
        </div>
      </Dialog>
      <HighRiskNotice
        contact={notice}
        onClose={() => setNotice(null)}
        onAgree={async () => {
          const c = notice;
          setNotice(null);
          try {
            await patch("/api/calls/settings", { high_risk_ack: true });
            if (c) await begin(c, pendingMore);
          } catch (e) {
            toast("error", (e as ApiError).message);
          }
        }}
      />
    </Ctx.Provider>
  );
}

function HighRiskNotice({ contact, onClose, onAgree }: { contact: Contact | null; onClose: () => void; onAgree: () => void }) {
  return (
    <Dialog open={!!contact} onClose={onClose} title="Before your first call in High risk mode">
      <div className="flex flex-col gap-4 text-[13.5px] leading-relaxed text-muted-foreground">
        <p>Messages travel over Tor. Calls can't: live voice over Tor is too slow to talk. So a call goes through your organization's call server.</p>
        <ul className="flex flex-col gap-1.5">
          <li className="flex gap-2">
            <span aria-hidden="true" className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
            The call server can't hear the call: it's encrypted with a key only you two have.
          </li>
          <li className="flex gap-2">
            <span aria-hidden="true" className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
            It does see your network address and when the call happens.
          </li>
          <li className="flex gap-2">
            <span aria-hidden="true" className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
            The other person never sees your network address.
          </li>
        </ul>
        <p>If that's too much for this conversation, send a message instead. In Extreme mode calls are off.</p>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" icon={<Phone size={15} />} onClick={onAgree}>
            Call anyway
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function since(ms: number | null) {
  if (!ms) return "";
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function InCallBar({ call, name }: { call: CallView; name: string }) {
  const toast = useToast();
  const [, tick] = useState(0);
  useEffect(() => {
    if (call.phase !== "active") return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [call.phase]);
  return (
    <div
      className="fixed top-3 left-1/2 flex h-10 -translate-x-1/2 items-center gap-3 rounded-full bg-pane pr-1.5 pl-4 text-[13px] whitespace-nowrap shadow-float"
      style={{ zIndex: "var(--z-sticky)" }}
      role="status"
    >
      <span aria-hidden="true" className="live-dot size-2 rounded-full bg-primary" />
      <span className="max-w-[40vw] truncate font-medium">
        {call.phase === "calling" ? `Calling ${name}…` : call.group ? `On a group call with ${name} and others` : `On a call with ${name}`}
      </span>
      {call.phase === "active" && <span className="text-muted-foreground tabular">{since(call.answered_at)}</span>}
      <button type="button" onClick={() => openCallWindow(call.id)} className="press h-7 cursor-pointer rounded-full px-3 font-medium text-muted-foreground hover:bg-muted hover:text-foreground">
        Show call
      </button>
      <button
        type="button"
        aria-label="End call"
        title="End call"
        onClick={() => post(`/api/calls/${call.id}/end`).catch((e: ApiError) => toast("error", e.message))}
        className="press grid size-7 cursor-pointer place-items-center rounded-full bg-destructive text-destructive-foreground hover:opacity-90"
      >
        <PhoneOff size={14} />
      </button>
    </div>
  );
}

/** A soft two-note ring, repeated every three seconds while a call rings. */
function useRingtone(on: boolean) {
  useEffect(() => {
    if (!on) return;
    let ctx: AudioContext;
    try {
      ctx = new AudioContext();
    } catch {
      return;
    }
    const ring = () => {
      const t = ctx.currentTime;
      [660, 880].forEach((f, i) => {
        const o = ctx.createOscillator();
        const g = ctx.createGain();
        const at = t + i * 0.32;
        o.frequency.value = f;
        o.type = "sine";
        g.gain.setValueAtTime(0, at);
        g.gain.linearRampToValueAtTime(0.12, at + 0.03);
        g.gain.linearRampToValueAtTime(0, at + 0.3);
        o.connect(g).connect(ctx.destination);
        o.start(at);
        o.stop(at + 0.32);
      });
    };
    ring();
    const timer = setInterval(ring, 3000);
    return () => {
      clearInterval(timer);
      void ctx.close();
    };
  }, [on]);
}

function IncomingCall({ call, contact }: { call: CallView; contact?: Contact }) {
  const toast = useToast();
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  useRingtone(true);
  const name = nameOf(contact);
  const act = async (path: string, body?: unknown) => {
    setBusy(true);
    try {
      await post(path, body);
      await qc.invalidateQueries({ queryKey: ["calls"] });
      return true;
    } catch (e) {
      toast("error", (e as ApiError).message);
      return false;
    } finally {
      setBusy(false);
    }
  };
  return (
    <div
      className="fixed top-4 left-1/2 w-[360px] max-w-[calc(100vw-32px)] -translate-x-1/2 rounded-2xl bg-pane p-4 shadow-float"
      style={{ zIndex: "var(--z-modal)", animation: "dialog-in 220ms var(--ease-smooth)" }}
      role="alertdialog"
      aria-label={call.group ? `${name} is adding you to a group call` : `Incoming call from ${name}`}
    >
      <div className="flex items-center gap-3">
        <span className="relative grid size-11 shrink-0 place-items-center">
          <span aria-hidden="true" className="call-ring is-calling absolute inset-0 rounded-full bg-[color-mix(in_srgb,var(--primary)_18%,transparent)]" />
          <Avatar seed={contact ? seedOf(contact) : name} size={40} />
        </span>
        <div className="min-w-0">
          <div className="truncate text-[15px] font-semibold tracking-tight">{name}</div>
          <div className="text-[12.5px] text-muted-foreground">{call.group ? "Adding you to a group call" : "Incoming call"} · end-to-end encrypted</div>
        </div>
      </div>
      <div className="mt-4 flex items-center gap-2">
        <Button className="flex-1" disabled={busy} onClick={() => act(`/api/calls/${call.id}/decline`, { say: false })}>
          Decline
        </Button>
        <Button
          autoFocus
          className="flex-1"
          variant="primary"
          icon={<Phone size={15} />}
          loading={busy}
          onClick={async () => {
            if (await act(`/api/calls/${call.id}/answer`)) openCallWindow(call.id);
          }}
        >
          Answer
        </Button>
      </div>
      <button
        type="button"
        disabled={busy}
        onClick={() => act(`/api/calls/${call.id}/decline`, { say: true })}
        className="mt-2 w-full cursor-pointer rounded-lg py-1 text-center text-[12.5px] text-muted-foreground hover:text-foreground"
      >
        Decline and say you can't talk now
      </button>
    </div>
  );
}
