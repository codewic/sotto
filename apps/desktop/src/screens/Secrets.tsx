import { useQueryClient } from "@tanstack/react-query";
import { Check, Copy, Eye, EyeOff, KeyRound } from "lucide-react";
import { useEffect, useState } from "react";
import { type ApiError, del, post } from "../api";
import type { Message, SecretView } from "../types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { Notice } from "../ui/bits";
import { useToast } from "../ui/toast";

const TTL: [number, string][] = [
  [3600, "1 hour"],
  [86_400, "1 day"],
  [7 * 86_400, "7 days"],
];

/** A secret as an item in the 1:1 timeline (so it sits in order with the messages). */
export function secretItem(contactId: string, s: SecretView): Message {
  return {
    id: `secret-${s.id}`,
    contact_id: contactId,
    outgoing: s.mine,
    kind: "secret",
    text: null,
    file: null,
    sent_at: s.sent_at,
    received_at: s.sent_at,
    expires_at: null,
    delivery: s.mine ? "sent" : "received",
    reply_to: null,
    reactions: [],
    edited: false,
    read: true,
    secret: s,
  };
}

function until(sec: number) {
  const left = sec - Date.now() / 1000;
  if (left <= 0) return "now";
  if (left < 3600) return `in ${Math.max(1, Math.round(left / 60))} min`;
  if (left < 86_400) return `in ${Math.round(left / 3600)} h`;
  return `in ${Math.round(left / 86_400)} days`;
}

const at = (sec: number) => new Date(sec * 1000).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

export function SecretCard({ contactId, s, name }: { contactId: string; s: SecretView; name: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [shown, setShown] = useState<string | null>(null);
  const status = s.mine
    ? s.state === "opened"
      ? `${name} opened it ${at(s.opened_at ?? 0)}. It's gone.`
      : s.state === "expired"
        ? "It expired before it was opened, and it's gone."
        : `Waiting for ${name} to open it. Expires ${until(s.expires_at)}.`
    : s.state === "opened"
      ? `You opened it ${at(s.opened_at ?? 0)}. It's gone from this device.`
      : s.state === "expired"
        ? "It expired before you opened it, and it's gone."
        : `Expires ${until(s.expires_at)}. It's deleted from this device as soon as you reveal it.`;
  const reveal = async () => {
    setBusy(true);
    try {
      const v = await post<SecretView & { text: string }>(`/api/contacts/${contactId}/secrets/${s.id}/open`);
      setShown(v.text);
      await qc.invalidateQueries({ queryKey: ["secrets", contactId] });
    } catch (e) {
      toast("error", (e as ApiError).message);
    } finally {
      setBusy(false);
    }
  };
  const live = s.state === "waiting";
  return (
    <div data-mid={`secret-${s.id}`} className="flex w-[min(340px,86%)] flex-col gap-2 rounded-2xl bg-[color-mix(in_srgb,var(--primary)_10%,var(--bubble-theirs))] px-3.5 py-3 shadow-[var(--bubble-theirs-shadow)]">
      <div className="flex items-center gap-2.5">
        <span className={`grid size-9 shrink-0 place-items-center rounded-xl ${live ? "bg-primary text-primary-foreground" : "bg-muted-strong text-muted-foreground"}`}>
          <KeyRound size={17} />
        </span>
        <div className="min-w-0">
          <div className="truncate text-[14px] font-semibold">{s.label}</div>
          <div className="text-[12px] text-muted-foreground">One-time secret{s.mine ? " you sent" : ""}</div>
        </div>
      </div>
      <p className="text-[12.5px] leading-relaxed text-muted-foreground">{status}</p>
      {!s.mine && live && (
        <Button size="sm" variant="primary" loading={busy} onClick={() => void reveal()}>
          Reveal once
        </Button>
      )}
      {(s.state !== "waiting" || s.mine) && (
        <button
          type="button"
          onClick={() => void del(`/api/contacts/${contactId}/secrets/${s.id}`).then(() => qc.invalidateQueries({ queryKey: ["secrets", contactId] }))}
          className="self-start text-[12px] text-muted-foreground hover:text-foreground hover:underline"
        >
          Remove from this conversation
        </button>
      )}
      <RevealDialog label={s.label} text={shown} onClose={() => setShown(null)} />
    </div>
  );
}

function RevealDialog({ label, text, onClose }: { label: string; text: string | null; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  const toast = useToast();
  // Clear what we put on the clipboard after a minute.
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => void navigator.clipboard.writeText("").catch(() => {}), 60_000);
    return () => clearTimeout(t);
  }, [copied]);
  return (
    <Dialog open={text !== null} onClose={onClose} title={label} description="This is the only time you'll see it. It's already deleted from this device; closing this removes it from the screen.">
      <div className="flex flex-col gap-4">
        <pre className="scroll-pane max-h-64 overflow-auto rounded-xl bg-muted px-3.5 py-3 font-mono text-[13px] leading-relaxed break-all whitespace-pre-wrap select-all">{text}</pre>
        <div className="flex items-center justify-between gap-3">
          <span className="text-[12px] text-muted-foreground">{copied ? "Copied. The clipboard is cleared in a minute." : "Copy it where it's needed, then close this."}</span>
          <div className="flex gap-2">
            <Button
              icon={copied ? <Check size={15} /> : <Copy size={15} />}
              onClick={() =>
                void navigator.clipboard
                  .writeText(text ?? "")
                  .then(() => setCopied(true))
                  .catch(() => toast("error", "Couldn't copy. Select the text and copy it instead."))
              }
            >
              {copied ? "Copied" : "Copy"}
            </Button>
            <Button variant="primary" onClick={onClose}>
              Done
            </Button>
          </div>
        </div>
      </div>
    </Dialog>
  );
}

