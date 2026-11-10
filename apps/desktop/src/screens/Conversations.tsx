import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BadgeCheck, Clock, FileText, Paperclip, Pencil, QrCode, SendHorizontal, ShieldAlert, Trash2, UserPlus } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, del, get, patch, post } from "../api";
import { SpotConversations, SpotDone } from "../brand/spots";
import type { Contact, Message } from "../types";
import { Button, IconButton } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field, TextArea } from "../ui/Field";
import { Chip, CopyButton, Empty, Notice, Skeleton, Switch, bytes, timeLabel, ttlLabel } from "../ui/bits";
import { useToast } from "../ui/toast";
import { PaneHeader, useNav } from "./Shell";

const name = (c: Contact) => c.nickname || c.their_name || "Unnamed contact";

export function Conversations({ initial }: { initial?: string }) {
  const contacts = useQuery({ queryKey: ["contacts"], queryFn: () => get<Contact[]>("/api/contacts") });
  const [selected, setSelected] = useState<string | null>(null);
  const [dialog, setDialog] = useState<"invite" | "accept" | null>(null);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    if (initial === "invite" || initial === "accept") setDialog(initial);
    else if (initial) setSelected(initial);
  }, [initial]);

  const list = contacts.data ?? [];
  const shown = list.filter((c) => name(c).toLowerCase().includes(filter.trim().toLowerCase()));
  const current = list.find((c) => c.id === selected) ?? null;

  return (
    <div className="flex h-full min-h-0">
      <section className="flex w-[300px] shrink-0 flex-col border-r border-line">
        <PaneHeader
          title="Conversations"
          actions={
            <>
              <IconButton label="Use an invite" onClick={() => setDialog("accept")}>
                <QrCode size={16} />
              </IconButton>
              <IconButton label="Invite someone" onClick={() => setDialog("invite")}>
                <UserPlus size={16} />
              </IconButton>
            </>
          }
        />
        {list.length > 0 && (
          <div className="px-4 pt-3 pb-2">
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter by name"
              aria-label="Filter conversations"
              className="h-9 w-full rounded-lg bg-muted px-3 text-[13px] outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-primary/40"
            />
          </div>
        )}
        <div className="scroll-pane flex-1 px-2 pb-3">
          {contacts.isPending && (
            <div className="flex flex-col gap-2 p-2">
              {[0, 1, 2].map((i) => (
                <Skeleton key={i} className="h-14" />
              ))}
            </div>
          )}
          {contacts.isError && (
            <div className="p-3">
              <Notice tone="danger">
                Couldn't load conversations. <button className="cursor-pointer underline" onClick={() => contacts.refetch()}>Try again</button>
              </Notice>
            </div>
          )}
          {list.length > 0 && shown.length === 0 && (
            <div className="px-3 py-8 text-center text-[13px] text-muted-foreground">
              No conversations match "{filter}".{" "}
              <button className="cursor-pointer font-medium text-foreground underline underline-offset-4" onClick={() => setFilter("")}>
                Clear filter
              </button>
            </div>
          )}
          {shown.map((c) => (
            <ContactRow key={c.id} c={c} active={c.id === selected} onClick={() => setSelected(c.id)} />
          ))}
        </div>
      </section>
      <section className="flex min-w-0 flex-1 flex-col">
        {current ? (
          <Thread key={current.id} contact={current} onDeleted={() => setSelected(null)} />
        ) : list.length === 0 && !contacts.isPending ? (
          <div className="grid flex-1 place-items-center">
            <Empty
              art={<SpotConversations />}
              title="No conversations yet"
              action={
                <div className="flex gap-2">
                  <Button variant="primary" onClick={() => setDialog("invite")}>
                    Invite someone
                  </Button>
                  <Button onClick={() => setDialog("accept")}>Use an invite</Button>
                </div>
              }
            >
              Sotto has no directory to search. Meet in person and show a QR code, or send a one-time invite through a channel you already trust.
            </Empty>
          </div>
        ) : (
          <div className="grid flex-1 place-items-center text-[13.5px] text-muted-foreground">Choose a conversation on the left.</div>
        )}
      </section>
      <InviteDialog open={dialog === "invite"} onClose={() => setDialog(null)} onCreated={(id) => setSelected(id)} />
      <AcceptDialog open={dialog === "accept"} onClose={() => setDialog(null)} onAccepted={(id) => setSelected(id)} />
    </div>
  );
}

