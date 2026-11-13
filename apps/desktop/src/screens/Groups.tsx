import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AtSign, BellOff, Check, ChevronLeft, Copy, Info, MoreHorizontal, Paperclip, Pencil, Phone, Pin, PinOff, Reply, Search, Shield, ShieldOff, Trash2, UserMinus, Users, X } from "lucide-react";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { type ApiError, del, get, patch, post } from "../api";
import type { Contact, GroupView, Message } from "../types";
import { Button, IconButton } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { plain } from "../ui/Rich";
import { Avatar, seedOf } from "../ui/Avatar";
import { SearchBar, useThreadSearch } from "../ui/ThreadSearch";
import { Notice, Skeleton, Switch, ttlLabel } from "../ui/bits";
import { useToast } from "../ui/toast";
import { Composer, type ComposerHandle, DraftLine, type Member, type SendAs, uploadFile } from "../ui/Composer";
import { Album, PhotoViewer, type ViewerItem, isPhotoFile, withAlbums } from "../ui/Photo";
import { type VideoItem, VideoViewer } from "../ui/Media";
import { useCalls } from "./Calls";
import { contactName } from "./ConversationDialogs";
import { AlbumMeta, Bubble, ContextMenu, MenuItem } from "./Conversations";
import { useNav } from "./Shell";
import { useMyAvatar } from "./AvatarPicker";

const GROUP_MS = 5 * 60 * 1000;

function initials(name: string) {
  const parts = name
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  return ((parts[0]?.[0] ?? "?") + (parts[1]?.[0] ?? "")).toUpperCase();
}

/** Groups have a rounded-square mark, conversations a circle, so the list reads at a glance. */
export function GroupAvatar({ title, size = 44 }: { title: string; size?: number }) {
  return (
    <span
      className="grid shrink-0 place-items-center rounded-[30%] bg-[color-mix(in_srgb,var(--primary)_14%,var(--muted-strong))] font-semibold text-primary"
      style={{ width: size, height: size, fontSize: size * 0.34 }}
      aria-hidden="true"
    >
      {initials(title)}
    </span>
  );
}

