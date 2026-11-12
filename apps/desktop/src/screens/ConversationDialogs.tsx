import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AtSign } from "lucide-react";
import { useState } from "react";
import { del, get, patch, post, put } from "../api";
import { SpotDone } from "../brand/spots";
import type { Contact } from "../types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field, TextArea } from "../ui/Field";
import { CopyButton, Notice, Switch } from "../ui/bits";
import { useToast } from "../ui/toast";

export const contactName = (c: Contact) => c.nickname || c.their_name || "Unnamed contact";
const name = contactName;

export type TagView = { tag: string | null; address?: string; relay?: string; ready?: number };

/** Lowercase letters, digits, dots and underscores, 3 to 24, starting with a letter (as the relay checks). */
export const tagOk = (t: string) => /^[a-z][a-z0-9._]{2,23}$/.test(t.trim().replace(/^@/, "").toLowerCase());

/** A tag someone typed: `@ada`, or a full tag on another relay, `ada@relay.example.org:7400`. */
export const addressOk = (t: string) => /^@?[a-z][a-z0-9._]{2,23}(@[a-z0-9.-]+(:\d{1,5})?)?$/.test(t.trim().toLowerCase());

/** Your tag: show and copy it, or pick one. Used in the invite dialog and in Settings. */
export function TagPanel({ compact = false }: { compact?: boolean }) {
  const qc = useQueryClient();
  const tag = useQuery({ queryKey: ["tag"], queryFn: () => get<TagView>("/api/tag") });
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const claim = async () => {
    setBusy(true);
    setError(null);
    try {
      await put("/api/tag", { tag: value });
      setValue("");
      await qc.invalidateQueries({ queryKey: ["tag"] });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  if (tag.data?.tag) {
    return (
      <div className="flex items-center gap-3 rounded-xl bg-muted px-4 py-3">
        <AtSign size={18} className="shrink-0 text-primary" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-[15px] font-semibold">@{tag.data.tag}</div>
          {tag.data.address && <div className="truncate font-mono text-[12px] text-muted-foreground" title="Your full tag">{tag.data.address}</div>}
          {!compact && (
            <div className="mt-1 text-[12.5px] text-muted-foreground">Anyone can reach you with your full tag; on your relay, @{tag.data.tag} is enough. You accept each person first.</div>
          )}
        </div>
        <CopyButton text={tag.data.address ?? `@${tag.data.tag}`} label="Copy" />
      </div>
    );
  }
  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        void claim();
      }}
    >
      <div className="flex items-end gap-2">
        <div className="min-w-0 flex-1">
          <Field
            label="Pick a tag"
            value={value}
            onChange={(e) => setValue(e.target.value.toLowerCase().replace(/\s/g, ""))}
            placeholder="ada.mensah"
            spellCheck={false}
            autoComplete="off"
            maxLength={25}
            error={error}
            hint={error ? undefined : "Like a username, so people can reach you without a link, on your relay or any other. 3 to 24 letters, numbers, dots or underscores."}
          />
        </div>
        <Button type="submit" variant="primary" loading={busy} disabled={!tagOk(value)}>
          Claim
        </Button>
      </div>
    </form>
  );
}