function ContactRow({ c, active, onClick }: { c: Contact; active: boolean; onClick: () => void }) {
  const sub =
    c.state === "invited"
      ? `Invite waiting${c.invite_expires ? `, expires ${new Date(c.invite_expires * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" })}` : ""}`
      : c.their_name && c.nickname && c.their_name !== c.nickname
        ? `Calls themselves ${c.their_name}`
        : `Active ${timeLabel(c.last_activity * 1000)}`;
  return (
    <button
      type="button"
      onClick={onClick}
      className={`press flex w-full cursor-pointer items-center gap-3 rounded-xl px-3 py-2.5 text-left ${active ? "bg-muted" : "hover:bg-muted/60"}`}
    >
      <span className="grid size-9 shrink-0 place-items-center rounded-full bg-muted-strong text-[13px] font-semibold text-muted-foreground">{name(c).slice(0, 1).toUpperCase()}</span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-[14px] font-medium">{name(c)}</span>
          {c.verified && <BadgeCheck size={14} className="shrink-0 text-primary" aria-label="Verified" />}
        </span>
        <span className="block truncate text-[12.5px] text-muted-foreground">{sub}</span>
      </span>
      {c.unread > 0 && <span className="min-w-5 rounded-full bg-primary px-1.5 text-center text-[11px] font-semibold text-primary-foreground tabular">{c.unread}</span>}
    </button>
  );
}

function Thread({ contact, onDeleted }: { contact: Contact; onDeleted: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const { go } = useNav();
  const messages = useQuery({ queryKey: ["messages", contact.id], queryFn: () => get<Message[]>(`/api/contacts/${contact.id}/messages`) });
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [dialog, setDialog] = useState<"safety" | "timer" | "rename" | "delete" | null>(null);
  const [reviewing, setReviewing] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const ready = contact.state === "ready";

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [messages.data?.length]);

  useEffect(() => {
    if (contact.unread > 0) void post(`/api/contacts/${contact.id}/read`).then(() => qc.invalidateQueries({ queryKey: ["contacts"] }));
  }, [contact.id, contact.unread, qc]);

  async function send() {
    const body = text.trim();
    if (!body || sending) return;
    setSending(true);
    try {
      await post(`/api/contacts/${contact.id}/messages`, { text: body });
      setText("");
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setSending(false);
      qc.invalidateQueries({ queryKey: ["messages", contact.id] });
    }
  }

  async function attach(file: File) {
    const data = new Uint8Array(await file.arrayBuffer());
    toast("info", `Encrypting and sending ${file.name}…`);
    try {
      await api("POST", `/api/contacts/${contact.id}/files`, data, { "content-type": file.type || "application/octet-stream", "x-file-name": encodeURIComponent(file.name) });
      toast("success", `${file.name} sent.`);
    } catch (e) {
      toast("error", (e as Error).message);
    }
  }

  async function review(m: Message) {
    setReviewing(m.id);
    try {
      const report = await post<{ key: string }>(`/api/contacts/${contact.id}/messages/${m.id}/review`);
      go("review", report.key);
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setReviewing(null);
    }
  }

  const grouped = useMemo(() => {
    const out: { day: string; items: Message[] }[] = [];
    for (const m of messages.data ?? []) {
      const day = new Date(m.received_at).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
      if (!out.length || out[out.length - 1].day !== day) out.push({ day, items: [] });
      out[out.length - 1].items.push(m);
    }
    return out;
  }, [messages.data]);

  return (
    <>
      <PaneHeader
        title={
          <span className="flex items-center gap-2">
            {name(contact)}
            {contact.verified ? (
              <Chip tone="accent">
                <BadgeCheck size={13} /> Verified
              </Chip>
            ) : ready ? (
              <Chip>Not verified</Chip>
            ) : (
              <Chip>Invite waiting</Chip>
            )}
          </span>
        }
        subtitle={ready ? `Messages disappear: ${ttlLabel(contact.ttl_secs).toLowerCase()}` : "They haven't used your invite yet. You can write once they do."}
        actions={
          <>
            {ready && (
              <Button size="sm" variant={contact.verified ? "ghost" : "secondary"} icon={<ShieldAlert size={14} />} onClick={() => setDialog("safety")}>
                {contact.verified ? "Safety number" : "Verify"}
              </Button>
            )}
            <IconButton label="Disappearing messages" onClick={() => setDialog("timer")} disabled={!ready}>
              <Clock size={16} />
            </IconButton>
            <IconButton label="Rename" onClick={() => setDialog("rename")}>
              <Pencil size={16} />
            </IconButton>
            <IconButton label="Delete conversation" onClick={() => setDialog("delete")}>
              <Trash2 size={16} />
            </IconButton>
          </>
        }
      />
      <div className="scroll-pane flex-1 px-6 py-5">
        {messages.isPending && (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-10 w-2/5" />
            <Skeleton className="ml-auto h-10 w-1/3" />
            <Skeleton className="h-16 w-1/2" />
          </div>
        )}
        {!messages.isPending && (messages.data?.length ?? 0) === 0 && (
          <div className="grid h-full place-items-center">
            <div className="max-w-sm text-center text-[13.5px] text-muted-foreground">
              {ready
                ? "No messages yet. Before you share anything sensitive, compare safety numbers in person or over a call you trust."
                : "Your invite is waiting. Once they use it, you'll see them here and can start writing."}
            </div>
          </div>
        )}
        {grouped.map((g) => (
          <div key={g.day}>
            <div className="my-4 text-center text-[12px] font-medium text-muted-foreground">{g.day}</div>
            <div className="flex flex-col gap-1.5">
              {g.items.map((m) => (
                <Bubble key={m.id} m={m} contactId={contact.id} onReview={() => review(m)} reviewing={reviewing === m.id} onOpenReview={() => go("review", `m:file/${m.id}`)} />
              ))}
            </div>
          </div>
        ))}
        <div ref={bottom} />
      </div>
      <div className="shrink-0 px-6 pt-2 pb-5">
        {ready ? (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void send();
            }}
            className="flex items-end gap-2 rounded-2xl bg-muted p-2"
          >
            <IconButton label="Attach a file" type="button" onClick={() => fileInput.current?.click()}>
              <Paperclip size={16} />
            </IconButton>
            <input
              ref={fileInput}
              type="file"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void attach(f);
                e.target.value = "";
              }}
            />
            <textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                }
              }}
              rows={Math.min(6, Math.max(1, text.split("\n").length))}
              placeholder={`Write to ${name(contact)}`}
              aria-label="Message"
              className="max-h-40 min-h-8 flex-1 resize-none bg-transparent px-1 py-1.5 text-[14px] leading-relaxed outline-none placeholder:text-muted-foreground"
            />
            <Button variant="primary" size="sm" type="submit" disabled={!text.trim()} loading={sending} icon={<SendHorizontal size={14} />}>
              Send
            </Button>
          </form>
        ) : (
          <Notice>Waiting for {name(contact)} to accept your invite. Send them the link or show the QR code again from the invite you created.</Notice>
        )}
      </div>
      <SafetyDialog open={dialog === "safety"} onClose={() => setDialog(null)} contact={contact} />
      <TimerDialog open={dialog === "timer"} onClose={() => setDialog(null)} contact={contact} />
      <RenameDialog open={dialog === "rename"} onClose={() => setDialog(null)} contact={contact} />
      <DeleteDialog
        open={dialog === "delete"}
        onClose={() => setDialog(null)}
        contact={contact}
        onDeleted={() => {
          setDialog(null);
          onDeleted();
        }}
      />
    </>
  );
}