export function SendSecretDialog({ open, contactId, name, onClose }: { open: boolean; contactId: string; name: string; onClose: () => void }) {
  const qc = useQueryClient();
  const [label, setLabel] = useState("");
  const [text, setText] = useState("");
  const [visible, setVisible] = useState(false);
  const [ttl, setTtl] = useState(86_400);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setLabel("");
    setText("");
    setVisible(false);
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title="Send a one-time secret" description={`For a password, an API key or a recovery code. ${name} can reveal it once; it's deleted the moment they do, or when it expires. Your copy isn't kept.`} width={480}>
      <form
        className="flex flex-col gap-5"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await post(`/api/contacts/${contactId}/secrets`, { label, text, ttl_secs: ttl });
            await qc.invalidateQueries({ queryKey: ["secrets", contactId] });
            close();
          } catch (err) {
            setError((err as ApiError).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label="What it is" value={label} onChange={(e) => setLabel(e.target.value)} autoFocus placeholder="For example, Staging database password" maxLength={80} hint="Not secret: they see this before revealing it." />
        <div>
          <label htmlFor="secret-text" className="text-[13px] font-medium text-foreground">
            Secret
          </label>
          <div className="relative mt-1.5">
            <textarea
              id="secret-text"
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={3}
              spellCheck={false}
              autoComplete="off"
              className={`w-full resize-none rounded-lg border border-line bg-transparent py-2 pr-10 pl-3 font-mono text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring ${visible ? "" : "[-webkit-text-security:disc]"}`}
            />
            <button
              type="button"
              onClick={() => setVisible((v) => !v)}
              aria-label={visible ? "Hide secret" : "Show secret"}
              aria-pressed={visible}
              className="press absolute top-1.5 right-1.5 inline-flex size-8 cursor-pointer items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              {visible ? <EyeOff size={16} /> : <Eye size={16} />}
            </button>
          </div>
        </div>
        <div>
          <div className="text-[13px] font-medium">If it isn't opened, destroy it after</div>
          <div className="mt-2 flex gap-1.5" role="radiogroup" aria-label="Expires after">
            {TTL.map(([v, l]) => (
              <button
                key={v}
                type="button"
                role="radio"
                aria-checked={ttl === v}
                onClick={() => setTtl(v)}
                className={`press h-8 cursor-pointer rounded-full px-3.5 text-[13px] font-medium ${ttl === v ? "bg-foreground text-pane" : "bg-muted text-muted-foreground hover:text-foreground"}`}
              >
                {l}
              </button>
            ))}
          </div>
        </div>
        {error && <Notice tone="danger">{error}</Notice>}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" icon={<KeyRound size={15} />} loading={busy} disabled={!label.trim() || !text.trim()}>
            Send secret
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