function listTime(sec: number) {
  const d = new Date(sec * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

export function GroupRow({ g, active, onOpen }: { g: GroupView; active: boolean; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className={`flex w-full cursor-pointer items-center gap-3 rounded-xl px-2.5 py-[var(--row-py)] text-left transition-colors duration-150 ${active ? "bg-muted" : "hover:bg-muted/60"}`}
    >
      <GroupAvatar title={g.title} />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-1.5">
          <span className="truncate text-[14.5px] font-semibold">{g.title}</span>
          <Users size={12} className="shrink-0 text-muted-foreground" aria-label="Group" />
          {g.muted && <BellOff size={12} className="shrink-0 text-muted-foreground" aria-label="Muted" />}
        </span>
        <span className="mt-0.5 flex items-center gap-2">
          <span className="truncate text-[13px] text-muted-foreground">
            {g.removed ? (
              "You're no longer in this group"
            ) : (
              <DraftLine draftKey={`g:${g.id}`} off={active}>
                {plain(g.preview) || `${g.member_count} members`}
              </DraftLine>
            )}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-1.5">
            <span className="text-[11.5px] text-muted-foreground tabular">{listTime(g.preview_at || g.last_activity)}</span>
            {g.mentions > 0 && (
              <span className="grid size-5 place-items-center rounded-full bg-primary text-primary-foreground" role="img" aria-label={g.mentions === 1 ? "You were mentioned" : `You were mentioned ${g.mentions} times`} title="You were mentioned">
                <AtSign size={12} strokeWidth={2.5} />
              </span>
            )}
            {g.unread > 0 && (
              <span className={`min-w-5 rounded-full px-1.5 text-center text-[11px] leading-5 font-semibold tabular ${g.muted ? "bg-muted-strong text-muted-foreground" : "bg-primary text-primary-foreground"}`}>{g.unread}</span>
            )}
          </span>
        </span>
      </span>
    </button>
  );
}

type Day = { day: string; runs: Message[][] };

function arrange(ms: Message[]): Day[] {
  const out: Day[] = [];
  for (const m of ms) {
    const day = new Date(m.received_at).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    let g = out[out.length - 1];
    if (!g || g.day !== day) {
      g = { day, runs: [] };
      out.push(g);
    }
    const run = g.runs[g.runs.length - 1];
    const last = run?.[run.length - 1];
    const same = last && last.kind !== "system" && m.kind !== "system" && last.outgoing === m.outgoing && last.author === m.author && m.received_at - last.received_at < GROUP_MS;
    if (same) run.push(m);
    else g.runs.push([m]);
  }
  return out;
}

export function GroupThread({ group, contacts, narrow, onBack, onGone }: { group: GroupView; contacts: Contact[]; narrow: boolean; onBack: () => void; onGone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const calls = useCalls();
  const messages = useQuery({ queryKey: ["group-messages", group.id], queryFn: () => get<Message[]>(`/api/groups/${group.id}/messages`) });
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const [editing, setEditing] = useState<Message | null>(null);
  const [info, setInfo] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const composer = useRef<ComposerHandle>(null);
  const list = useMemo(() => messages.data ?? [], [messages.data]);
  const byId = useMemo(() => new Map(list.map((m) => [m.id, m])), [list]);
  const days = useMemo(() => arrange(list), [list]);
  const { go, status } = useNav();
  const manager = !group.removed && (group.owner || group.am_admin);
  const [menu, setMenu] = useState<{ m: Message; x: number; y: number } | null>(null);
  const [reviewing, setReviewing] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);
  const [pinIndex, setPinIndex] = useState(0);
  const searchable = useMemo(() => list.filter((m) => m.kind !== "system"), [list]);
  const search = useThreadSearch(searchable, (m) => `${m.text ?? ""} ${m.file?.name ?? ""} ${m.file?.caption ?? ""} ${m.author ?? ""}`, (m) => m.id, scroller, "data-mid");
  // Newest pin first; clicking the bar jumps to it and moves to the next older one.
  const pinned = useMemo(() => [...group.pins].reverse().map((id) => byId.get(id)).filter((m): m is Message => !!m), [group.pins, byId]);
  const shownPin = pinned.length ? pinned[pinIndex % pinned.length] : null;

  const jump = (id: string) => {
    scroller.current?.querySelector(`[data-mid="${id}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
    setFlash(id);
    setTimeout(() => setFlash((f) => (f === id ? null : f)), 1600);
  };

  // Opening a group marks it read; new messages while it's open do too.
  useEffect(() => {
    if (group.unread > 0) void post(`/api/groups/${group.id}/read`).then(() => qc.invalidateQueries({ queryKey: ["groups"] }));
  }, [group.id, group.unread, qc]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [list.length]);

  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["group-messages", group.id] });
    void qc.invalidateQueries({ queryKey: ["groups"] });
  };

  async function sendText(body: string) {
    if (editing) await patch(`/api/groups/${group.id}/messages/${editing.id}`, { text: body });
    else await post(`/api/groups/${group.id}/messages`, { text: body, reply_to: replyTo?.id ?? null });
    setReplyTo(null);
    setEditing(null);
    refresh();
  }

  async function sendFile(file: File, caption: string, sendAs: SendAs) {
    await uploadFile(`/api/groups/${group.id}/files`, file, caption, sendAs);
    setReplyTo(null);
    refresh();
  }

  const react = (m: Message, emoji: string) =>
    post(`/api/groups/${group.id}/messages/${m.id}/react`, { emoji, on: !m.reactions.some((r) => r.emoji === emoji && r.mine) })
      .then(refresh)
      .catch((e: ApiError) => toast("error", e.message));

  const startEdit = (m: Message) => {
    setReplyTo(null);
    setEditing(m);
    requestAnimationFrame(() => {
      composer.current?.setText(m.text ?? "");
      composer.current?.focus();
    });
  };

  async function review(m: Message) {
    setReviewing(m.id);
    try {
      const report = await post<{ key: string }>(`/api/groups/${group.id}/messages/${m.id}/review`);
      go("review", report.key);
    } catch (e) {
      toast("error", (e as ApiError).message);
    } finally {
      setReviewing(null);
    }
  }

  const setPin = (m: Message, on: boolean) =>
    post(`/api/groups/${group.id}/messages/${m.id}/pin`, { on })
      .then(() => qc.invalidateQueries({ queryKey: ["groups"] }))
      .then(refresh)
      .catch((e: ApiError) => toast("error", e.message));

  const remove = (m: Message) =>
    del(`/api/groups/${group.id}/messages/${m.id}`)
      .then(refresh)
      .catch((e: ApiError) => toast("error", e.message));

  // Members who are also your conversations can be called together.
  const callable = group.members.map((m) => contacts.find((c) => c.id === m.contact_id && c.state === "ready")).filter((c): c is Contact => !!c);
  const noun = group.member_count === 1 ? "member" : "members";
  // Who you can @mention: everyone who has joined, except you.
  const mentionable: Member[] = useMemo(
    () =>
      group.members
        .filter((m) => !m.you && !m.pending)
        .map((m) => {
          const local = contacts.find((c) => c.id === m.contact_id);
          return { name: m.name, seed: local ? seedOf(local) : `member:${m.name}`, choice: local?.avatar };
        }),
    [group.members, contacts],
  );
  const callBusy = calls.calls.some((c) => c.phase !== "ended");
  const [viewing, setViewing] = useState<string | null>(null);
  const [watching, setWatching] = useState<VideoItem | null>(null);
  const photos: ViewerItem[] = useMemo(
    () =>
      list
        .filter((m) => m.kind === "file" && m.file && isPhotoFile(m.file.mime) && (m.outgoing || m.file.status === "ready"))
        .map((m) => ({ key: `s:sblob/g/${group.id}/${m.id}`, own: m.outgoing, name: m.file?.name ?? "Photo", caption: m.file?.caption, who: m.outgoing ? "You" : (m.author ?? "Someone"), at: m.received_at })),
    [list, group.id],
  );
  const mentionNames = useMemo(() => ({ names: group.members.map((m) => m.name), me: group.members.find((m) => m.you)?.name }), [group.members]);

  return (
    <div className="@container/thread relative flex min-h-0 flex-1">
      <div
        className="relative flex min-w-0 flex-1 flex-col"
        onDragOver={(e) => {
          if (group.removed || !e.dataTransfer.types.includes("Files")) return;
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(e) => {
          if (e.currentTarget === e.target) setDragging(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          if (!group.removed && e.dataTransfer.files.length) {
            setEditing(null);
            composer.current?.addFiles(Array.from(e.dataTransfer.files));
          }
        }}
      >
        <header className="flex h-16 shrink-0 items-center gap-3 px-4">
          {narrow && (
            <IconButton label="Back to conversations" onClick={onBack}>
              <ChevronLeft size={18} />
            </IconButton>
          )}
          <button type="button" onClick={() => setInfo((v) => !v)} className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left">
            <GroupAvatar title={group.title} size={36} />
            <span className="min-w-0">
              <span className="block truncate text-[15.5px] font-semibold tracking-tight">{group.title}</span>
              <span className="block truncate text-[12.5px] text-muted-foreground">
                {group.member_count} {noun} · end-to-end encrypted{group.ttl_secs ? ` · disappears after ${ttlLabel(group.ttl_secs).toLowerCase()}` : ""}
              </span>
            </span>
          </button>
          <IconButton label="Search this group" aria-pressed={search.open} onClick={() => (search.open ? search.close() : search.show())}>
            <Search size={17} />
          </IconButton>
          {callable.length > 0 && !group.removed && (
            <IconButton label="Call the group" onClick={() => void calls.call(callable[0], callable.slice(1))} disabled={calls.calls.some((c) => c.phase !== "ended")}>
              <Phone size={17} />
            </IconButton>
          )}
          <IconButton label="Group info" aria-pressed={info} onClick={() => setInfo((v) => !v)}>
            <Info size={17} />
          </IconButton>
        </header>
        {search.open && <SearchBar search={search} placeholder={`Search ${group.title}`} />}
        {shownPin && (
          <div className="flex h-12 shrink-0 items-center gap-2 px-4">
            <button
              type="button"
              onClick={() => {
                jump(shownPin.id);
                setPinIndex((i) => i + 1);
              }}
              className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 rounded-xl px-2 py-1.5 text-left hover:bg-muted"
            >
              <span className="flex h-8 shrink-0 flex-col gap-0.5" aria-hidden="true">
                {pinned.slice(0, 4).map((p) => (
                  <span key={p.id} className={`w-0.5 flex-1 rounded-full ${p.id === shownPin.id ? "bg-primary" : "bg-muted-strong"}`} />
                ))}
              </span>
              <span className="min-w-0">
                <span className="block text-[12px] font-semibold text-primary">
                  Pinned{pinned.length > 1 ? ` · ${(pinIndex % pinned.length) + 1} of ${pinned.length}` : ""}
                </span>
                <span className="block truncate text-[13px] text-muted-foreground">{shownPin.text ?? shownPin.file?.name ?? ""}</span>
              </span>
            </button>
            {manager && (
              <IconButton label="Unpin this message" onClick={() => void setPin(shownPin, false)}>
                <PinOff size={16} />
              </IconButton>
            )}
          </div>
        )}
        <div className="h-px shrink-0 bg-line" />
        <div ref={scroller} className="scroll-pane relative flex-1 overflow-x-hidden px-4 pt-2 pb-4 sm:px-8">
          {messages.isPending && (
            <div className="flex flex-col gap-2 pt-6" aria-label="Loading messages">
              <Skeleton className="h-3 w-16" />
              <Skeleton className="h-10 w-2/5 rounded-2xl" />
              <Skeleton className="ml-auto h-10 w-1/3 rounded-2xl" />
              <Skeleton className="h-3 w-20" />
              <Skeleton className="h-16 w-1/2 rounded-2xl" />
            </div>
          )}
          {days.map((d) => (
            <section key={d.day}>
              <div className="sticky top-1 z-[1] my-3 flex justify-center">
                <span className="rounded-full bg-pane/90 px-3 py-1 text-[12px] font-medium text-muted-foreground shadow-pane backdrop-blur">{d.day}</span>
              </div>
              <div className="flex flex-col gap-[var(--run-gap)]">
                {d.runs.map((run) => (
                  <div key={run[0].id} className={`flex flex-col gap-0.5 ${run[0].kind === "system" ? "items-center" : run[0].outgoing ? "items-end" : "items-start"}`}>
                    {run[0].kind !== "system" && !run[0].outgoing && <div className="mb-0.5 ml-3 text-[12px] font-semibold text-muted-foreground">{run[0].author}</div>}
                    {withAlbums(run).map((it) =>
                      it.kind === "album" ? (
                        <Album
                          key={it.ms[0].id}
                          ms={it.ms}
                          keyOf={(m) => `s:sblob/g/${group.id}/${m.id}`}
                          mine={it.ms[0].outgoing}
                          meta={<AlbumMeta last={it.ms[it.ms.length - 1]} />}
                          onOpen={setViewing}
                          onMenu={(m, x, y) => setMenu({ m, x, y })}
                        />
                      ) : (
                        ((m: Message, i: number) => (
                      <Fragment key={m.id}>
                        <Bubble
                          m={m}
                          first={i === 0}
                          last={i === run.length - 1}
                          reply={m.reply_to ? byId.get(m.reply_to) : undefined}
                          replyName={(r) => (r.outgoing ? "You" : (r.author ?? "Someone"))}
                          photoKey={`s:sblob/g/${group.id}/${m.id}`}
                          onOpenPhoto={() => setViewing(`s:sblob/g/${group.id}/${m.id}`)}
                          onOpenVideo={() => setWatching({ key: `s:sblob/g/${group.id}/${m.id}`, own: m.outgoing, name: m.file?.name ?? "Video", who: m.outgoing ? "You" : (m.author ?? "Someone"), at: m.received_at })}
                          mentions={mentionNames}
                          flash={flash === m.id || search.current === m.id}
                          reviewing={reviewing === m.id}
                          onReply={() => {
                            setEditing(null);
                            setReplyTo(m);
                            composer.current?.focus();
                          }}
                          onReact={(e) => void react(m, e)}
                          onEdit={() => startEdit(m)}
                          onCopy={() => navigator.clipboard.writeText(m.text ?? "").then(() => toast("info", "Copied. Clear your clipboard when you're done."))}
                          onMenu={(x, y) => setMenu({ m, x, y })}
                          onJump={jump}
                          onReview={() => void review(m)}
                          onOpenReview={() => go("review", `s:sblob/g/${group.id}/${m.id}`)}
                          onRetry={() => {}}
                        />
                      </Fragment>
                        ))(it.m, it.i)
                      ),
                    )}
                  </div>
                ))}
              </div>
            </section>
          ))}
        </div>
        <div className="relative shrink-0 px-4 pt-1 pb-4 sm:px-8">
          {group.removed ? (
            <Notice>You're no longer in this group. You can still read what you had, or delete it from the group info.</Notice>
          ) : (
            <Composer
              handle={composer}
              draftKey={`g:${group.id}`}
              placeholder={`Message ${group.title}`}
              editing={!!editing}
              banner={
                (replyTo || editing) && (
                  <div className="flex items-center gap-3 border-b border-line px-4 py-2">
                    {editing ? <Pencil size={15} className="text-primary" /> : <Reply size={15} className="text-primary" />}
                    <div className="min-w-0 flex-1 border-l-2 border-primary pl-2.5">
                      <div className="text-[12px] font-semibold text-primary">{editing ? "Editing" : `Replying to ${replyTo?.outgoing ? "yourself" : (replyTo?.author ?? "someone")}`}</div>
                      <div className="truncate text-[12.5px] text-muted-foreground">{plain((editing ?? replyTo)?.text ?? (editing ?? replyTo)?.file?.name ?? "")}</div>
                    </div>
                    <IconButton
                      label="Cancel"
                      type="button"
                      onClick={() => {
                        setReplyTo(null);
                        setEditing(null);
                      }}
                    >
                      <X size={15} />
                    </IconButton>
                  </div>
                )
              }
              onSendText={sendText}
              onSendFile={sendFile}
              onEscape={() => {
                setReplyTo(null);
                setEditing(null);
              }}
              onArrowUpEmpty={() => {
                const mine = [...list].reverse().find((m) => m.outgoing && m.kind === "text");
                if (mine) startEdit(mine);
              }}
              members={mentionable}
              commands={callable.length && !callBusy ? [{ name: "call", hint: "Call everyone you can reach in this group", run: () => void calls.call(callable[0], callable.slice(1)) }] : []}
              schedule={{ kind: "g", target: group.id }}
              risk={status.settings?.risk_mode}
            />
          )}
        </div>
      </div>
        {dragging && (
          <div className="pointer-events-none absolute inset-3 grid place-items-center rounded-2xl bg-[color-mix(in_srgb,var(--primary)_10%,var(--pane))] shadow-[inset_0_0_0_2px_var(--primary)]">
            <div className="text-center">
              <Paperclip className="mx-auto text-primary" size={28} />
              <p className="mt-2 text-[15px] font-semibold">Drop to share with {group.title}</p>
              <p className="text-[13px] text-muted-foreground">It's encrypted on this computer, and arrives quarantined for everyone else.</p>
            </div>
          </div>
        )}
      {info && <GroupInfo group={group} contacts={contacts} floating={narrow} onClose={() => setInfo(false)} onGone={onGone} />}
      {viewing && photos.some((p) => p.key === viewing) && <PhotoViewer items={photos} start={photos.findIndex((p) => p.key === viewing)} onClose={() => setViewing(null)} />}
      {watching && <VideoViewer item={watching} onClose={() => setWatching(null)} />}
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          {!group.removed && (
            <MenuItem
              icon={<Reply size={15} />}
              onClick={() => {
                setEditing(null);
                setReplyTo(menu.m);
                composer.current?.focus();
              }}
            >
              Reply
            </MenuItem>
          )}
          {menu.m.text && (
            <MenuItem icon={<Copy size={15} />} onClick={() => void navigator.clipboard.writeText(menu.m.text ?? "").then(() => toast("info", "Copied. Clear your clipboard when you're done."))}>
              Copy text
            </MenuItem>
          )}
          {manager &&
            (group.pins.includes(menu.m.id) ? (
              <MenuItem icon={<PinOff size={15} />} onClick={() => void setPin(menu.m, false)}>
                Unpin
              </MenuItem>
            ) : (
              <MenuItem icon={<Pin size={15} />} onClick={() => void setPin(menu.m, true)}>
                Pin for everyone
              </MenuItem>
            ))}
          {menu.m.outgoing && menu.m.kind === "text" && !group.removed && (
            <MenuItem icon={<Pencil size={15} />} onClick={() => startEdit(menu.m)}>
              Edit
            </MenuItem>
          )}
          <MenuItem danger icon={<Trash2 size={15} />} onClick={() => void remove(menu.m)}>
            {menu.m.outgoing && !group.removed ? "Delete for everyone" : "Delete for me"}
          </MenuItem>
        </ContextMenu>
      )}
    </div>
  );
}

function GroupInfo({ group, contacts, floating, onClose, onGone }: { group: GroupView; contacts: Contact[]; floating: boolean; onClose: () => void; onGone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [adding, setAdding] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [title, setTitle] = useState(group.title);
  const run = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      await qc.invalidateQueries({ queryKey: ["groups"] });
      await qc.invalidateQueries({ queryKey: ["group-messages", group.id] });
    } catch (e) {
      toast("error", (e as ApiError).message);
    }
  };
  const addable = contacts.filter((c) => c.state === "ready" && !group.members.some((m) => m.contact_id === c.id));
  const manage = !group.removed && (group.owner || group.am_admin);
  const me = useMyAvatar();
  const [memberMenu, setMemberMenu] = useState<{ m: GroupView["members"][number]; x: number; y: number } | null>(null);
  const safety = useQuery({ queryKey: ["group-safety", group.id, group.member_count], queryFn: () => get<{ number: string; members: number }>(`/api/groups/${group.id}/safety`), enabled: !group.removed });
  const rename = () => {
    if (title.trim() && title.trim() !== group.title) void run(() => patch(`/api/groups/${group.id}`, { title }));
  };
  return (
    <aside className={`scroll-pane flex w-[320px] max-w-full shrink-0 flex-col border-l border-line bg-pane ${floating ? "absolute inset-y-0 right-0 z-10 border-l-0 shadow-float" : "@max-[760px]/thread:absolute @max-[760px]/thread:inset-y-0 @max-[760px]/thread:right-0 @max-[760px]/thread:z-10 @max-[760px]/thread:border-l-0 @max-[760px]/thread:shadow-float"}`} aria-label="Group info">
      <div className="flex h-16 shrink-0 items-center justify-between px-4">
        <span className="text-[14.5px] font-semibold">Group info</span>
        <IconButton label="Close group info" onClick={onClose}>
          <X size={17} />
        </IconButton>
      </div>
      <div className="flex flex-col items-center px-5 pb-5 text-center">
        <GroupAvatar title={group.title} size={76} />
        {manage ? (
          <form
            className="mt-3 w-full"
            onSubmit={(e) => {
              e.preventDefault();
              rename();
            }}
          >
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onBlur={rename}
              aria-label="Group name"
              className="w-full rounded-lg bg-transparent px-2 py-1 text-center text-[17px] font-semibold outline-none hover:bg-muted focus:bg-muted"
            />
          </form>
        ) : (
          <div className="mt-3 text-[17px] font-semibold">{group.title}</div>
        )}
        <div className="text-[12.5px] text-muted-foreground">
          End-to-end encrypted group · {group.member_count} {group.member_count === 1 ? "member" : "members"}
        </div>
        {group.am_admin && <div className="mt-1 text-[12.5px] font-medium text-primary">You're an admin</div>}
      </div>
      <section className="border-t border-line px-5 py-4">
        <div className="flex items-center justify-between">
          <h3 className="text-[13px] font-semibold">Members</h3>
          {manage && addable.length > 0 && (
            <button type="button" onClick={() => setAdding((v) => !v)} className="cursor-pointer text-[12.5px] font-medium text-primary hover:underline">
              {adding ? "Done" : "Add people"}
            </button>
          )}
        </div>
        <ul className="mt-2 flex flex-col">
          {group.members.map((m, i) => {
            // Your own label for someone you already talk to; otherwise the name they chose.
            const local = contacts.find((c) => c.id === m.contact_id);
            const shown = m.you ? "You" : local ? contactName(local) : m.name;
            return (
            <li key={`${i}-${m.name}`} className="flex h-10 items-center gap-2.5">
              <Avatar seed={m.you ? me.seed : local ? seedOf(local) : `member:${m.name}`} choice={m.you ? me.choice : local?.avatar} size={28} />
              <span className="min-w-0 flex-1 truncate text-[13.5px]">{shown}</span>
              {m.owner && <span className="text-[11.5px] text-muted-foreground">Made the group</span>}
              {m.admin && <span className="rounded-full bg-[color-mix(in_srgb,var(--primary)_12%,transparent)] px-2 py-0.5 text-[11px] font-medium text-primary">Admin</span>}
              {m.pending && <span className="text-[11.5px] text-muted-foreground">Joining…</span>}
              {manage && !m.owner && !m.you && (
                <IconButton
                  label={`More for ${shown}`}
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect();
                    setMemberMenu({ m, x: r.left - 160, y: r.bottom + 4 });
                  }}
                >
                  <MoreHorizontal size={15} />
                </IconButton>
              )}
            </li>
            );
          })}
        </ul>
        {adding && (
          <ul className="mt-2 flex flex-col rounded-xl bg-muted p-1">
            {addable.map((c) => (
              <li key={c.id}>
                <button
                  type="button"
                  onClick={() => void run(() => post(`/api/groups/${group.id}/members`, { contact_id: c.id }))}
                  className="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2 text-left text-[13px] hover:bg-pane"
                >
                  <Avatar seed={seedOf(c)} size={24} />
                  <span className="flex-1 truncate">{contactName(c)}</span>
                  <span className="text-[12px] text-primary">Add</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
      {memberMenu && (
        <ContextMenu x={memberMenu.x} y={memberMenu.y} onClose={() => setMemberMenu(null)}>
          {group.owner && !memberMenu.m.pending && (
            <MenuItem
              icon={memberMenu.m.admin ? <ShieldOff size={15} /> : <Shield size={15} />}
              onClick={() => void run(() => post(`/api/groups/${group.id}/admins`, { name: memberMenu.m.name, on: !memberMenu.m.admin }))}
            >
              {memberMenu.m.admin ? "Remove as admin" : "Make admin"}
            </MenuItem>
          )}
          <MenuItem
            danger
            icon={<UserMinus size={15} />}
            onClick={() =>
              void run(async () => {
                await del(`/api/groups/${group.id}/members/${encodeURIComponent(memberMenu.m.contact_id ?? memberMenu.m.name)}`);
                if (!group.owner && !memberMenu.m.pending) toast("info", "Asked to remove them. It happens when the group's creator is next online.");
              })
            }
          >
            {memberMenu.m.pending ? "Stop inviting" : "Remove from group"}
          </MenuItem>
        </ContextMenu>
      )}
      {manage && (
        <section className="border-t border-line px-5 py-4">
          <h3 className="text-[13px] font-semibold">Disappearing messages</h3>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {[0, 3600, 86_400, 604_800, 2_592_000].map((s) => (
              <button
                key={s}
                type="button"
                onClick={() => void run(() => patch(`/api/groups/${group.id}`, { ttl_secs: s }))}
                className={`press h-7 cursor-pointer rounded-full px-3 text-[12.5px] font-medium ${(group.ttl_secs ?? 0) === s ? "bg-foreground text-pane" : "bg-muted text-muted-foreground hover:text-foreground"}`}
              >
                {s === 0 ? "Off" : ttlLabel(s)}
              </button>
            ))}
          </div>
        </section>
      )}
      <section className="flex items-center gap-3 border-t border-line px-5 py-4">
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-semibold">Mute</h3>
          <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">Unread counts stay grey and don't add to the badge.</p>
        </div>
        <Switch checked={group.muted} label="Mute this group" onChange={(v) => void run(() => patch(`/api/groups/${group.id}`, { muted: v }))} />
      </section>
      {!group.removed && (
        <section className="border-t border-line px-5 py-4">
          <h3 className="text-[13px] font-semibold">Safety number</h3>
          {safety.data ? (
            <p className="mt-2 grid grid-cols-4 gap-x-3 gap-y-1 font-mono text-[13.5px] tabular" aria-label={`Safety number ${safety.data.number}`}>
              {safety.data.number.split(" ").map((g, i) => (
                <span key={i}>{g}</span>
              ))}
            </p>
          ) : (
            <Skeleton className="mt-2 h-10 w-full rounded-lg" />
          )}
          <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
            Everyone in the group sees the same number. Compare it with members in person or on a call: if it matches, nobody else is listening in. It changes when someone joins or leaves.
          </p>
        </section>
      )}
      <section className="mt-auto border-t border-line px-3 py-3">
        <button
          type="button"
          onClick={() => setConfirm(true)}
          className="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2 text-[13.5px] text-destructive hover:bg-[color-mix(in_srgb,var(--destructive)_8%,transparent)]"
        >
          {group.owner && !group.removed ? "Delete group for everyone" : "Leave and delete group"}
        </button>
      </section>
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title={group.owner && !group.removed ? `Delete ${group.title} for everyone?` : `Leave ${group.title}?`}
        description={
          group.owner && !group.removed
            ? "Everyone is told the group is gone, and its messages are deleted from this computer. Others keep what they already have."
            : "You leave the group and its messages are deleted from this computer. Others keep what they already have."
        }
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setConfirm(false)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={async () => {
              setConfirm(false);
              await run(() => del(`/api/groups/${group.id}`));
              onGone();
            }}
          >
            {group.owner && !group.removed ? "Delete group" : "Leave group"}
          </Button>
        </div>
      </Dialog>
    </aside>
  );
}

export function NewGroupDialog({ open, contacts, onClose, onCreated }: { open: boolean; contacts: Contact[]; onClose: () => void; onCreated: (id: string) => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ready = contacts.filter((c) => c.state === "ready");
  const close = () => {
    setTitle("");
    setPicked(new Set());
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title="New group" description="Everyone you add sees everyone else in the group. Messages are end-to-end encrypted." width={460}>
      <form
        className="flex flex-col gap-5"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            const g = await post<GroupView>("/api/groups", { title, members: [...picked] });
            await qc.invalidateQueries({ queryKey: ["groups"] });
            close();
            onCreated(g.id);
          } catch (err) {
            setError((err as ApiError).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label="Group name" value={title} onChange={(e) => setTitle(e.target.value)} autoFocus placeholder="For example, Port contracts" maxLength={80} />
        <div>
          <div className="text-[13px] font-medium">People</div>
          {ready.length === 0 ? (
            <p className="mt-1.5 text-[13px] text-muted-foreground">Start a conversation with someone first; then you can add them to groups.</p>
          ) : (
            <ul className="scroll-pane mt-2 flex max-h-60 flex-col rounded-xl bg-muted p-1">
              {ready.map((c) => {
                const on = picked.has(c.id);
                return (
                  <li key={c.id}>
                    <label className={`group/pick flex h-11 cursor-pointer items-center gap-2.5 rounded-lg px-2 transition-colors duration-150 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring ${on ? "bg-[color-mix(in_srgb,var(--primary)_12%,transparent)]" : "hover:bg-pane"}`}>
                      <input
                        type="checkbox"
                        checked={on}
                        onChange={() =>
                          setPicked((p) => {
                            const n = new Set(p);
                            if (on) n.delete(c.id);
                            else n.add(c.id);
                            return n;
                          })
                        }
                        className="sr-only"
                      />
                      <Avatar seed={seedOf(c)} size={28} />
                      <span className="flex-1 truncate text-[13.5px]">{contactName(c)}</span>
                      <span
                        aria-hidden="true"
                        className={`grid size-5 shrink-0 place-items-center rounded-full transition-[background-color,transform] duration-150 ${on ? "scale-100 bg-primary text-primary-foreground" : "scale-90 bg-muted-strong text-transparent"}`}
                      >
                        <Check size={12} strokeWidth={3} />
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        {error && <Notice tone="danger">{error}</Notice>}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!title.trim() || picked.size === 0}>
            {picked.size > 0 ? `Create group with ${picked.size + 1}` : "Create group"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