function Bubble({ m, contactId, onReview, reviewing, onOpenReview }: { m: Message; contactId: string; onReview: () => void; reviewing: boolean; onOpenReview: () => void }) {
  const qc = useQueryClient();
  if (m.kind === "system") {
    return <div className="my-1 text-center text-[12px] text-muted-foreground">{m.text}</div>;
  }
  const mine = m.outgoing;
  return (
    <div className={`flex flex-col ${mine ? "items-end" : "items-start"}`}>
      <div className={`max-w-[68%] rounded-2xl px-3.5 py-2 text-[14px] leading-relaxed ${mine ? "rounded-br-md bg-foreground text-pane" : "rounded-bl-md bg-muted"}`}>
        {m.kind === "text" && <p className="break-words whitespace-pre-wrap">{m.text}</p>}
        {m.kind === "file" && m.file && (
          <div className="flex min-w-[240px] flex-col gap-2 py-1">
            <div className="flex items-center gap-2.5">
              <span className={`grid size-9 place-items-center rounded-lg ${mine ? "bg-pane/15" : "bg-pane"}`}>
                <FileText size={18} />
              </span>
              <div className="min-w-0">
                <div className="truncate font-medium">{m.file.name}</div>
                <div className={`text-[12px] ${mine ? "text-pane/70" : "text-muted-foreground"}`}>
                  {bytes(m.file.size)}
                  {m.file.status === "downloading" && " · Receiving…"}
                  {m.file.status === "failed" && " · Didn't arrive intact"}
                </div>
              </div>
            </div>
            {m.file.caption && <p className="whitespace-pre-wrap">{m.file.caption}</p>}
            {!mine && m.file.status === "ready" && m.file.review.state === "quarantined" && (
              <div className="flex items-center justify-between gap-3 rounded-xl bg-pane px-3 py-2">
                <span className="text-[12.5px] text-muted-foreground">Quarantined until reviewed</span>
                <Button size="sm" variant="primary" onClick={onReview} loading={reviewing}>
                  {reviewing ? "Opening safely…" : "Review safely"}
                </Button>
              </div>
            )}
            {!mine && m.file.review.state === "clean" && (
              <Button size="sm" onClick={onOpenReview}>
                Open clean copy · {m.file.review.pages} page{m.file.review.pages === 1 ? "" : "s"}
              </Button>
            )}
            {!mine && m.file.review.state === "failed" && <span className="text-[12.5px] text-destructive">{m.file.review.reason}</span>}
          </div>
        )}
      </div>
      <div className="mt-0.5 flex items-center gap-1.5 px-1 text-[11.5px] text-muted-foreground tabular">
        {timeLabel(m.received_at)}
        {mine && m.delivery === "delivered" && " · Delivered"}
        {mine && m.delivery === "sent" && " · Sent"}
        {mine && m.delivery === "sending" && " · Sending…"}
        {mine && m.delivery === "failed" && (
          <button
            className="cursor-pointer text-destructive underline-offset-2 hover:underline"
            onClick={() => post(`/api/contacts/${contactId}/messages/${m.id}/retry`).finally(() => qc.invalidateQueries({ queryKey: ["messages", contactId] }))}
          >
            · Not sent. Retry
          </button>
        )}
        {m.expires_at && <Clock size={11} aria-label="Disappears" />}
      </div>
    </div>
  );
}

