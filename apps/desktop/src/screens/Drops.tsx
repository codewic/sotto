import { useQuery, useQueryClient } from "@tanstack/react-query";
import { FileText, Link2, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { del, get, post } from "../api";
import { SpotDrop } from "../brand/spots";
import type { DropView, ItemView, SourceView } from "../types";
import { Button, IconButton } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { Chip, CopyButton, Empty, Notice, Skeleton, bytes, dayLabel, timeLabel } from "../ui/bits";
import { useToast } from "../ui/toast";
import { Avatar } from "../ui/Avatar";
import { PaneHeader, useNav } from "./Shell";

export function Drops() {
  const drops = useQuery({ queryKey: ["drops"], queryFn: () => get<DropView[]>("/api/drops") });
  const [dropId, setDropId] = useState<string | null>(null);
  const [connect, setConnect] = useState(false);
  const list = drops.data ?? [];
  const current = list.find((d) => d.id === dropId) ?? list[0] ?? null;

  if (drops.isPending) {
    return (
      <>
        <PaneHeader title="Anonymous inbox" loading />
        <div className="flex flex-col gap-3 p-6">
          <Skeleton className="h-12 w-1/3" />
          <Skeleton className="h-40" />
        </div>
      </>
    );
  }
  if (list.length === 0) {
    return (
      <>
        <PaneHeader title="Anonymous inbox" />
        <div className="grid flex-1 place-items-center">
          <Empty
            art={<SpotDrop />}
            title="Connect your anonymous inbox"
            action={
              <Button variant="primary" icon={<Plus size={16} />} onClick={() => setConnect(true)}>
                Connect an inbox
              </Button>
            }
          >
            An anonymous inbox lets anyone reach your team from Tor Browser without installing anything or creating an account: whistleblowing reports, security vulnerability reports, tips. Each sender gets a codename; you see what they send here, encrypted until it reaches this device.
          </Empty>
        </div>
        <ConnectDialog open={connect} onClose={() => setConnect(false)} onCreated={setDropId} />
      </>
    );
  }
  return (
    <>
      <DropWorkspace drop={current!} drops={list} onSwitch={setDropId} onConnect={() => setConnect(true)} />
      <ConnectDialog open={connect} onClose={() => setConnect(false)} onCreated={setDropId} />
    </>
  );
}

function DropWorkspace({ drop, drops, onSwitch, onConnect }: { drop: DropView; drops: DropView[]; onSwitch: (id: string) => void; onConnect: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const sources = useQuery({ queryKey: ["drop-sources", drop.id], queryFn: () => get<SourceView[]>(`/api/drops/${drop.id}/sources`) });
  const [sourceId, setSourceId] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [setup, setSetup] = useState(false);
  const [remove, setRemove] = useState(false);
  const current = sources.data?.find((s) => s.id === sourceId) ?? null;

  async function sync() {
    setSyncing(true);
    try {
      const r = await post<{ added: number }>(`/api/drops/${drop.id}/sync`);
      toast(r.added ? "success" : "info", r.added ? `${r.added} new item${r.added === 1 ? "" : "s"} downloaded and removed from the inbox server.` : "Nothing new in the inbox.");
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setSyncing(false);
      qc.invalidateQueries({ queryKey: ["drop-sources", drop.id] });
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <section className="flex w-[300px] shrink-0 flex-col border-r border-line">
        <PaneHeader
          title={drop.name || "Anonymous inbox"}
          subtitle={drop.last_sync ? `Checked ${timeLabel(drop.last_sync * 1000)}` : "Not checked yet"}
          loading={syncing}
          actions={
            <IconButton label="Check for new submissions" onClick={sync} disabled={syncing}>
              <RefreshCw size={16} />
            </IconButton>
          }
        />
        {drops.length > 1 && (
          <div className="flex flex-wrap gap-1.5 px-4 pt-3">
            {drops.map((d) => (
              <button key={d.id} onClick={() => onSwitch(d.id)} className={`press h-7 cursor-pointer rounded-full px-3 text-[12.5px] ${d.id === drop.id ? "bg-foreground text-pane" : "bg-muted text-muted-foreground"}`}>
                {d.name}
              </button>
            ))}
          </div>
        )}
        <div className="scroll-pane flex-1 px-2 py-2">
          {sources.isPending && <Skeleton className="m-2 h-14" />}
          {sources.data?.length === 0 && (
            <div className="px-3 py-6 text-[13px] leading-relaxed text-muted-foreground">
              Nothing yet. When someone writes to your inbox, they appear here under a pseudonym.{" "}
              <button className="cursor-pointer font-medium text-foreground underline underline-offset-4" onClick={sync}>
                Check now
              </button>
            </div>
          )}
          {sources.data?.map((s) => (
            <button
              key={s.id}
              onClick={() => setSourceId(s.id)}
              className={`press flex w-full cursor-pointer items-center gap-3 rounded-xl px-3 py-2.5 text-left ${s.id === sourceId ? "bg-muted" : "hover:bg-muted/60"}`}
            >
              <Avatar seed={`sender:${s.id}`} size={36} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[14px] font-medium">{s.pseudonym}</span>
                <span className="block text-[12.5px] text-muted-foreground">Last wrote {dayLabel(s.last_day)}</span>
              </span>
              {s.unread > 0 && <span className="min-w-5 rounded-full bg-primary px-1.5 text-center text-[11px] font-semibold text-primary-foreground tabular">{s.unread}</span>}
            </button>
          ))}
        </div>
        <div className="flex gap-1 border-t border-line p-2">
          <Button size="sm" variant="ghost" className="flex-1" onClick={() => setSetup(true)}>
            Inbox keys
          </Button>
          <Button size="sm" variant="ghost" className="flex-1" icon={<Plus size={14} />} onClick={onConnect}>
            Add inbox
          </Button>
          <IconButton label="Remove this inbox" onClick={() => setRemove(true)}>
            <Trash2 size={15} />
          </IconButton>
        </div>
      </section>
      <section className="flex min-w-0 flex-1 flex-col">
        {current ? (
          <SourceThread key={current.id} drop={drop} source={current} />
        ) : (
          <div className="grid flex-1 place-items-center px-8">
            <div className="max-w-md text-center text-[13.5px] leading-relaxed text-muted-foreground">
              Choose a sender on the left. Senders are shown by pseudonym only. Sotto never learns their codename or anything else about them.
            </div>
          </div>
        )}
      </section>
      <KeysDialog open={setup} onClose={() => setSetup(false)} drop={drop} />
      <RemoveDropDialog open={remove} onClose={() => setRemove(false)} drop={drop} />
    </div>
  );
}

function SourceThread({ drop, source }: { drop: DropView; source: SourceView }) {
  const qc = useQueryClient();
  const toast = useToast();
  const { go } = useNav();
  const items = useQuery({ queryKey: ["drop-items", drop.id, source.id], queryFn: () => get<ItemView[]>(`/api/drops/${drop.id}/sources/${source.id}`) });
  const [reply, setReply] = useState("");
  const [sending, setSending] = useState(false);
  const [reviewing, setReviewing] = useState<string | null>(null);

  useEffect(() => {
    qc.invalidateQueries({ queryKey: ["drop-sources", drop.id] });
    qc.invalidateQueries({ queryKey: ["drops"] });
  }, [items.data, drop.id, qc]);

  async function insertInvite() {
    try {
      const r = await post<{ link: string }>("/api/invites", { nickname: source.pseudonym, label: "", listed: false });
      setReply((t) => `${t}${t ? "\n\n" : ""}To keep talking more safely, install Sotto and use this one-time invite:\n${r.link}`);
      toast("info", `An invite for ${source.pseudonym} was added to Conversations.`);
    } catch (e) {
      toast("error", (e as Error).message);
    }
  }

  async function review(it: ItemView) {
    setReviewing(it.name);
    try {
      const r = await post<{ key: string }>(`/api/drops/${drop.id}/sources/${source.id}/items/${it.name}/review`);
      go("review", r.key);
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setReviewing(null);
    }
  }

  return (
    <>
      <PaneHeader title={source.pseudonym} subtitle={`First wrote ${dayLabel(source.first_day)}. Dates are kept to the day only.`} />
      <div className="scroll-pane flex-1 px-6 py-5">
        {items.isPending && <Skeleton className="h-24" />}
        <ol className="flex flex-col gap-3">
          {items.data?.map((it) => (
            <li key={it.name} className={`flex flex-col ${it.kind === "reply" ? "items-end" : "items-start"}`}>
              <div className={`max-w-[72%] rounded-2xl px-4 py-3 text-[14px] leading-relaxed ${it.kind === "reply" ? "rounded-br-md bg-foreground text-pane" : "rounded-bl-md bg-muted"}`}>
                {it.kind !== "file" && <p className="break-words whitespace-pre-wrap">{it.text}</p>}
                {it.kind === "file" && (
                  <div className="flex min-w-[260px] flex-col gap-2">
                    <div className="flex items-center gap-2.5">
                      <span className="grid size-9 place-items-center rounded-lg bg-pane">
                        <FileText size={18} />
                      </span>
                      <div className="min-w-0">
                        <div className="truncate font-medium">{it.file_name}</div>
                        <div className="text-[12px] text-muted-foreground">{bytes(it.size ?? 0)}</div>
                      </div>
                    </div>
                    {it.review.state === "quarantined" && (
                      <div className="flex items-center justify-between gap-3 rounded-xl bg-pane px-3 py-2">
                        <span className="text-[12.5px] text-muted-foreground">Quarantined until reviewed</span>
                        <Button size="sm" variant="primary" onClick={() => review(it)} loading={reviewing === it.name}>
                          {reviewing === it.name ? "Opening safely…" : "Review safely"}
                        </Button>
                      </div>
                    )}
                    {it.review.state === "clean" && it.key && (
                      <Button size="sm" onClick={() => go("review", it.key!)}>
                        Open clean copy · {it.review.pages} page{it.review.pages === 1 ? "" : "s"}
                      </Button>
                    )}
                    {it.review.state === "failed" && <span className="text-[12.5px] text-destructive">{it.review.reason}</span>}
                  </div>
                )}
              </div>
              <span className="mt-0.5 px-1 text-[11.5px] text-muted-foreground">
                {it.kind === "reply" ? "You replied" : "Sender"} · {dayLabel(it.day)}
              </span>
            </li>
          ))}
        </ol>
      </div>
      <form
        className="shrink-0 px-6 pt-2 pb-5"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!reply.trim()) return;
          setSending(true);
          try {
            await post(`/api/drops/${drop.id}/sources/${source.id}/reply`, { text: reply });
            setReply("");
            toast("success", "Reply sealed. They'll see it next time they use their codename.");
          } catch (err) {
            toast("error", (err as Error).message);
          } finally {
            setSending(false);
          }
        }}
      >
        <div className="rounded-2xl bg-muted p-2">
          <textarea
            value={reply}
            onChange={(e) => setReply(e.target.value)}
            rows={Math.min(8, Math.max(2, reply.split("\n").length))}
            placeholder={`Reply to ${source.pseudonym}`}
            aria-label="Reply"
            className="w-full resize-none bg-transparent px-2 py-1.5 text-[14px] leading-relaxed outline-none placeholder:text-muted-foreground"
          />
          <div className="flex items-center justify-between gap-2 px-1">
            <Button type="button" size="sm" variant="ghost" icon={<Link2 size={14} />} onClick={insertInvite}>
              Add an invite to Sotto
            </Button>
            <Button size="sm" variant="primary" type="submit" loading={sending} disabled={!reply.trim()}>
              Send reply
            </Button>
          </div>
        </div>
        <p className="mt-2 px-1 text-[12px] text-muted-foreground">Replies are sealed to this sender's codename. The inbox server can't read them.</p>
      </form>
    </>
  );
}