export function InviteDialog({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (id: string) => void }) {
  const [nickname, setNickname] = useState("");
  const [label, setLabel] = useState("");
  const [listed, setListed] = useState(false);
  const [linkMode, setLinkMode] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ link: string; qr: string | null; contact_id: string } | null>(null);
  const close = () => {
    setResult(null);
    setNickname("");
    setLabel("");
    setError(null);
    setLinkMode(false);
    onClose();
  };
  return (
    <Dialog
      open={open}
      onClose={close}
      title={result ? "Share this invite" : "Invite someone"}
      description={result ? "It works once and expires in 7 days." : linkMode ? "A one-time link, for someone who shouldn't know your tag." : "The easy way: give them your tag."}
      width={result ? 520 : 480}
    >
      {result ? (
        <div className="flex flex-col items-center gap-5">
          {result.qr && <div className="rounded-xl bg-white p-3" dangerouslySetInnerHTML={{ __html: result.qr }} aria-label="Invite QR code" />}
          <div className="w-full rounded-xl bg-muted p-3 font-mono text-[11.5px] leading-relaxed break-all text-muted-foreground select-all">{result.link}</div>
          <div className="flex w-full items-center justify-between gap-3">
            <span className="text-[12.5px] text-muted-foreground">Show the code in person, or send the link through a channel you trust.</span>
            <CopyButton text={result.link} label="Copy link" />
          </div>
          <Button variant="primary" className="self-end" onClick={close}>
            Done
          </Button>
        </div>
      ) : !linkMode ? (
        <div className="flex flex-col gap-5">
          <TagPanel />
          <div className="flex items-center justify-between gap-3 border-t border-line pt-4">
            <span className="text-[13px] text-muted-foreground">Shouldn't know your tag?</span>
            <Button variant="secondary" onClick={() => setLinkMode(true)}>
              Make a one-time link
            </Button>
          </div>
        </div>
      ) : (
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError(null);
            try {
              const r = await post<{ link: string; qr: string | null; contact_id: string }>("/api/invites", { nickname: nickname.trim(), label: label.trim(), listed });
              setResult(r);
              onCreated(r.contact_id);
            } catch (err) {
              setError((err as Error).message);
            } finally {
              setBusy(false);
            }
          }}
          className="flex flex-col gap-6"
        >
          <Field label="Your name for them" value={nickname} onChange={(e) => setNickname(e.target.value)} autoFocus hint="Only you see this, for example Priya (security) or Harbour contact." />
          <Field label="Name they'll see (optional)" value={label} onChange={(e) => setLabel(e.target.value)} hint="Leave empty if they shouldn't learn your name yet." />
          <label className="flex items-start justify-between gap-4">
            <span>
              <span className="block text-[14px] font-medium">Use my verified organization identity</span>
              <span className="block text-[13px] text-muted-foreground">Lets them check you against your organization's website. Leave off for anyone who must not be linked to your public identity.</span>
            </span>
            <Switch checked={listed} onChange={setListed} label="Use my verified organization identity" />
          </label>
          {error && <Notice tone="danger">{error}</Notice>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => setLinkMode(false)}>
              Back
            </Button>
            <Button variant="primary" type="submit" loading={busy}>
              Create invite
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

/** Add someone by their tag (`@ada`) or by an invite link they gave you: one box for both. */
export function AcceptDialog({ open, onClose, onAccepted }: { open: boolean; onClose: () => void; onAccepted: (id: string) => void }) {
  const qc = useQueryClient();
  const [input, setInput] = useState("");
  const [nickname, setNickname] = useState("");
  const [share, setShare] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const value = input.trim();
  const isTag = addressOk(value);
  const isLink = !isTag && (/^sotto:\/\//i.test(value) || value.length > 40);
  const close = () => {
    setInput("");
    setNickname("");
    setShare("");
    setError(null);
    setDone(false);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title={done ? "You're connected" : "Add someone"} description={done ? undefined : "Type their tag, like @ada, or their full tag if they're on another relay (ada@relay.example.org). Or paste an invite link."}>
      {done ? (
        <div className="flex flex-col items-center text-center">
          <SpotDone />
          <p className="mt-2 text-[13.5px] text-muted-foreground">Compare safety numbers before you share anything sensitive.</p>
          <Button variant="primary" className="mt-5" onClick={close}>
            Open conversation
          </Button>
        </div>
      ) : (
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError(null);
            try {
              const r = isLink
                ? await post<{ contact_id: string }>("/api/invites/accept", { link: value, nickname: nickname.trim(), share_name: share.trim() })
                : await post<{ contact_id: string }>("/api/tag/connect", { tag: value });
              await qc.invalidateQueries({ queryKey: ["contacts"] });
              onAccepted(r.contact_id);
              setDone(true);
            } catch (err) {
              setError((err as Error).message);
            } finally {
              setBusy(false);
            }
          }}
          className="flex flex-col gap-6"
        >
          <TextArea
            label="Tag or invite link"
            rows={isLink ? 3 : 1}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            autoFocus
            spellCheck={false}
            autoComplete="off"
            placeholder="@ada or sotto://invite/…"
            error={error}
          />
          {isLink && (
            <>
              <Field label="Your name for them (optional)" value={nickname} onChange={(e) => setNickname(e.target.value)} />
              <Field label="Name they'll see (optional)" value={share} onChange={(e) => setShare(e.target.value)} hint="Leave empty to stay anonymous to them." />
            </>
          )}
          {isTag && <p className="-mt-3 text-[12.5px] text-muted-foreground">They'll see your display name and can accept or remove you. Nobody else learns you connected.</p>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button variant="primary" type="submit" loading={busy} disabled={!isLink && !isTag}>
              Connect
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

export function SafetyDialog({ open, onClose, contact }: { open: boolean; onClose: () => void; contact: Contact }) {
  const qc = useQueryClient();
  const groups = (contact.safety_number ?? "").split(" ");
  async function setVerified(v: boolean) {
    await patch(`/api/contacts/${contact.id}`, { verified: v });
    await qc.invalidateQueries({ queryKey: ["contacts"] });
    onClose();
  }
  return (
    <Dialog open={open} onClose={onClose} title={`Safety number with ${name(contact)}`} description="Compare these numbers in person or on a call where you recognise their voice. If they match, nobody is in the middle.">
      <div className="grid grid-cols-4 gap-2 rounded-xl bg-muted p-4 font-mono text-[17px] tracking-wider tabular">
        {groups.map((g, i) => (
          <span key={i} className="text-center">
            {g}
          </span>
        ))}
      </div>
      <div className="mt-6 flex justify-end gap-2">
        {contact.verified ? (
          <Button variant="ghost" onClick={() => setVerified(false)}>
            Mark as not verified
          </Button>
        ) : (
          <>
            <Button variant="ghost" onClick={onClose}>
              Not now
            </Button>
            <Button variant="primary" onClick={() => setVerified(true)}>
              They match
            </Button>
          </>
        )}
      </div>
    </Dialog>
  );
}

const timers: [number, string][] = [
  [0, "Off"],
  [3600, "1 hour"],
  [86_400, "1 day"],
  [604_800, "7 days"],
  [2_592_000, "30 days"],
];

export function TimerDialog({ open, onClose, contact }: { open: boolean; onClose: () => void; contact: Contact }) {
  const qc = useQueryClient();
  const toast = useToast();
  return (
    <Dialog open={open} onClose={onClose} title="Disappearing messages" description="New messages are deleted from both devices after this time. It changes for both of you.">
      <div className="flex flex-col gap-1">
        {timers.map(([s, label]) => {
          const active = (contact.ttl_secs ?? 0) === s;
          return (
            <button
              key={s}
              type="button"
              onClick={async () => {
                try {
                  await patch(`/api/contacts/${contact.id}`, { timer: s });
                  await qc.invalidateQueries({ queryKey: ["contacts"] });
                  onClose();
                } catch (e) {
                  toast("error", (e as Error).message);
                }
              }}
              className={`press flex h-11 cursor-pointer items-center justify-between rounded-xl px-4 text-left text-[14px] ${active ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
            >
              {label}
              {active && <span className="size-2 rounded-full bg-primary" />}
            </button>
          );
        })}
      </div>
      {(contact.ttl_secs ?? 0) === 0 && <p className="mt-3 text-[12.5px] text-muted-foreground">Keeping messages forever means they can be found if this device is taken.</p>}
    </Dialog>
  );
}

export function RenameDialog({ open, onClose, contact }: { open: boolean; onClose: () => void; contact: Contact }) {
  const qc = useQueryClient();
  const [value, setValue] = useState(contact.nickname);
  return (
    <Dialog open={open} onClose={onClose} title="Rename" description="Only you see this name.">
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          await patch(`/api/contacts/${contact.id}`, { nickname: value.trim() });
          await qc.invalidateQueries({ queryKey: ["contacts"] });
          onClose();
        }}
      >
        <Field label="Name" value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit">
            Save name
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function DeleteDialog({ open, onClose, contact, onDeleted }: { open: boolean; onClose: () => void; contact: Contact; onDeleted: () => void }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  return (
    <Dialog open={open} onClose={onClose} title={`Delete ${name(contact)}?`} description="Every message and file in this conversation is erased from this device, and your mailbox on the relay is closed. They keep their own copy.">
      <div className="mt-2 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="danger"
          loading={busy}
          onClick={async () => {
            setBusy(true);
            try {
              await del(`/api/contacts/${contact.id}`);
              await qc.invalidateQueries({ queryKey: ["contacts"] });
              onDeleted();
            } finally {
              setBusy(false);
            }
          }}
        >
          Delete conversation
        </Button>
      </div>
    </Dialog>
  );
}