function InviteDialog({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (id: string) => void }) {
  const [nickname, setNickname] = useState("");
  const [label, setLabel] = useState("");
  const [listed, setListed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ link: string; qr: string | null; contact_id: string } | null>(null);
  const close = () => {
    setResult(null);
    setNickname("");
    setLabel("");
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title={result ? "Share this invite" : "Invite someone"} description={result ? "It works once and expires in 7 days." : "Each invite works once. Nothing about it is stored on any server."} width={result ? 520 : 480}>
      {!result ? (
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
          <Field label="Your name for them" value={nickname} onChange={(e) => setNickname(e.target.value)} autoFocus hint="Only you see this, for example Source K7 or Harbour contact." />
          <Field label="Name they'll see (optional)" value={label} onChange={(e) => setLabel(e.target.value)} hint="Leave empty if they shouldn't learn your name yet." />
          <label className="flex items-start justify-between gap-4">
            <span>
              <span className="block text-[14px] font-medium">Use my verified newsroom identity</span>
              <span className="block text-[13px] text-muted-foreground">Lets them check you against your outlet's website. Leave off for sources who must not be linked to your public identity.</span>
            </span>
            <Switch checked={listed} onChange={setListed} label="Use my verified newsroom identity" />
          </label>
          {error && <Notice tone="danger">{error}</Notice>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button variant="primary" type="submit" loading={busy}>
              Create invite
            </Button>
          </div>
        </form>
      ) : (
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
      )}
    </Dialog>
  );
}

function AcceptDialog({ open, onClose, onAccepted }: { open: boolean; onClose: () => void; onAccepted: (id: string) => void }) {
  const [link, setLink] = useState("");
  const [nickname, setNickname] = useState("");
  const [share, setShare] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const close = () => {
    setLink("");
    setNickname("");
    setShare("");
    setError(null);
    setDone(false);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title={done ? "You're connected" : "Use an invite"} description={done ? undefined : "Paste the invite link someone gave you."}>
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
              const r = await post<{ contact_id: string }>("/api/invites/accept", { link: link.trim(), nickname: nickname.trim(), share_name: share.trim() });
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
          <TextArea label="Invite link" rows={3} value={link} onChange={(e) => setLink(e.target.value)} autoFocus spellCheck={false} placeholder="sotto://invite/…" error={error} />
          <Field label="Your name for them (optional)" value={nickname} onChange={(e) => setNickname(e.target.value)} />
          <Field label="Name they'll see (optional)" value={share} onChange={(e) => setShare(e.target.value)} hint="Leave empty to stay anonymous to them." />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button variant="primary" type="submit" loading={busy} disabled={!link.trim()}>
              Connect
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

function SafetyDialog({ open, onClose, contact }: { open: boolean; onClose: () => void; contact: Contact }) {
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

function TimerDialog({ open, onClose, contact }: { open: boolean; onClose: () => void; contact: Contact }) {
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

function RenameDialog({ open, onClose, contact }: { open: boolean; onClose: () => void; contact: Contact }) {
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

function DeleteDialog({ open, onClose, contact, onDeleted }: { open: boolean; onClose: () => void; contact: Contact; onDeleted: () => void }) {
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