function ConnectDialog({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (id: string) => void }) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<DropView | null>(null);
  const close = () => {
    setCreated(null);
    setName("");
    setUrl("");
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title={created ? "Add these keys to your inbox server" : "Connect an anonymous inbox"} width={created ? 560 : 480}>
      {created ? (
        <KeysBody drop={created} onDone={close} />
      ) : (
        <form
          className="flex flex-col gap-6"
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError(null);
            try {
              const d = await post<DropView>("/api/drops", { name: name.trim(), url: url.trim() });
              await qc.invalidateQueries({ queryKey: ["drops"] });
              onCreated(d.id);
              setCreated(d);
            } catch (err) {
              setError((err as Error).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Whistleblowing reports" autoFocus />
          <Field label="Inbox address" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="http://…onion" spellCheck={false} error={error} hint="The onion address your inbox server prints when it starts." />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button variant="primary" type="submit" loading={busy} disabled={!url.trim()}>
              Create keys
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

function KeysBody({ drop, onDone }: { drop: DropView; onDone: () => void }) {
  const config = JSON.stringify({ newsroom_key: drop.newsroom_key, journalists: [{ name: "This device", sign_pk: drop.journalist_key }] }, null, 2);
  return (
    <div className="flex flex-col gap-4">
      <p className="text-[13.5px] text-muted-foreground">
        Put these two values in the inbox server's <span className="font-mono text-foreground">drop.json</span>. The organization key lets the server seal what people send to you; the device key lets this app fetch it. Neither is secret, but check you are editing your own server.
      </p>
      <pre className="max-h-56 overflow-auto rounded-xl bg-muted p-3 font-mono text-[11.5px] leading-relaxed break-all whitespace-pre-wrap text-muted-foreground">{config}</pre>
      <div className="flex items-center justify-between">
        <CopyButton text={config} label="Copy settings" />
        <Button variant="primary" onClick={onDone}>
          Done
        </Button>
      </div>
      <Notice>Colleagues can share one organization key. Each device still gets its own device key, so access can be removed one person at a time.</Notice>
    </div>
  );
}

function KeysDialog({ open, onClose, drop }: { open: boolean; onClose: () => void; drop: DropView }) {
  return (
    <Dialog open={open} onClose={onClose} title={`${drop.name || "Inbox"} keys`} width={560}>
      <div className="mb-4 flex items-center gap-2 text-[13px] text-muted-foreground">
        <Chip>{drop.url}</Chip>
      </div>
      <KeysBody drop={drop} onDone={onClose} />
    </Dialog>
  );
}

function RemoveDropDialog({ open, onClose, drop }: { open: boolean; onClose: () => void; drop: DropView }) {
  const qc = useQueryClient();
  return (
    <Dialog open={open} onClose={onClose} title={`Remove ${drop.name || "this inbox"}?`} description="Every sender, message and file downloaded from this inbox is erased from this device. The inbox server itself is not touched.">
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="danger"
          onClick={async () => {
            await del(`/api/drops/${drop.id}`);
            await qc.invalidateQueries({ queryKey: ["drops"] });
            onClose();
          }}
        >
          Remove inbox
        </Button>
      </div>
    </Dialog>
  );
}
