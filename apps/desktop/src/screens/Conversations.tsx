import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Archive,
  ArrowDown,
  BadgeCheck,
  Check,
  CheckCheck,
  ChevronLeft,
  Clock,
  Copy,
  CornerUpLeft,
  FileText,
  Info,
  KeyRound,
  Phone,
  Paperclip,
  Pencil,
  Pin,
  AtSign,
  Search,
  ShieldAlert,
  SmilePlus,
  Trash2,
  UserPlus,
  SquarePen,
  Users,
  X,
} from "lucide-react";
import { Fragment, type ReactNode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { del, get, patch, post } from "../api";
import { SpotConversations } from "../brand/spots";
import type { ChannelView, Contact, GroupView, Message, SecretView } from "../types";
import { useAppearance } from "../theme/appearance";
import { Wallpaper } from "../theme/Wallpaper";
import { Button, IconButton } from "../ui/Button";
import { SearchBar, useThreadSearch } from "../ui/ThreadSearch";
import { type Mentions, Rich, plain } from "../ui/Rich";
import { Avatar as AvatarMark, seedOf } from "../ui/Avatar";
import { SecretCard, SendSecretDialog, secretItem } from "./Secrets";
import { Chip, Empty, Kbd, Notice, Skeleton, Switch, bytes, ttlLabel } from "../ui/bits";
import { useToast } from "../ui/toast";
import { Album, InlinePhoto, PhotoViewer, type ViewerItem, isPhotoFile, withAlbums } from "../ui/Photo";
import { InlineVideo, type VideoItem, VideoViewer, VoicePlayer, isAudioFile, isVideoFile } from "../ui/Media";
import { Composer, type ComposerHandle, DraftLine, uploadFile } from "../ui/Composer";
import { AcceptDialog, DeleteDialog, InviteDialog, RenameDialog, SafetyDialog, contactName as name } from "./ConversationDialogs";
import { useCalls } from "./Calls";
import { BroadcastGlyph, ChannelRow, ChannelThread, NewChannelDialog } from "./Channels";
import { GroupRow, GroupThread, NewGroupDialog } from "./Groups";
import { useNav } from "./Shell";

const REACTIONS = ["👍", "❤️", "😂", "😮", "🙏", "✅"];
const GROUP_MS = 5 * 60 * 1000;
// Drafts live in memory for this session only: they may hold sensitive text, so they never touch storage.

type Filter = "all" | "unread" | "groups" | "channels" | "pinned" | "archived";

function useBelow(px: number) {
  const [below, setBelow] = useState(() => window.innerWidth < px);
  useEffect(() => {
    const on = () => setBelow(window.innerWidth < px);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, [px]);
  return below;
}

function listTime(sec: number) {
  if (!sec) return "";
  const d = new Date(sec * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const days = (now.getTime() - d.getTime()) / 86_400_000;
  if (days < 6) return d.toLocaleDateString(undefined, { weekday: "short" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

function Avatar({ c, size = 40 }: { c: Contact; size?: number }) {
  return (
    <AvatarMark seed={seedOf(c)} choice={c.avatar} size={size}>
      {c.verified && (
        <span className="absolute -right-0.5 -bottom-0.5 grid size-4 place-items-center rounded-full bg-pane">
          <BadgeCheck size={14} className="text-primary" aria-label="Verified" />
        </span>
      )}
    </AvatarMark>
  );
}

export function Conversations({ initial }: { initial?: string }) {
  const qc = useQueryClient();
  const narrow = useBelow(1000);
  const guarded = (useNav().status.settings?.risk_mode ?? "standard") !== "standard";
  const contacts = useQuery({ queryKey: ["contacts"], queryFn: () => get<Contact[]>("/api/contacts") });
  const [selected, setSelected] = useState<string | null>(null);
  const [dialog, setDialog] = useState<"invite" | "accept" | "group" | "channel" | null>(null);
  const [newMenu, setNewMenu] = useState<{ x: number; y: number } | null>(null);
  const groupsQ = useQuery({ queryKey: ["groups"], queryFn: () => get<GroupView[]>("/api/groups") });
  const channelsQ = useQuery({ queryKey: ["channels"], queryFn: () => get<ChannelView[]>("/api/channels") });
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [menu, setMenu] = useState<{ c: Contact; x: number; y: number } | null>(null);

  useEffect(() => {
    if (initial === "invite" || initial === "accept") setDialog(initial);
    else if (initial) setSelected(initial);
  }, [initial]);

  const all = contacts.data ?? [];
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return all
      .filter((c) => (filter === "archived" ? c.archived : !c.archived))
      .filter((c) => filter !== "unread" || c.unread > 0)
      .filter((c) => filter !== "pinned" || c.pinned)
      .filter((c) => !q || name(c).toLowerCase().includes(q) || c.preview.toLowerCase().includes(q))
      .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.preview_at - a.preview_at);
  }, [all, query, filter]);
  const current = all.find((c) => c.id === selected) ?? null;
  const groups = groupsQ.data ?? [];
  const shownGroups = useMemo(() => {
    const q = query.trim().toLowerCase();
    return groups
      .filter((g) => (filter === "archived" ? g.archived : !g.archived))
      .filter((g) => filter !== "unread" || g.unread > 0)
      .filter((g) => filter !== "pinned" || g.pinned)
      .filter((g) => !q || g.title.toLowerCase().includes(q) || g.preview.toLowerCase().includes(q));
  }, [groups, query, filter]);
  const currentGroup = groups.find((g) => g.id === selected) ?? null;
  const channels = channelsQ.data ?? [];
  const shownChannels = useMemo(() => {
    const q = query.trim().toLowerCase();
    return channels
      .filter((c) => (filter === "archived" ? c.archived : !c.archived))
      .filter((c) => filter !== "unread" || c.unread > 0)
      .filter((c) => filter !== "pinned" || c.pinned)
      .filter((c) => !q || c.title.toLowerCase().includes(q) || c.preview.toLowerCase().includes(q));
  }, [channels, query, filter]);
  const currentChannel = channels.find((c) => c.id === selected) ?? null;
  // One list: conversations, groups and channels together, newest first. "Groups" and
  // "Channels" narrow it to one kind.
  const items = useMemo(() => {
    const cs = filter === "groups" || filter === "channels" ? [] : shown.map((c) => ({ kind: "c" as const, id: c.id, pinned: c.pinned, at: c.preview_at, c }));
    const gs = filter === "channels" ? [] : shownGroups.map((g) => ({ kind: "g" as const, id: g.id, pinned: g.pinned, at: g.preview_at || g.last_activity, g }));
    const hs = filter === "groups" ? [] : shownChannels.map((h) => ({ kind: "h" as const, id: h.id, pinned: h.pinned, at: h.preview_at || h.last_activity, h }));
    return [...cs, ...gs, ...hs].sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.at - a.at);
  }, [shown, shownGroups, shownChannels, filter]);

  // Alt+Up / Alt+Down move between conversations.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
      e.preventDefault();
      const i = items.findIndex((c) => c.id === selected);
      const next = items[Math.max(0, Math.min(items.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) setSelected(next.id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [items, selected]);

  async function setFlag(c: Contact, body: Record<string, boolean>) {
    await patch(`/api/contacts/${c.id}`, body);
    await qc.invalidateQueries({ queryKey: ["contacts"] });
  }

  const open = current ?? currentGroup ?? currentChannel;
  const showList = !narrow || !open;
  const showThread = !narrow || !!open;
  const counts = {
    unread:
      all.filter((c) => c.unread > 0 && !c.archived).length + groups.filter((g) => g.unread > 0 && !g.archived).length + channels.filter((c) => c.unread > 0 && !c.archived).length,
    archived: all.filter((c) => c.archived).length + groups.filter((g) => g.archived).length + channels.filter((c) => c.archived).length,
  };

  return (
    <div className="flex h-full min-h-0">
      {showList && (
        <section className={`flex shrink-0 flex-col border-r border-line ${narrow ? "w-full" : "w-[340px]"}`}>
          <header className="flex h-16 shrink-0 items-center gap-2 px-4">
            <h1 className="flex-1 text-[17px] font-semibold tracking-tight">Conversations</h1>
            <IconButton
              label="New group or channel"
              aria-haspopup="menu"
              aria-expanded={!!newMenu}
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setNewMenu(newMenu ? null : { x: r.left, y: r.bottom + 6 });
              }}
            >
              <SquarePen size={17} />
            </IconButton>
            <IconButton label="Add someone by tag or link" onClick={() => setDialog("accept")}>
              <AtSign size={17} />
            </IconButton>
            <IconButton label="Invite someone" onClick={() => setDialog("invite")}>
              <UserPlus size={17} />
            </IconButton>
          </header>
          {all.length > 0 && (
            <div className="px-3 pb-2">
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search names and messages"
                aria-label="Search conversations"
                className="h-9 w-full rounded-xl bg-muted px-3.5 text-[13.5px] outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-primary/40"
              />
              <div className="no-scrollbar -mx-1 mt-2 flex gap-0.5 overflow-x-auto px-1" role="tablist" aria-label="Filter conversations">
                {(
                  [
                    ["all", "All"],
                    ["unread", counts.unread ? `Unread ${counts.unread}` : "Unread"],
                    ["groups", "Groups"],
                    ["channels", "Channels"],
                    ["pinned", "Pinned"],
                    ["archived", counts.archived ? `Archived ${counts.archived}` : "Archived"],
                  ] as [Filter, string][]
                ).map(([f, label]) => (
                  <button
                    key={f}
                    role="tab"
                    aria-selected={filter === f}
                    onClick={() => setFilter(f)}
                    className={`press h-7 shrink-0 cursor-pointer rounded-full px-2.5 text-[12.5px] font-medium whitespace-nowrap tabular ${filter === f ? "bg-foreground text-pane" : "text-muted-foreground hover:bg-muted hover:text-foreground"}`}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          )}
          <div className={`scroll-pane flex-1 px-2 pb-3 ${guarded ? "privacy-content" : ""}`}>
            {contacts.isPending && (
              <div className="flex flex-col gap-1 p-1">
                {[0, 1, 2, 3].map((i) => (
                  <div key={i} className="flex items-center gap-3 px-2 py-2">
                    <Skeleton className="size-11 rounded-full" />
                    <div className="flex flex-1 flex-col gap-1.5">
                      <Skeleton className="h-3.5 w-1/2" />
                      <Skeleton className="h-3 w-4/5" />
                    </div>
                  </div>
                ))}
              </div>
            )}
            {contacts.isError && (
              <div className="p-2">
                <Notice tone="danger">
                  Couldn't load conversations.{" "}
                  <button className="cursor-pointer underline" onClick={() => contacts.refetch()}>
                    Try again
                  </button>
                </Notice>
              </div>
            )}
            {all.length > 0 && items.length === 0 && (
              <div className="px-4 py-10 text-center text-[13px] text-muted-foreground">
                {query
                  ? `Nothing matches "${query}".`
                  : filter === "archived"
                    ? "No archived conversations."
                    : filter === "unread"
                      ? "You're all caught up."
                      : filter === "groups"
                        ? "No groups yet. Make one from the new button above."
                        : filter === "channels"
                          ? "No channels yet. Make one from the new button above, or wait for an invitation."
                        : "No pinned conversations yet. Pin one from its menu."}{" "}
                {(query || filter !== "all") && (
                  <button
                    className="cursor-pointer font-medium text-foreground underline underline-offset-4"
                    onClick={() => {
                      setQuery("");
                      setFilter("all");
                    }}
                  >
                    Show all
                  </button>
                )}
              </div>
            )}
            {items.map((it) =>
              it.kind === "c" ? (
                <ChatRow key={it.id} c={it.c} active={it.id === selected} onOpen={() => setSelected(it.id)} onMenu={(x, y) => setMenu({ c: it.c, x, y })} />
              ) : (
                it.kind === "g" ? (
                  <GroupRow key={it.id} g={it.g} active={it.id === selected} onOpen={() => setSelected(it.id)} />
                ) : (
                  <ChannelRow key={it.id} c={it.h} active={it.id === selected} onOpen={() => setSelected(it.id)} />
                )
              ),
            )}
          </div>
        </section>
      )}
      {showThread && (
        <section className="flex min-w-0 flex-1 flex-col">
          {current ? (
            <Thread key={current.id} contact={current} narrow={narrow} onBack={() => setSelected(null)} onDeleted={() => setSelected(null)} />
          ) : currentGroup ? (
            <GroupThread key={currentGroup.id} group={currentGroup} contacts={all} narrow={narrow} onBack={() => setSelected(null)} onGone={() => setSelected(null)} />
          ) : currentChannel ? (
            <ChannelThread
              key={currentChannel.id}
              channel={currentChannel}
              contacts={all}
              narrow={narrow}
              onBack={() => setSelected(null)}
              onGone={() => setSelected(null)}
              onOpenContact={(id) => setSelected(id)}
            />
          ) : all.length === 0 && !contacts.isPending ? (
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
            <div className="grid flex-1 place-items-center px-8">
              <Empty art={<SpotConversations />} title="Choose a conversation">
                <span className="block">Everything here is end-to-end encrypted and kept only on this computer.</span>
                <span className="mt-4 grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2 text-left text-[12.5px]">
                  <span className="justify-self-end"><Kbd>⌘K</Kbd></span>
                  <span>Jump to anyone or anything</span>
                  <span className="justify-self-end"><Kbd>Alt ↑↓</Kbd></span>
                  <span>Previous or next conversation</span>
                  <span className="justify-self-end"><Kbd>⌘I</Kbd></span>
                  <span>Conversation info and safety number</span>
                  <span className="justify-self-end"><Kbd>↑</Kbd></span>
                  <span>Edit your last message</span>
                </span>
              </Empty>
            </div>
          )}
        </section>
      )}
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <MenuItem icon={<Pin size={15} />} onClick={() => setFlag(menu.c, { pinned: !menu.c.pinned })}>
            {menu.c.pinned ? "Unpin" : "Pin to top"}
          </MenuItem>
          {menu.c.unread > 0 && (
            <MenuItem icon={<Check size={15} />} onClick={() => post(`/api/contacts/${menu.c.id}/read`).then(() => qc.invalidateQueries({ queryKey: ["contacts"] }))}>
              Mark as read
            </MenuItem>
          )}
          <MenuItem icon={<Archive size={15} />} onClick={() => setFlag(menu.c, { archived: !menu.c.archived })}>
            {menu.c.archived ? "Move out of archive" : "Archive"}
          </MenuItem>
        </ContextMenu>
      )}
      <InviteDialog open={dialog === "invite"} onClose={() => setDialog(null)} onCreated={(id) => setSelected(id)} />
      <AcceptDialog open={dialog === "accept"} onClose={() => setDialog(null)} onAccepted={(id) => setSelected(id)} />
      <NewGroupDialog open={dialog === "group"} contacts={all} onClose={() => setDialog(null)} onCreated={(id) => setSelected(id)} />
      <NewChannelDialog open={dialog === "channel"} contacts={all} onClose={() => setDialog(null)} onCreated={(id) => setSelected(id)} />
      {newMenu && (
        <ContextMenu x={newMenu.x} y={newMenu.y} onClose={() => setNewMenu(null)}>
          <MenuItem icon={<Users size={15} />} onClick={() => setDialog("group")}>
            New group
          </MenuItem>
          <MenuItem icon={<BroadcastGlyph size={15} />} onClick={() => setDialog("channel")}>
            New channel
          </MenuItem>
        </ContextMenu>
      )}
    </div>
  );
}

/** Previews show the words, not the formatting marks. */
function ChatRow({ c, active, onOpen, onMenu }: { c: Contact; active: boolean; onOpen: () => void; onMenu: (x: number, y: number) => void }) {
  const { density } = useAppearance();
  const preview = c.request_tag ? `Wants to connect (found you by @${c.request_tag})` : c.state === "invited" ? "Invite waiting for them to accept" : c.preview ? `${c.preview_outgoing ? "You: " : ""}${plain(c.preview)}` : "No messages yet";
  return (
    <div className="group relative">
      <button
        type="button"
        onClick={onOpen}
        onContextMenu={(e) => {
          e.preventDefault();
          onMenu(e.clientX, e.clientY);
        }}
        aria-current={active ? "true" : undefined}
        className={`flex w-full cursor-pointer items-center gap-3 rounded-xl px-2.5 py-[var(--row-py)] text-left transition-colors duration-150 ${active ? "bg-muted" : "hover:bg-muted/60"}`}
      >
        <Avatar c={c} size={density === "compact" ? 36 : 44} />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-1.5">
            <span className="truncate text-[14.5px] font-semibold">{name(c)}</span>
            {c.ttl_secs ? <Clock size={12} className="shrink-0 text-muted-foreground" aria-label={`Disappears after ${ttlLabel(c.ttl_secs)}`} /> : null}
            <span className={`ml-auto shrink-0 text-[12px] tabular transition-opacity duration-150 group-focus-within:opacity-0 group-hover:opacity-0 ${c.unread ? "font-medium text-primary" : "text-muted-foreground"}`}>{listTime(c.preview_at)}</span>
          </span>
          <span className="mt-0.5 flex items-center gap-2">
            <span className={`truncate text-[13px] ${c.unread ? "text-foreground" : "text-muted-foreground"}`}>
              <DraftLine draftKey={`c:${c.id}`} off={active}>
                {preview}
              </DraftLine>
            </span>
            <span className="ml-auto flex shrink-0 items-center gap-1.5">
              {c.pinned && !c.unread && <Pin size={12} className="text-muted-foreground" aria-label="Pinned" />}
              {c.unread > 0 && <span className="min-w-5 rounded-full bg-primary px-1.5 text-center text-[11px] leading-5 font-semibold text-primary-foreground tabular">{c.unread}</span>}
            </span>
          </span>
        </span>
      </button>
      <button
        type="button"
        aria-label={`More for ${name(c)}`}
        onClick={(e) => {
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          onMenu(r.left, r.bottom);
        }}
        className="press absolute top-1.5 right-2 grid size-7 cursor-pointer place-items-center rounded-full text-muted-foreground opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 hover:bg-pane hover:text-foreground focus-visible:opacity-100"
      >
        <span className="text-[15px] leading-none">⋯</span>
      </button>
    </div>
  );
}

// ---------- Thread ----------

type Group = { day: string; runs: Message[][] };

function groupMessages(ms: Message[]): Group[] {
  const out: Group[] = [];
  for (const m of ms) {
    const day = new Date(m.received_at).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    if (!out.length || out[out.length - 1].day !== day) out.push({ day, runs: [] });
    const g = out[out.length - 1];
    const run = g.runs[g.runs.length - 1];
    const last = run?.[run.length - 1];
    if (last && m.kind !== "system" && last.kind !== "system" && last.outgoing === m.outgoing && m.received_at - last.received_at < GROUP_MS) run.push(m);
    else g.runs.push([m]);
  }
  return out;
}

function Thread({ contact, narrow, onBack, onDeleted }: { contact: Contact; narrow: boolean; onBack: () => void; onDeleted: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const { go, status } = useNav();
  // The info panel docks beside the thread only when there is room for both.
  const floatInfo = useBelow(1360);
  const messages = useQuery({ queryKey: ["messages", contact.id], queryFn: () => get<Message[]>(`/api/contacts/${contact.id}/messages`) });
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const [editing, setEditing] = useState<Message | null>(null);
  const [dragging, setDragging] = useState(false);
  const [info, setInfo] = useState(false);
  const [dialog, setDialog] = useState<"safety" | "rename" | "delete" | null>(null);
  const [reviewing, setReviewing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ m: Message; x: number; y: number } | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [missed, setMissed] = useState(0);
  const [flash, setFlash] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const composer = useRef<ComposerHandle>(null);
  const firstUnread = useRef<string | null | undefined>(undefined);
  const ready = contact.state === "ready";
  const list = useMemo(() => messages.data ?? [], [messages.data]);
  const searchable = useMemo(() => list.filter((m) => m.kind !== "system"), [list]);
  const search = useThreadSearch(searchable, (m) => `${m.text ?? ""} ${m.file?.name ?? ""} ${m.file?.caption ?? ""}`, (m) => m.id, scroller, "data-mid");
  const byId = useMemo(() => new Map(list.map((m) => [m.id, m])), [list]);
  const secrets = useQuery({ queryKey: ["secrets", contact.id], queryFn: () => get<SecretView[]>(`/api/contacts/${contact.id}/secrets`) });
  const [sendingSecret, setSendingSecret] = useState(false);
  const [viewing, setViewing] = useState<string | null>(null);
  const [watching, setWatching] = useState<VideoItem | null>(null);
  // Every photo in the conversation, oldest first, for the viewer's next and previous.
  const photos: ViewerItem[] = useMemo(
    () =>
      list
        .filter((m) => m.kind === "file" && m.file && isPhotoFile(m.file.mime) && (m.outgoing || m.file.status === "ready"))
        .map((m) => ({ key: `m:file/${m.id}`, own: m.outgoing, name: m.file?.name ?? "Photo", caption: m.file?.caption, who: m.outgoing ? "You" : name(contact), at: m.received_at })),
    [list, contact],
  );
  // One-time secrets sit in the timeline in order with the messages.
  const timeline = useMemo(
    () => [...list, ...(secrets.data ?? []).map((x) => secretItem(contact.id, x))].sort((a, b) => a.received_at - b.received_at),
    [list, secrets.data, contact.id],
  );
  const groups = useMemo(() => groupMessages(timeline), [timeline]);

  // Remember where unread started when the conversation opened (the divider stays there).
  if (firstUnread.current === undefined && messages.data) {
    const unread = list.filter((m) => !m.outgoing && m.kind !== "system");
    firstUnread.current = contact.unread > 0 ? (unread[unread.length - contact.unread]?.id ?? null) : null;
  }

  useEffect(() => {
    if (contact.unread > 0) void post(`/api/contacts/${contact.id}/read`).then(() => qc.invalidateQueries({ queryKey: ["contacts"] }));
  }, [contact.id, contact.unread, qc]);

  // Stick to the bottom when already there; otherwise count what arrived below.
  const lastCount = useRef(0);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !messages.data) return;
    const added = timeline.length - lastCount.current;
    if (lastCount.current === 0) {
      const target = firstUnread.current && el.querySelector(`[data-mid="${firstUnread.current}"]`);
      if (target) (target as HTMLElement).scrollIntoView({ block: "center" });
      else el.scrollTop = el.scrollHeight;
      setAtBottom(el.scrollHeight - el.scrollTop - el.clientHeight < 80);
    } else if (atBottom || timeline[timeline.length - 1]?.outgoing) {
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    } else if (added > 0) {
      setMissed((n) => n + added);
    }
    lastCount.current = timeline.length;
  }, [timeline, messages.data, atBottom]);

  const jumpTo = useCallback((id: string) => {
    const el = scroller.current?.querySelector(`[data-mid="${id}"]`) as HTMLElement | null;
    if (!el) return;
    el.scrollIntoView({ block: "center", behavior: "smooth" });
    setFlash(id);
    setTimeout(() => setFlash(null), 1200);
  }, []);

  async function sendText(body: string) {
    try {
      if (editing) {
        await patch(`/api/contacts/${contact.id}/messages/${editing.id}`, { text: body });
        setEditing(null);
      } else {
        await post(`/api/contacts/${contact.id}/messages`, { text: body, reply_to: replyTo?.id ?? null });
        setReplyTo(null);
      }
    } finally {
      qc.invalidateQueries({ queryKey: ["messages", contact.id] });
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

  async function react(m: Message, emoji: string) {
    const mine = m.reactions.some((r) => r.emoji === emoji && r.mine);
    try {
      await post(`/api/contacts/${contact.id}/messages/${m.id}/react`, { emoji, on: !mine });
    } catch (e) {
      toast("error", (e as Error).message);
    }
  }

  function startEdit(m: Message) {
    setReplyTo(null);
    setEditing(m);
    // After the composer knows it's editing, so the edit doesn't overwrite your draft.
    requestAnimationFrame(() => {
      composer.current?.setText(m.text ?? "");
      composer.current?.focus();
    });
  }

  async function remove(m: Message, both: boolean) {
    await del(`/api/contacts/${contact.id}/messages/${m.id}${both ? "?both=1" : ""}`);
    qc.invalidateQueries({ queryKey: ["messages", contact.id] });
  }

  const privacy = (status.settings?.risk_mode ?? "standard") !== "standard";
  const look = useAppearance();
  const calls = useCalls();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "i") {
        e.preventDefault();
        setInfo((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="@container/thread relative flex min-h-0 flex-1">
      <div
        className="relative flex min-w-0 flex-1 flex-col"
        onDragOver={(e) => {
          if (!ready || !e.dataTransfer.types.includes("Files")) return;
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(e) => {
          if (e.currentTarget === e.target) setDragging(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          if (ready && e.dataTransfer.files.length) {
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
            <Avatar c={contact} size={36} />
            <span className="min-w-0">
              <span className="block truncate text-[15.5px] font-semibold tracking-tight">{name(contact)}</span>
              <span className="block truncate text-[12.5px] text-muted-foreground">
                {ready ? (contact.their_name && contact.their_name !== contact.nickname ? `Calls themselves ${contact.their_name}` : "End-to-end encrypted") : "Waiting for them to accept your invite"}
              </span>
            </span>
          </button>
          {ready &&
            !contact.verified &&
            (narrow ? (
              <IconButton label="Verify safety number" onClick={() => setDialog("safety")}>
                <ShieldAlert size={17} />
              </IconButton>
            ) : (
              <Button size="sm" icon={<ShieldAlert size={14} />} onClick={() => setDialog("safety")}>
                Verify
              </Button>
            ))}
          <IconButton label="Search this conversation" aria-pressed={search.open} onClick={() => (search.open ? search.close() : search.show())}>
            <Search size={17} />
          </IconButton>
          {ready && (
            <IconButton label="Call" onClick={() => void calls.call(contact)} disabled={calls.calls.some((c) => c.phase !== "ended")}>
              <Phone size={17} />
            </IconButton>
          )}
          <IconButton label="Conversation info" aria-pressed={info} onClick={() => setInfo((v) => !v)}>
            <Info size={17} />
          </IconButton>
        </header>
        {search.open && <SearchBar search={search} placeholder={`Search your conversation with ${name(contact)}`} />}
        <ProtectionStrip contact={contact} tor={status.profile?.transport.kind === "tor"} onVerify={() => setDialog("safety")} />
        {contact.request_tag && <RequestBanner contact={contact} onRemoved={onDeleted} />}
        <div className="relative flex min-h-0 flex-1 flex-col bg-[var(--chat-bg)]">
        <Wallpaper kind={look.wallpaper} strength={look.wallStrength} />
        <div
          ref={scroller}
          onScroll={(e) => {
            const el = e.currentTarget;
            const bottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
            setAtBottom(bottom);
            if (bottom) setMissed(0);
          }}
          className={`scroll-pane relative flex-1 overflow-x-hidden px-4 pt-2 pb-4 sm:px-8 ${privacy ? "privacy-content" : ""}`}
        >
          {messages.isPending && (
            <div className="flex flex-col gap-2 pt-6">
              <Skeleton className="h-10 w-2/5 rounded-2xl" />
              <Skeleton className="ml-auto h-10 w-1/3 rounded-2xl" />
              <Skeleton className="h-16 w-1/2 rounded-2xl" />
            </div>
          )}
          {!messages.isPending && list.length === 0 && (
            <div className="grid h-full place-items-center">
              <div className="max-w-sm text-center text-[13.5px] text-muted-foreground">
                {ready ? "No messages yet. Before you share anything sensitive, compare safety numbers in person or over a call you trust." : "Your invite is waiting. Once they use it, you can start writing."}
              </div>
            </div>
          )}
          {groups.map((g) => (
            <section key={g.day} className="relative">
              <div className="sticky top-1 z-[1] my-3 flex justify-center">
                <span className="rounded-full bg-pane/90 px-3 py-1 text-[12px] font-medium text-muted-foreground shadow-pane backdrop-blur">{g.day}</span>
              </div>
              <div className="flex flex-col gap-[var(--run-gap)]">
                {g.runs.map((run) => (
                  <div key={run[0].id} className={`flex flex-col gap-0.5 ${run[0].outgoing ? "items-end" : "items-start"}`}>
                    {withAlbums(run).map((it) =>
                      it.kind === "album" ? (
                        <Album
                          key={it.ms[0].id}
                          ms={it.ms}
                          keyOf={(m) => `m:file/${m.id}`}
                          mine={it.ms[0].outgoing}
                          meta={<AlbumMeta last={it.ms[it.ms.length - 1]} />}
                          onOpen={setViewing}
                          onMenu={(m, x, y) => setMenu({ m, x, y })}
                        />
                      ) : (
                        ((m: Message, i: number) => (
                      <Fragment key={m.id}>
                        {firstUnread.current === m.id && (
                          <div className="my-2 flex w-full items-center gap-3 text-[12px] font-medium text-primary">
                            <span className="h-px flex-1 bg-[color-mix(in_srgb,var(--primary)_35%,transparent)]" />
                            New messages
                            <span className="h-px flex-1 bg-[color-mix(in_srgb,var(--primary)_35%,transparent)]" />
                          </div>
                        )}
                        {m.kind === "secret" && m.secret ? (
                          <SecretCard contactId={contact.id} s={m.secret} name={name(contact)} />
                        ) : (
                        <Bubble
                          m={m}
                          first={i === 0}
                          last={i === run.length - 1}
                          reply={m.reply_to ? byId.get(m.reply_to) : undefined}
                          replyName={(r) => (r.outgoing ? "You" : name(contact))}
                          flash={flash === m.id || search.current === m.id}
                          reviewing={reviewing === m.id}
                          photoKey={`m:file/${m.id}`}
                          onOpenPhoto={() => setViewing(`m:file/${m.id}`)}
                          onOpenVideo={() => setWatching({ key: `m:file/${m.id}`, own: m.outgoing, name: m.file?.name ?? "Video", who: m.outgoing ? "You" : name(contact), at: m.received_at })}
                          onReply={() => {
                            setEditing(null);
                            setReplyTo(m);
                            composer.current?.focus();
                          }}
                          onReact={(e) => react(m, e)}
                          onEdit={() => startEdit(m)}
                          onCopy={() => navigator.clipboard.writeText(m.text ?? "").then(() => toast("info", "Copied. Clear your clipboard when you're done."))}
                          onMenu={(x, y) => setMenu({ m, x, y })}
                          onJump={jumpTo}
                          onReview={() => review(m)}
                          onOpenReview={() => go("review", `m:file/${m.id}`)}
                          onRetry={() => post(`/api/contacts/${contact.id}/messages/${m.id}/retry`).finally(() => qc.invalidateQueries({ queryKey: ["messages", contact.id] }))}
                        />
                        )}
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
        {!atBottom && (
          <button
            type="button"
            onClick={() => scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: "smooth" })}
            className="press absolute right-6 bottom-28 grid size-10 cursor-pointer place-items-center rounded-full bg-pane text-foreground shadow-float"
            aria-label={missed ? `${missed} new messages, jump to latest` : "Jump to latest"}
          >
            <ArrowDown size={18} />
            {missed > 0 && <span className="absolute -top-1.5 -right-1 min-w-5 rounded-full bg-primary px-1.5 text-center text-[11px] leading-5 font-semibold text-primary-foreground tabular">{missed}</span>}
          </button>
        )}
        <div className="relative shrink-0 px-4 pt-1 pb-4 sm:px-8">
          {ready ? (
            <Composer
              handle={composer}
              draftKey={`c:${contact.id}`}
              placeholder={`Message ${name(contact)}`}
              editing={!!editing}
              autoFocus
              banner={
                (replyTo || editing) && (
                  <div className="flex items-center gap-3 border-b border-line px-4 py-2">
                    {editing ? <Pencil size={15} className="text-primary" /> : <CornerUpLeft size={15} className="text-primary" />}
                    <div className="min-w-0 flex-1 border-l-2 border-primary pl-2.5">
                      <div className="text-[12px] font-semibold text-primary">{editing ? "Editing" : `Replying to ${replyTo!.outgoing ? "yourself" : name(contact)}`}</div>
                      <div className="truncate text-[12.5px] text-muted-foreground">{plain((editing ?? replyTo)!.text ?? (editing ?? replyTo)!.file?.name ?? "")}</div>
                    </div>
                    <IconButton
                      label="Cancel"
                      type="button"
                      onClick={() => {
                        setEditing(null);
                        setReplyTo(null);
                      }}
                    >
                      <X size={15} />
                    </IconButton>
                  </div>
                )
              }
              onSendText={sendText}
              onSendFile={(f, caption, photo) => uploadFile(`/api/contacts/${contact.id}/files`, f, caption, photo)}
              onEscape={() => {
                setEditing(null);
                setReplyTo(null);
              }}
              onArrowUpEmpty={() => {
                const mine = [...list].reverse().find((m) => m.outgoing && m.kind === "text");
                if (mine) startEdit(mine);
              }}
              extra={
                <IconButton label="Send a one-time secret" type="button" onClick={() => setSendingSecret(true)}>
                  <KeyRound size={17} />
                </IconButton>
              }
              commands={[
                { name: "secret", hint: "Send a one-time secret, like a password", run: () => setSendingSecret(true) },
                ...(calls.calls.some((c) => c.phase !== "ended") ? [] : [{ name: "call", hint: `Call ${name(contact)}`, run: () => void calls.call(contact) }]),
              ]}
              schedule={{ kind: "c", target: contact.id }}
              risk={status.settings?.risk_mode}
            />
          ) : (
            <Notice>Waiting for {name(contact)} to accept your invite. Send them the link or show the QR code again from the invite you created.</Notice>
          )}
        </div>
        </div>
        {dragging && (
          <div className="pointer-events-none absolute inset-3 grid place-items-center rounded-2xl bg-[color-mix(in_srgb,var(--primary)_10%,var(--pane))] shadow-[inset_0_0_0_2px_var(--primary)]">
            <div className="text-center">
              <Paperclip className="mx-auto text-primary" size={28} />
              <p className="mt-2 text-[15px] font-semibold">Drop to send to {name(contact)}</p>
              <p className="text-[13px] text-muted-foreground">It's encrypted on this computer before it goes anywhere.</p>
            </div>
          </div>
        )}
      </div>
      {watching && <VideoViewer item={watching} onClose={() => setWatching(null)} />}
      {viewing && photos.some((p) => p.key === viewing) && <PhotoViewer items={photos} start={photos.findIndex((p) => p.key === viewing)} onClose={() => setViewing(null)} />}
      <SendSecretDialog open={sendingSecret} contactId={contact.id} name={name(contact)} onClose={() => setSendingSecret(false)} />
      {info && (
        <InfoPanel
          contact={contact}
          messages={list}
          narrow={floatInfo}
          onClose={() => setInfo(false)}
          onSafety={() => setDialog("safety")}
          onRename={() => setDialog("rename")}
          onDelete={() => setDialog("delete")}
          onJump={(id) => {
            jumpTo(id);
            if (narrow) setInfo(false);
          }}
        />
      )}
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <div className="flex gap-0.5 px-1.5 pt-1 pb-1.5">
            {REACTIONS.map((e) => (
              <button key={e} onClick={() => react(menu.m, e).then(() => setMenu(null))} className="press grid size-8 cursor-pointer place-items-center rounded-lg text-[17px] hover:bg-muted" aria-label={`React ${e}`}>
                {e}
              </button>
            ))}
          </div>
          {menu.m.kind !== "system" && (
            <MenuItem icon={<CornerUpLeft size={15} />} onClick={() => setReplyTo(menu.m)}>
              Reply
            </MenuItem>
          )}
          {menu.m.outgoing && menu.m.kind === "text" && (
            <MenuItem icon={<Pencil size={15} />} onClick={() => startEdit(menu.m)}>
              Edit
            </MenuItem>
          )}
          {menu.m.text && (
            <MenuItem icon={<Copy size={15} />} onClick={() => navigator.clipboard.writeText(menu.m.text ?? "")}>
              Copy text
            </MenuItem>
          )}
          <MenuItem icon={<Trash2 size={15} />} onClick={() => remove(menu.m, false)}>
            Delete for me
          </MenuItem>
          <MenuItem icon={<Trash2 size={15} />} danger onClick={() => remove(menu.m, true)}>
            Delete for both
          </MenuItem>
        </ContextMenu>
      )}
      <SafetyDialog open={dialog === "safety"} onClose={() => setDialog(null)} contact={contact} />
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
    </div>
  );
}

function ProtectionStrip({ contact, tor, onVerify }: { contact: Contact; tor: boolean; onVerify: () => void }) {
  if (contact.state !== "ready") return <div className="h-px bg-line" />;
  const parts: ReactNode[] = [
    <span key="e2e">End-to-end encrypted</span>,
    contact.verified ? (
      <span key="v" className="inline-flex items-center gap-1 text-primary">
        <BadgeCheck size={13} /> Safety number checked
      </span>
    ) : (
      <button key="v" onClick={onVerify} className="cursor-pointer underline-offset-2 hover:text-foreground hover:underline">
        Not verified yet
      </button>
    ),
    tor ? <span key="t">Through Tor</span> : <span key="t" className="text-destructive">Not through Tor</span>,
    <span key="d">{contact.ttl_secs ? `Disappears after ${ttlLabel(contact.ttl_secs).toLowerCase()}` : "Messages kept until deleted"}</span>,
  ];
  return (
    <div className="no-scrollbar flex h-8 shrink-0 items-center gap-2 overflow-x-auto border-y border-line px-4 text-[12px] whitespace-nowrap text-muted-foreground sm:px-8" aria-label="How this conversation is protected">
      {parts.map((p, i) => (
        <Fragment key={i}>
          {i > 0 && <span aria-hidden="true">·</span>}
          {p}
        </Fragment>
      ))}
    </div>
  );
}

/** Someone reached you through your tag: keep talking, or remove them (they aren't told why). */
function RequestBanner({ contact, onRemoved }: { contact: Contact; onRemoved: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [busy, setBusy] = useState<"accept" | "remove" | null>(null);
  const act = async (kind: "accept" | "remove") => {
    setBusy(kind);
    try {
      if (kind === "accept") await post(`/api/tag/requests/${contact.id}/accept`);
      else await del(`/api/tag/requests/${contact.id}`);
      await qc.invalidateQueries({ queryKey: ["contacts"] });
      if (kind === "remove") onRemoved();
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="flex flex-wrap items-center gap-3 bg-[color-mix(in_srgb,var(--primary)_8%,var(--pane))] px-4 py-3 sm:px-8" role="region" aria-label="Contact request">
      <AtSign size={18} className="shrink-0 text-primary" aria-hidden="true" />
      <p className="min-w-0 flex-1 text-[13px] leading-relaxed">
        <span className="font-semibold">{name(contact)}</span> found you by your tag <span className="font-mono">@{contact.request_tag}</span>. Check who they are before you share anything, then keep the conversation or remove it.
      </p>
      <div className="flex gap-2">
        <Button size="sm" variant="danger-outline" loading={busy === "remove"} disabled={!!busy} onClick={() => void act("remove")}>
          Remove
        </Button>
        <Button size="sm" variant="primary" loading={busy === "accept"} disabled={!!busy} onClick={() => void act("accept")}>
          Accept
        </Button>
      </div>
    </div>
  );
}

function Ticks({ m }: { m: Message }) {
  if (m.delivery === "sending") return <Clock size={12} aria-label="Sending" />;
  if (m.delivery === "failed") return null;
  if (m.read) return <CheckCheck size={14} className="text-[var(--tick-read)]" aria-label="Read" />;
  if (m.delivery === "delivered") return <CheckCheck size={14} aria-label="Delivered" />;
  return <Check size={14} aria-label="Sent" />;
}

/** Time and ticks under an album, from its last photo. */
export function AlbumMeta({ last }: { last: Message }) {
  return (
    <>
      {last.expires_at && <Clock size={11} aria-label="Disappears" />}
      {new Date(last.received_at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
      {last.outgoing && <Ticks m={last} />}
    </>
  );
}

/** A file in a message: name, size, and Safe review for what you received. */
function FileCard(props: Parameters<typeof Bubble>[0]) {
  const { m } = props;
  const mine = m.outgoing;
  if (!m.file) return null;
  return (
    <div className="flex min-w-[250px] flex-col gap-2 pt-0.5 pb-4">
      <div className="flex items-center gap-2.5">
        <span className={`grid size-10 place-items-center rounded-xl ${mine ? "bg-current/10" : "bg-[var(--bubble-inset)]"}`}>
          <FileText size={19} />
        </span>
        <div className="min-w-0">
          <div className="truncate font-medium">{m.file.name}</div>
          <div className="text-[12px] opacity-70">
            {bytes(m.file.size)}
            {m.file.status === "downloading" && " · Receiving…"}
            {m.file.status === "failed" && " · Didn't arrive intact"}
          </div>
        </div>
      </div>
      {m.file.caption && <div className="whitespace-pre-wrap"><Rich text={m.file.caption} mentions={props.mentions} /></div>}
      {!mine && m.file.status === "ready" && m.file.review.state === "quarantined" && (
        <div className="flex items-center justify-between gap-3 rounded-xl bg-[var(--bubble-inset)] px-3 py-2">
          <span className="text-[12.5px] text-muted-foreground">Quarantined until reviewed</span>
          <Button size="sm" variant="primary" onClick={props.onReview} loading={props.reviewing}>
            {props.reviewing ? "Opening safely…" : "Review safely"}
          </Button>
        </div>
      )}
      {!mine && m.file.review.state === "clean" && (
        <Button size="sm" onClick={props.onOpenReview}>
          Open clean copy · {m.file.review.pages} page{m.file.review.pages === 1 ? "" : "s"}
        </Button>
      )}
      {!mine && m.file.review.state === "failed" && <span className="text-[12.5px] text-destructive">{m.file.review.reason}</span>}
    </div>
  );
}

export function Bubble(props: {
  /** Group members' names, so @mentions stand out (yours most). */
  mentions?: Mentions;
  /** Where this message's photo lives (a review key), when its file is a photo. */
  photoKey?: string;
  onOpenPhoto?: () => void;
  onOpenVideo?: () => void;
  m: Message;
  first: boolean;
  last: boolean;
  reply?: Message;
  replyName: (m: Message) => string;
  flash: boolean;
  reviewing: boolean;
  onReply: () => void;
  onReact: (emoji: string) => void;
  onEdit: () => void;
  onCopy: () => void;
  onMenu: (x: number, y: number) => void;
  onJump: (id: string) => void;
  onReview: () => void;
  onOpenReview: () => void;
  onRetry: () => void;
}) {
  const { m, first, last, reply } = props;
  const [picker, setPicker] = useState(false);
  if (m.kind === "system") {
    return (
      <div data-mid={m.id} className="my-1 self-center rounded-full bg-[var(--bubble-theirs)] px-3 py-1 text-[12px] shadow-[var(--bubble-theirs-shadow)] text-muted-foreground">
        {m.text}
      </div>
    );
  }
  const mine = m.outgoing;
  // Photos, videos and audio show as themselves once the file is here (or is yours).
  const shown = m.kind === "file" && !!m.file && !!props.photoKey && (mine || m.file.status === "ready");
  const mime = m.file?.mime ?? "";
  const mediaKind = !shown ? null : isPhotoFile(mime) ? "photo" : isVideoFile(mime) ? "video" : isAudioFile(mime) ? "audio" : null;
  const corner = mine ? (last ? "rounded-2xl rounded-br-md" : first ? "rounded-2xl rounded-br-md" : "rounded-2xl rounded-r-md") : last ? "rounded-2xl rounded-bl-md" : first ? "rounded-2xl rounded-bl-md" : "rounded-2xl rounded-l-md";
  const time = new Date(m.received_at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const meta = (
    <>
      {m.edited && <span>edited</span>}
      {m.expires_at && <Clock size={11} aria-label="Disappears" />}
      {time}
      {mine && <Ticks m={m} />}
    </>
  );
  return (
    <div
      data-mid={m.id}
      className={`group relative flex max-w-[min(86%,560px)] flex-col sm:max-w-[min(72%,560px)] ${mine ? "items-end" : "items-start"}`}
      onContextMenu={(e) => {
        e.preventDefault();
        props.onMenu(e.clientX, e.clientY);
      }}
    >
      <div
        className={`relative max-w-full min-w-0 px-[var(--bubble-px)] py-[var(--bubble-py)] text-[length:var(--msg-size)] leading-relaxed transition-[box-shadow] duration-300 ${corner} ${mine ? "bg-[var(--bubble-mine)] text-[var(--bubble-mine-fg)]" : "bg-[var(--bubble-theirs)] text-foreground shadow-[var(--bubble-theirs-shadow)]"} ${props.flash ? "shadow-[0_0_0_3px_var(--primary)]" : ""}`}
      >
        {reply && (
          <button
            type="button"
            onClick={() => props.onJump(reply.id)}
            className={`mb-1.5 block w-full max-w-full min-w-0 cursor-pointer rounded-lg border-l-2 px-2.5 py-1 text-left ${mine ? "border-current/50 bg-current/10" : "border-primary bg-[color-mix(in_srgb,var(--primary)_8%,transparent)]"}`}
          >
            <span className={`block text-[12px] font-semibold ${mine ? "" : "text-primary"}`}>{props.replyName(reply)}</span>
            <span className="block truncate text-[12.5px] opacity-80">{reply.text ? plain(reply.text) : (reply.file?.name ?? "Message")}</span>
          </button>
        )}
        {m.kind === "text" && (
          <div className="break-words whitespace-pre-wrap">
            <Rich text={m.text ?? ""} mentions={props.mentions} />
            <span className="invisible ml-2 inline-flex items-center gap-1 text-[11px]" aria-hidden="true">
              {meta}
            </span>
          </div>
        )}
        {mediaKind && m.file && props.photoKey && (
          <div className="flex flex-col gap-2 pt-0.5 pb-4">
            {mediaKind === "photo" && <InlinePhoto photoKey={props.photoKey} own={mine} name={m.file.name} onOpen={() => props.onOpenPhoto?.()} fallback={<FileCard {...props} />} />}
            {mediaKind === "video" && <InlineVideo mediaKey={props.photoKey} own={mine} name={m.file.name} onOpen={() => props.onOpenVideo?.()} fallback={<FileCard {...props} />} />}
            {mediaKind === "audio" && <VoicePlayer mediaKey={props.photoKey} own={mine} name={m.file.name} fallback={<FileCard {...props} />} />}
            {m.file.caption && <div className="max-w-[320px] whitespace-pre-wrap"><Rich text={m.file.caption} mentions={props.mentions} /></div>}
          </div>
        )}
        {m.kind === "file" && m.file && !mediaKind && <FileCard {...props} />}
        <span className={`absolute right-2.5 bottom-1.5 flex items-center gap-1 text-[11px] tabular ${mine ? "text-[var(--bubble-mine-muted)]" : "text-muted-foreground"}`}>
          {meta}
        </span>
      </div>
      {m.reactions.length > 0 && (
        <div className={`-mt-1.5 flex flex-wrap gap-1 px-2 ${mine ? "justify-end" : ""}`}>
          {m.reactions.map((r) => (
            <button
              key={r.emoji}
              onClick={() => props.onReact(r.emoji)}
              aria-pressed={r.mine}
              className={`press flex h-6 cursor-pointer items-center gap-1 rounded-full px-2 text-[13px] shadow-pane ${r.mine ? "bg-[color-mix(in_srgb,var(--primary)_16%,var(--pane))] text-primary" : "bg-pane"}`}
            >
              {r.emoji}
              {r.count > 1 && <span className="text-[11.5px] font-medium tabular">{r.count}</span>}
            </button>
          ))}
        </div>
      )}
      {mine && m.delivery === "failed" && (
        <button className="mt-0.5 cursor-pointer text-[12px] text-destructive hover:underline" onClick={props.onRetry}>
          Not sent. Retry
        </button>
      )}
      {/* Hover actions: one gesture to reply, react, edit or copy. */}
      <div
        className={`absolute top-1 hidden items-center gap-0.5 rounded-full bg-pane p-0.5 shadow-float group-focus-within:flex group-hover:flex ${mine ? "right-full mr-2" : "left-full ml-2"} ${picker ? "!flex" : ""}`}
        style={{ zIndex: "var(--z-dropdown)" }}
      >
        {picker ? (
          REACTIONS.map((e) => (
            <button
              key={e}
              onClick={() => {
                props.onReact(e);
                setPicker(false);
              }}
              className="press grid size-7 cursor-pointer place-items-center rounded-full text-[15px] hover:bg-muted"
              aria-label={`React ${e}`}
            >
              {e}
            </button>
          ))
        ) : (
          <>
            <IconButton label="React" className="size-7" onClick={() => setPicker(true)}>
              <SmilePlus size={15} />
            </IconButton>
            <IconButton label="Reply" className="size-7" onClick={props.onReply}>
              <CornerUpLeft size={15} />
            </IconButton>
            {mine && m.kind === "text" && (
              <IconButton label="Edit" className="size-7" onClick={props.onEdit}>
                <Pencil size={14} />
              </IconButton>
            )}
            {m.text && (
              <IconButton label="Copy" className="size-7" onClick={props.onCopy}>
                <Copy size={14} />
              </IconButton>
            )}
            <IconButton
              label="More"
              className="size-7"
              onClick={(e) => {
                const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                props.onMenu(r.left, r.bottom + 4);
              }}
            >
              <span className="text-[15px] leading-none">⋯</span>
            </IconButton>
          </>
        )}
      </div>
    </div>
  );
}

function InfoPanel(props: {
  contact: Contact;
  messages: Message[];
  narrow: boolean;
  onClose: () => void;
  onSafety: () => void;
  onRename: () => void;
  onDelete: () => void;
  onJump: (id: string) => void;
}) {
  const { contact: c } = props;
  const qc = useQueryClient();
  const toast = useToast();
  const files = props.messages.filter((m) => m.kind === "file" && m.file).reverse();
  const ready = c.state === "ready";
  async function set(body: Record<string, unknown>) {
    try {
      await patch(`/api/contacts/${c.id}`, body);
      await qc.invalidateQueries({ queryKey: ["contacts"] });
    } catch (e) {
      toast("error", (e as Error).message);
    }
  }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !document.querySelector("dialog[open]") && props.onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  return (
    <aside
      className={`scroll-pane flex w-[320px] max-w-full shrink-0 flex-col border-l border-line bg-pane ${props.narrow ? "absolute inset-y-0 right-0 z-10 border-l-0 shadow-float" : "@max-[760px]/thread:absolute @max-[760px]/thread:inset-y-0 @max-[760px]/thread:right-0 @max-[760px]/thread:z-10 @max-[760px]/thread:border-l-0 @max-[760px]/thread:shadow-float"}`}
      style={props.narrow ? { zIndex: "var(--z-sticky)" } : undefined}
      aria-label="Conversation info"
    >
      <div className="flex h-16 shrink-0 items-center justify-between px-4">
        <h2 className="text-[15px] font-semibold">Info</h2>
        <IconButton label="Close info" onClick={props.onClose}>
          <X size={16} />
        </IconButton>
      </div>
      <div className="flex flex-col items-center px-5 pb-5 text-center">
        <Avatar c={c} size={76} />
        <div className="mt-3 text-[17px] font-semibold tracking-tight">{name(c)}</div>
        {c.their_name && c.their_name !== c.nickname && <div className="text-[13px] text-muted-foreground">Calls themselves {c.their_name}</div>}
        <div className="mt-3 flex gap-2">
          <Button size="sm" icon={<Pencil size={13} />} onClick={props.onRename}>
            Rename
          </Button>
          <Button size="sm" icon={<Pin size={13} />} onClick={() => set({ pinned: !c.pinned })}>
            {c.pinned ? "Unpin" : "Pin"}
          </Button>
        </div>
      </div>
      {ready && (
        <section className="border-t border-line px-5 py-4">
          <div className="flex items-center justify-between">
            <h3 className="text-[13px] font-semibold">Safety number</h3>
            {c.verified ? <Chip tone="accent">Checked</Chip> : <Chip>Not checked</Chip>}
          </div>
          <div className="mt-2 grid grid-cols-4 gap-1 font-mono text-[12.5px] text-muted-foreground tabular">
            {(c.safety_number ?? "").split(" ").map((g, i) => (
              <span key={i}>{g}</span>
            ))}
          </div>
          <Button size="sm" className="mt-3 w-full" variant={c.verified ? "ghost" : "primary"} onClick={props.onSafety}>
            {c.verified ? "Compare again" : "Compare in person"}
          </Button>
        </section>
      )}
      {ready && (
        <section className="border-t border-line px-5 py-4">
          <h3 className="text-[13px] font-semibold">Disappearing messages</h3>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {(
              [
                [0, "Off"],
                [3600, "1 hour"],
                [86_400, "1 day"],
                [604_800, "7 days"],
                [2_592_000, "30 days"],
              ] as [number, string][]
            ).map(([s, label]) => (
              <button
                key={s}
                onClick={() => set({ timer: s })}
                aria-pressed={(c.ttl_secs ?? 0) === s}
                className={`press h-7 cursor-pointer rounded-full px-3 text-[12.5px] font-medium ${(c.ttl_secs ?? 0) === s ? "bg-foreground text-pane" : "bg-muted text-muted-foreground hover:text-foreground"}`}
              >
                {label}
              </button>
            ))}
          </div>
          <p className="mt-2 text-[12px] text-muted-foreground">Changes it for both of you. New messages are deleted from both devices after this time.</p>
        </section>
      )}
      {ready && (
        <section className="border-t border-line px-5 py-4">
          <label className="flex items-start justify-between gap-3">
            <span>
              <span className="block text-[13px] font-semibold">Send read receipts</span>
              <span className="mt-0.5 block text-[12px] text-muted-foreground">Tells them when you've read their messages, which also reveals when you're active. Off is safer for anyone at risk.</span>
            </span>
            <Switch checked={c.read_receipts} onChange={(v) => set({ read_receipts: v })} label="Send read receipts" />
          </label>
        </section>
      )}
      <section className="border-t border-line px-5 py-4">
        <h3 className="text-[13px] font-semibold">Files</h3>
        {files.length === 0 ? (
          <p className="mt-1.5 text-[12.5px] text-muted-foreground">No files in this conversation.</p>
        ) : (
          <ul className="mt-2 flex flex-col">
            {files.slice(0, 20).map((m) => (
              <li key={m.id}>
                <button onClick={() => props.onJump(m.id)} className="flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-1.5 py-1.5 text-left hover:bg-muted">
                  <FileText size={16} className="shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px]">{m.file!.name}</span>
                    <span className="block text-[11.5px] text-muted-foreground">
                      {bytes(m.file!.size)} · {m.outgoing ? "Sent" : m.file!.review.state === "clean" ? "Clean copy ready" : m.file!.review.state === "quarantined" ? "Quarantined" : m.file!.review.state}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
      <section className="mt-auto flex flex-col gap-1 border-t border-line px-3 py-3">
        <button onClick={() => set({ archived: !c.archived })} className="flex h-9 cursor-pointer items-center gap-2.5 rounded-lg px-2 text-[13.5px] hover:bg-muted">
          <Archive size={15} className="text-muted-foreground" />
          {c.archived ? "Move out of archive" : "Archive conversation"}
        </button>
        <button onClick={props.onDelete} className="flex h-9 cursor-pointer items-center gap-2.5 rounded-lg px-2 text-[13.5px] text-destructive hover:bg-[color-mix(in_srgb,var(--destructive)_8%,transparent)]">
          <Trash2 size={15} />
          Delete conversation
        </button>
      </section>
    </aside>
  );
}

// ---------- Menus ----------

export function ContextMenu({ x, y, onClose, children }: { x: number; y: number; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({ left: Math.min(x, window.innerWidth - r.width - 8), top: Math.min(y, window.innerHeight - r.height - 8) });
    el.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [x, y]);
  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      onClose();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);
  return (
    <div
      ref={ref}
      role="menu"
      className="fixed min-w-[200px] rounded-xl bg-pane p-1 shadow-float"
      style={{ left: pos.left, top: pos.top, zIndex: "var(--z-modal)", transformOrigin: "top left", animation: "menu-in 140ms var(--ease-smooth)" }}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("[role=menuitem]")) onClose();
      }}
    >
      {children}
    </div>
  );
}

export function MenuItem({ icon, children, onClick, danger }: { icon: ReactNode; children: ReactNode; onClick: () => void; danger?: boolean }) {
  return (
    <button
      role="menuitem"
      onClick={onClick}
      className={`flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13.5px] outline-none hover:bg-muted focus-visible:bg-muted ${danger ? "text-destructive" : ""}`}
    >
      <span className={danger ? "" : "text-muted-foreground"}>{icon}</span>
      {children}
    </button>
  );
}
