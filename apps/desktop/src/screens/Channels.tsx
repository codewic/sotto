import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BellOff, Check, ChevronLeft, Eye, FileText, Info, MessageSquare, Paperclip, Pencil, Search, SendHorizontal, Trash2, X } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { type ApiError, del, get, patch, post } from "../api";
import type { ChannelPost, ChannelReply, ChannelView, Contact } from "../types";
import { Button, IconButton } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { Rich, plain } from "../ui/Rich";
import { Avatar, seedOf } from "../ui/Avatar";
import { SearchBar, useThreadSearch } from "../ui/ThreadSearch";
import { Notice, Skeleton, Switch, bytes } from "../ui/bits";
import { useToast } from "../ui/toast";
import { Composer, type ComposerHandle, DraftLine, type SendAs, uploadFile } from "../ui/Composer";
import { InlinePhoto, PhotoViewer, type ViewerItem, isPhotoFile } from "../ui/Photo";
import { InlineVideo, type VideoItem, VideoViewer, VoicePlayer, isAudioFile, isVideoFile } from "../ui/Media";
import { contactName } from "./ConversationDialogs";
import { useNav } from "./Shell";
import { useMyAvatar } from "./AvatarPicker";

function initials(name: string) {
  const parts = name
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  return ((parts[0]?.[0] ?? "?") + (parts[1]?.[0] ?? "")).toUpperCase();
}

/** The channel glyph: a source dot sending two waves outward. Hand-built, two tones. */
export function BroadcastGlyph({ size = 12, className = "" }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 12 12" className={`broadcast-glyph shrink-0 ${className}`} aria-hidden="true">
      <circle cx="3" cy="6" r="1.6" fill="currentColor" />
      <path d="M6 3.2a4 4 0 0 1 0 5.6" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" className="wave-1" />
      <path d="M8.4 1.4a6.6 6.6 0 0 1 0 9.2" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" opacity="0.5" className="wave-2" />
    </svg>
  );
}

/** Channels share the group's rounded square, with the broadcast mark in the corner. */
export function ChannelAvatar({ title, size = 44, badge: showBadge = true }: { title: string; size?: number; badge?: boolean }) {
  const badge = Math.round(size * 0.42);
  return (
    <span className="relative grid shrink-0 place-items-center" style={{ width: size, height: size }} aria-hidden="true">
      <span
        className="grid size-full place-items-center rounded-[30%] bg-[color-mix(in_srgb,var(--primary)_14%,var(--muted-strong))] font-semibold text-primary"
        style={{ fontSize: size * 0.34 }}
      >
        {initials(title)}
      </span>
      {showBadge && (
        <span
          className="absolute -right-1 -bottom-1 grid place-items-center rounded-full bg-primary text-primary-foreground shadow-[0_0_0_2px_var(--pane)]"
          style={{ width: badge, height: badge }}
        >
          <BroadcastGlyph size={Math.round(badge * 0.62)} />
        </span>
      )}
    </span>
  );
}

function listTime(sec: number) {
  const d = new Date(sec * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

const clock = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

function followers(n: number) {
  return n === 1 ? "1 follower" : `${n} followers`;
}

export function ChannelRow({ c, active, onOpen }: { c: ChannelView; active: boolean; onOpen: () => void }) {
  const line = c.invited ? "Invitation to follow" : c.removed ? "You no longer follow this channel" : c.closed ? "This channel was closed" : plain(c.preview);
  return (
    <button
      type="button"
      onClick={onOpen}
      className={`flex w-full cursor-pointer items-center gap-3 rounded-xl px-2.5 py-[var(--row-py)] text-left transition-colors duration-150 ${active ? "bg-muted" : "hover:bg-muted/60"}`}
    >
      <ChannelAvatar title={c.title} />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-1.5">
          <span className="truncate text-[14.5px] font-semibold">{c.title}</span>
          <span className="sr-only">Channel</span>
          {c.muted && <BellOff size={12} className="shrink-0 text-muted-foreground" aria-label="Muted" />}
        </span>
        <span className="mt-0.5 flex items-center gap-2">
          <span className={`truncate text-[13px] ${c.invited ? "font-medium text-primary" : "text-muted-foreground"}`}>
            <DraftLine draftKey={`h:${c.id}`} off={active || !c.owner}>
              {line}
            </DraftLine>
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-1.5">
            <span className="text-[11.5px] text-muted-foreground tabular">{listTime(c.preview_at || c.last_activity)}</span>
            {c.unread > 0 && (
              <span className={`min-w-5 rounded-full px-1.5 text-center text-[11px] leading-5 font-semibold tabular ${c.muted ? "bg-muted-strong text-muted-foreground" : "bg-primary text-primary-foreground"}`}>{c.unread}</span>
            )}
          </span>
        </span>
      </span>
    </button>
  );
}

type Panel = null | { kind: "info" } | { kind: "replies"; post: ChannelPost };

export function ChannelThread({
  channel,
  contacts,
  narrow,
  onBack,
  onGone,
  onOpenContact,
}: {
  channel: ChannelView;
  contacts: Contact[];
  narrow: boolean;
  onBack: () => void;
  onGone: () => void;
  onOpenContact: (id: string) => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const posts = useQuery({ queryKey: ["channel-posts", channel.id], queryFn: () => get<ChannelPost[]>(`/api/channels/${channel.id}/posts`), enabled: !channel.invited });
  const [editing, setEditing] = useState<ChannelPost | null>(null);
  const [panel, setPanel] = useState<Panel>(null);
  const [deleting, setDeleting] = useState<ChannelPost | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const composer = useRef<ComposerHandle>(null);
  const list = useMemo(() => posts.data ?? [], [posts.data]);
  const owner = contacts.find((c) => c.id === channel.from);
  const ownerName = owner ? contactName(owner) : "the person who made it";
  const { go, status } = useNav();
  const [dragging, setDragging] = useState(false);
  const [reviewing, setReviewing] = useState<string | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [watching, setWatching] = useState<VideoItem | null>(null);
  const photos: ViewerItem[] = useMemo(
    () =>
      list
        .filter((p) => p.file && isPhotoFile(p.file.mime) && (channel.owner || p.file.status === "ready"))
        .map((p) => ({ key: `s:sblob/c/${channel.id}/${p.id}`, own: channel.owner, name: p.file?.name ?? "Photo", caption: p.file?.caption, who: channel.title, at: p.sent_at })),
    [list, channel.id, channel.owner, channel.title],
  );
  const search = useThreadSearch(list, (p) => `${p.body} ${p.file?.name ?? ""} ${p.file?.caption ?? ""}`, (p) => p.id, scroller, "data-pid");

  // Opening a channel marks it read; new posts while it's open do too.
  useEffect(() => {
    if (channel.unread > 0 && !channel.invited) void post(`/api/channels/${channel.id}/read`).then(() => qc.invalidateQueries({ queryKey: ["channels"] }));
  }, [channel.id, channel.unread, channel.invited, qc]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [list.length]);

  // The replies panel follows the post's live reply count.
  const openPost = panel?.kind === "replies" ? (list.find((p) => p.id === panel.post.id) ?? panel.post) : null;

  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["channel-posts", channel.id] });
    void qc.invalidateQueries({ queryKey: ["channels"] });
  };

  async function sendText(body: string) {
    if (editing) await patch(`/api/channels/${channel.id}/posts/${editing.id}`, { text: body });
    else await post(`/api/channels/${channel.id}/posts`, { text: body });
    setEditing(null);
    refresh();
  }

  async function sendFile(file: File, caption: string, sendAs: SendAs) {
    await uploadFile(`/api/channels/${channel.id}/files`, file, caption, sendAs);
    refresh();
  }

  async function review(p: ChannelPost) {
    setReviewing(p.id);
    try {
      const report = await post<{ key: string }>(`/api/channels/${channel.id}/posts/${p.id}/review`);
      go("review", report.key);
    } catch (e) {
      toast("error", (e as ApiError).message);
    } finally {
      setReviewing(null);
    }
  }

  const startEdit = (p: ChannelPost) => {
    setEditing(p);
    requestAnimationFrame(() => {
      composer.current?.setText(p.body);
      composer.current?.focus();
    });
  };

  const days = useMemo(() => {
    const out: { day: string; posts: ChannelPost[] }[] = [];
    for (const p of list) {
      const day = new Date(p.sent_at).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
      const last = out[out.length - 1];
      if (last?.day === day) last.posts.push(p);
      else out.push({ day, posts: [p] });
    }
    return out;
  }, [list]);

  const pending = (channel.subscribers ?? []).filter((x) => !x.joined).length;
  const sub = channel.owner
    ? `${followers(channel.subscriber_count ?? 0)}${pending ? ` · ${pending} invited` : ""} · only you see who follows`
    : channel.invited
      ? `Invitation from ${ownerName}`
      : `Channel from ${ownerName}`;

  return (
    <div className="@container/thread relative flex min-h-0 flex-1">
      <div
        className="relative flex min-w-0 flex-1 flex-col"
        onDragOver={(e) => {
          if (!channel.owner || !e.dataTransfer.types.includes("Files")) return;
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(e) => {
          if (e.currentTarget === e.target) setDragging(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          if (channel.owner && e.dataTransfer.files.length) {
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
          <button type="button" onClick={() => setPanel((p) => (p?.kind === "info" ? null : { kind: "info" }))} className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left">
            <ChannelAvatar title={channel.title} size={36} />
            <span className="min-w-0">
              <span className="block truncate text-[15.5px] font-semibold tracking-tight">{channel.title}</span>
              <span className="block truncate text-[12.5px] text-muted-foreground">{sub}</span>
            </span>
          </button>
          {!channel.invited && (
            <IconButton label="Search this channel" aria-pressed={search.open} onClick={() => (search.open ? search.close() : search.show())}>
              <Search size={17} />
            </IconButton>
          )}
          <IconButton label="Channel info" aria-pressed={panel?.kind === "info"} onClick={() => setPanel((p) => (p?.kind === "info" ? null : { kind: "info" }))}>
            <Info size={17} />
          </IconButton>
        </header>
        {search.open && !channel.invited && <SearchBar search={search} placeholder={`Search ${channel.title}`} />}
        <div className="h-px shrink-0 bg-line" />

        {channel.invited ? (
          <InviteCard channel={channel} ownerName={ownerName} onGone={onGone} />
        ) : (
          <>
            <div ref={scroller} className="scroll-pane relative flex-1 overflow-x-hidden px-4 pt-2 pb-4 sm:px-8">
              {posts.isPending && (
                <div className="mx-auto flex max-w-[640px] flex-col gap-3 pt-6" aria-label="Loading posts">
                  <Skeleton className="h-24 w-full rounded-2xl" />
                  <Skeleton className="h-16 w-full rounded-2xl" />
                </div>
              )}
              {!posts.isPending && list.length === 0 && <EmptyFeed channel={channel} />}
              {days.map((d) => (
                <section key={d.day} className="mx-auto max-w-[640px]">
                  <div className="sticky top-1 z-[1] my-3 flex justify-center">
                    <span className="rounded-full bg-pane/90 px-3 py-1 text-[12px] font-medium text-muted-foreground shadow-pane backdrop-blur">{d.day}</span>
                  </div>
                  <div className="flex flex-col gap-3">
                    {d.posts.map((p) => (
                      <PostCard
                        key={p.id}
                        p={p}
                        channel={channel}
                        open={openPost?.id === p.id || search.current === p.id}
                        reviewing={reviewing === p.id}
                        onReview={() => void review(p)}
                        onOpenReview={() => go("review", `s:sblob/c/${channel.id}/${p.id}`)}
                        onOpenPhoto={() => setViewing(`s:sblob/c/${channel.id}/${p.id}`)}
                        onOpenVideo={() => setWatching({ key: `s:sblob/c/${channel.id}/${p.id}`, own: channel.owner, name: p.file?.name ?? "Video", who: channel.title, at: p.sent_at })}
                        onReplies={() => setPanel({ kind: "replies", post: p })}
                        onEdit={() => startEdit(p)}
                        onDelete={() => setDeleting(p)}
                      />
                    ))}
                  </div>
                </section>
              ))}
            </div>
            <div className="relative shrink-0 px-4 pt-1 pb-4 sm:px-8">
              {channel.owner ? (
                <div className="mx-auto max-w-[640px]">
                  <Composer
                    handle={composer}
                    draftKey={`h:${channel.id}`}
                    placeholder={`Post to ${channel.title}`}
                    editing={!!editing}
                    banner={
                      editing && (
                        <div className="flex items-center gap-3 border-b border-line px-4 py-2">
                          <Pencil size={15} className="text-primary" />
                          <div className="min-w-0 flex-1 border-l-2 border-primary pl-2.5">
                            <div className="text-[12px] font-semibold text-primary">Editing your post</div>
                            <div className="truncate text-[12.5px] text-muted-foreground">{plain(editing.body)}</div>
                          </div>
                          <IconButton label="Cancel edit" type="button" onClick={() => setEditing(null)}>
                            <X size={15} />
                          </IconButton>
                        </div>
                      )
                    }
                    onSendText={sendText}
                    onSendFile={sendFile}
                    onEscape={() => setEditing(null)}
                    onArrowUpEmpty={() => {
                      const last = list[list.length - 1];
                      if (last) startEdit(last);
                    }}
                    schedule={{ kind: "h", target: channel.id }}
                    risk={status.settings?.risk_mode}
                  />
                </div>
              ) : channel.closed ? (
                <div className="mx-auto max-w-[640px]">
                  <Notice>
                    {channel.removed ? `${ownerName} took you off this channel.` : "This channel was closed."} You can still read the posts you have, or delete it from the channel info.
                  </Notice>
                </div>
              ) : (
                <p className="mx-auto max-w-[640px] text-center text-[12.5px] text-muted-foreground">
                  Only {ownerName} posts here. Replies go to them privately; other followers never see them or you.
                </p>
              )}
            </div>
          </>
        )}
      </div>
      {dragging && (
        <div className="pointer-events-none absolute inset-3 grid place-items-center rounded-2xl bg-[color-mix(in_srgb,var(--primary)_10%,var(--pane))] shadow-[inset_0_0_0_2px_var(--primary)]">
          <div className="text-center">
            <Paperclip className="mx-auto text-primary" size={28} />
            <p className="mt-2 text-[15px] font-semibold">Drop to post to {channel.title}</p>
            <p className="text-[13px] text-muted-foreground">Each follower gets it separately, and it arrives quarantined.</p>
          </div>
        </div>
      )}
      {panel?.kind === "info" && <ChannelInfo channel={channel} contacts={contacts} ownerName={ownerName} floating={narrow} onClose={() => setPanel(null)} onGone={onGone} onOpenContact={onOpenContact} />}
      {openPost && <Replies channel={channel} post={openPost} contacts={contacts} ownerName={ownerName} floating={narrow} onClose={() => setPanel(null)} onOpenContact={onOpenContact} />}
      {watching && <VideoViewer item={watching} onClose={() => setWatching(null)} />}
      {viewing && photos.some((x) => x.key === viewing) && <PhotoViewer items={photos} start={photos.findIndex((x) => x.key === viewing)} onClose={() => setViewing(null)} />}
      <Dialog open={!!deleting} onClose={() => setDeleting(null)} title="Delete this post for everyone?" description="It's removed from every follower's device, along with its replies.">
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDeleting(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={async () => {
              const p = deleting;
              setDeleting(null);
              if (!p) return;
              if (openPost?.id === p.id) setPanel(null);
              await del(`/api/channels/${channel.id}/posts/${p.id}`).then(refresh, (e: ApiError) => toast("error", e.message));
            }}
          >
            Delete post
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function EmptyFeed({ channel }: { channel: ChannelView }) {
  return (
    <div className="mx-auto flex max-w-[340px] flex-col items-center pt-16 text-center">
      <BroadcastSpot />
      <h2 className="mt-5 text-[15px] font-semibold">{channel.owner ? "Post your first update" : "No posts yet"}</h2>
      <p className="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
        {channel.owner
          ? "Followers get each post the moment you send it. Each one goes separately, so nobody learns who else follows."
          : `New posts from ${channel.title} appear here as soon as they're sent.`}
      </p>
    </div>
  );
}

/** Spot illustration: a source sending waves to three separate receivers that can't see each other. */
function BroadcastSpot() {
  return (
    <svg width="132" height="96" viewBox="0 0 132 96" className="broadcast-spot" aria-hidden="true">
      <rect x="8" y="30" width="36" height="36" rx="11" fill="color-mix(in srgb, var(--primary) 18%, var(--muted-strong))" />
      <circle cx="26" cy="48" r="6" fill="var(--primary)" />
      <path className="spot-wave-1" d="M50 36a16 16 0 0 1 0 24" fill="none" stroke="var(--primary)" strokeWidth="3" strokeLinecap="round" />
      <path className="spot-wave-2" d="M58 28a28 28 0 0 1 0 40" fill="none" stroke="var(--primary)" strokeWidth="3" strokeLinecap="round" opacity="0.5" />
      {[14, 48, 82].map((y) => (
        <g key={y}>
          <rect x="90" y={y - 8} width="32" height="16" rx="8" fill="var(--muted-strong)" />
          <circle cx="98" cy={y} r="4" fill="color-mix(in srgb, var(--primary) 40%, var(--muted-strong))" />
        </g>
      ))}
    </svg>
  );
}

function PostCard({
  p,
  channel,
  open,
  reviewing,
  onReplies,
  onEdit,
  onDelete,
  onReview,
  onOpenReview,
  onOpenPhoto,
  onOpenVideo,
}: {
  p: ChannelPost;
  channel: ChannelView;
  open: boolean;
  reviewing: boolean;
  onReplies: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onReview: () => void;
  onOpenReview: () => void;
  onOpenPhoto: () => void;
  onOpenVideo: () => void;
}) {
  const replyLabel = channel.owner
    ? p.replies === 0
      ? "No replies"
      : p.replies === 1
        ? "1 private reply"
        : `${p.replies} private replies`
    : p.replies > 0
      ? `You replied${p.replies > 1 ? ` ${p.replies} times` : ""}`
      : "Reply privately";
  const canReply = channel.owner ? p.replies > 0 : !channel.closed;
  const fileCard = p.file && (
        <div className="mt-2 flex flex-col gap-2">
          <div className="flex items-center gap-2.5 rounded-xl bg-pane/60 px-3 py-2.5">
            <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-[color-mix(in_srgb,var(--primary)_12%,transparent)] text-primary">
              <FileText size={19} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-[14px] font-medium">{p.file!.name}</div>
              <div className="text-[12px] text-muted-foreground">
                {bytes(p.file!.size)}
                {p.file!.status === "downloading" && " · Receiving…"}
                {p.file!.status === "failed" && " · Didn't arrive intact"}
              </div>
            </div>
            {!channel.owner && p.file!.status === "ready" && p.file!.review.state === "quarantined" && (
              <Button size="sm" variant="primary" onClick={onReview} loading={reviewing}>
                {reviewing ? "Opening safely…" : "Review safely"}
              </Button>
            )}
            {!channel.owner && p.file!.review.state === "clean" && (
              <Button size="sm" onClick={onOpenReview}>
                Open clean copy
              </Button>
            )}
          </div>
          {!channel.owner && p.file!.status === "ready" && p.file!.review.state === "quarantined" && (
            <p className="text-[12px] text-muted-foreground">Quarantined until reviewed. Safe review opens it in a sealed sandbox and rebuilds a clean copy.</p>
          )}
          {!channel.owner && p.file!.review.state === "failed" && <p className="text-[12.5px] text-destructive">{p.file!.review.reason}</p>}
          {p.file!.caption && <div className="text-[length:var(--msg-size)] leading-relaxed break-words whitespace-pre-wrap"><Rich text={p.file!.caption} /></div>}
        </div>
  );
  return (
    <article data-pid={p.id} className={`group/post rounded-2xl px-4 pt-3.5 pb-2.5 transition-colors duration-150 ${open ? "bg-[color-mix(in_srgb,var(--primary)_9%,var(--muted))]" : "bg-muted"}`}>
      <div className="flex items-center gap-2">
        <ChannelAvatar title={channel.title} size={22} badge={false} />
        <span className="truncate text-[13px] font-semibold">{channel.title}</span>
        <span className="ml-auto shrink-0 text-[11.5px] text-muted-foreground tabular">
          {p.edited && "edited "}
          {clock(p.sent_at)}
        </span>
      </div>
      {p.file ? (
        (isPhotoFile(p.file.mime) || isVideoFile(p.file.mime) || isAudioFile(p.file.mime)) && (channel.owner || p.file.status === "ready") ? (
          <div className="mt-2 flex flex-col gap-2">
            {isPhotoFile(p.file.mime) && <InlinePhoto photoKey={`s:sblob/c/${channel.id}/${p.id}`} own={channel.owner} name={p.file.name} onOpen={onOpenPhoto} fallback={fileCard} />}
            {isVideoFile(p.file.mime) && <InlineVideo mediaKey={`s:sblob/c/${channel.id}/${p.id}`} own={channel.owner} name={p.file.name} onOpen={onOpenVideo} fallback={fileCard} />}
            {isAudioFile(p.file.mime) && <VoicePlayer mediaKey={`s:sblob/c/${channel.id}/${p.id}`} own={channel.owner} name={p.file.name} fallback={fileCard} />}
            {p.file.caption && <div className="text-[length:var(--msg-size)] leading-relaxed break-words whitespace-pre-wrap"><Rich text={p.file.caption} /></div>}
          </div>
        ) : (
          fileCard
        )
      ) : (
        <div className="mt-2 text-[length:var(--msg-size)] leading-relaxed break-words whitespace-pre-wrap"><Rich text={p.body} /></div>
      )}
      <div className="mt-2 flex items-center gap-1">
        <button
          type="button"
          onClick={onReplies}
          disabled={!canReply}
          className="press flex h-7 cursor-pointer items-center gap-1.5 rounded-full px-2.5 text-[12.5px] font-medium text-primary hover:bg-[color-mix(in_srgb,var(--primary)_10%,transparent)] disabled:cursor-default disabled:text-muted-foreground disabled:hover:bg-transparent"
        >
          <MessageSquare size={13} />
          {replyLabel}
        </button>
        {channel.owner && (p.seen ?? 0) > 0 && (
          <span className="flex items-center gap-1 px-1.5 text-[12px] text-muted-foreground tabular" title="Counts only followers who chose to share read receipts">
            <Eye size={13} aria-hidden="true" />
            Seen by {p.seen}
          </span>
        )}
        {channel.owner && (
          <span className="ml-auto flex items-center gap-0.5 opacity-0 transition-opacity duration-150 group-focus-within/post:opacity-100 group-hover/post:opacity-100">
            <IconButton label="Edit post" onClick={onEdit}>
              <Pencil size={14} />
            </IconButton>
            <IconButton label="Delete post" onClick={onDelete}>
              <Trash2 size={14} />
            </IconButton>
          </span>
        )}
      </div>
    </article>
  );
}

function InviteCard({ channel, ownerName, onGone }: { channel: ChannelView; ownerName: string; onGone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [busy, setBusy] = useState<"join" | "decline" | null>(null);
  const act = async (kind: "join" | "decline") => {
    setBusy(kind);
    try {
      if (kind === "join") await post(`/api/channels/${channel.id}/join`);
      else await del(`/api/channels/${channel.id}`);
      await qc.invalidateQueries({ queryKey: ["channels"] });
      if (kind === "decline") onGone();
    } catch (e) {
      toast("error", (e as ApiError).message);
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="scroll-pane flex flex-1 items-center justify-center p-6">
      <div className="flex w-full max-w-[380px] flex-col items-center text-center">
        <ChannelAvatar title={channel.title} size={76} />
        <h2 className="mt-4 text-[18px] font-semibold tracking-tight text-balance">{channel.title}</h2>
        {channel.about && <p className="mt-1.5 text-[13.5px] leading-relaxed text-muted-foreground">{channel.about}</p>}
        <p className="mt-4 text-[13px] text-muted-foreground">{ownerName} invited you to follow this channel.</p>
        <ul className="mt-4 flex w-full flex-col gap-2 rounded-2xl bg-muted p-4 text-left text-[13px] leading-relaxed">
          <li className="flex gap-2.5">
            <BroadcastGlyph size={14} className="mt-[3px] text-primary" />
            Only {ownerName} posts. Posts arrive end-to-end encrypted, like your messages.
          </li>
          <li className="flex gap-2.5">
            <BroadcastGlyph size={14} className="mt-[3px] text-primary" />
            Only {ownerName} knows you follow. Other followers never see you, and you never see them.
          </li>
          <li className="flex gap-2.5">
            <BroadcastGlyph size={14} className="mt-[3px] text-primary" />
            Replies go to {ownerName} alone. You can stop following any time.
          </li>
        </ul>
        <div className="mt-5 flex w-full gap-2">
          <Button className="flex-1" disabled={!!busy} loading={busy === "decline"} onClick={() => void act("decline")}>
            Not now
          </Button>
          <Button autoFocus className="flex-1" variant="primary" disabled={!!busy} loading={busy === "join"} onClick={() => void act("join")}>
            Follow channel
          </Button>
        </div>
      </div>
    </div>
  );
}

const asideClass = (floating: boolean) =>
  `scroll-pane flex w-[320px] max-w-full shrink-0 flex-col border-l border-line bg-pane ${floating ? "absolute inset-y-0 right-0 z-10 border-l-0 shadow-float" : "@max-[760px]/thread:absolute @max-[760px]/thread:inset-y-0 @max-[760px]/thread:right-0 @max-[760px]/thread:z-10 @max-[760px]/thread:border-l-0 @max-[760px]/thread:shadow-float"}`;

function Replies({
  channel,
  post: p,
  contacts,
  ownerName,
  floating,
  onClose,
  onOpenContact,
}: {
  channel: ChannelView;
  post: ChannelPost;
  contacts: Contact[];
  ownerName: string;
  floating: boolean;
  onClose: () => void;
  onOpenContact: (id: string) => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const replies = useQuery({ queryKey: ["channel-replies", channel.id, p.id], queryFn: () => get<ChannelReply[]>(`/api/channels/${channel.id}/posts/${p.id}/replies`) });
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const list = replies.data ?? [];
  const nameOf = (id: string | null) => {
    const c = contacts.find((x) => x.id === id);
    return c ? contactName(c) : "A follower";
  };
  const me = useMyAvatar();
  const seedFor = (id: string | null) => {
    const c = contacts.find((x) => x.id === id);
    return c ? seedOf(c) : `contact:${id}`;
  };
  async function send() {
    const body = text.trim();
    if (!body || busy) return;
    setBusy(true);
    try {
      await post(`/api/channels/${channel.id}/posts/${p.id}/replies`, { text: body });
      setText("");
      await qc.invalidateQueries({ queryKey: ["channel-replies", channel.id] });
      await qc.invalidateQueries({ queryKey: ["channel-posts", channel.id] });
    } catch (e) {
      toast("error", (e as ApiError).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <aside className={asideClass(floating)} aria-label="Private replies">
      <div className="flex h-16 shrink-0 items-center justify-between px-4">
        <span className="text-[14.5px] font-semibold">{channel.owner ? "Private replies" : "Your private replies"}</span>
        <IconButton label="Close replies" onClick={onClose}>
          <X size={17} />
        </IconButton>
      </div>
      <div className="mx-4 mb-3 rounded-xl bg-muted px-3 py-2.5">
        <div className="text-[11.5px] font-semibold text-muted-foreground">Post · {clock(p.sent_at)}</div>
        <p className="mt-0.5 line-clamp-3 text-[13px] leading-relaxed">{p.body}</p>
      </div>
      <div className="flex flex-1 flex-col gap-3 px-4 pb-4">
        {replies.isPending && <Skeleton className="h-14 w-full rounded-xl" />}
        {!replies.isPending && list.length === 0 && (
          <p className="pt-6 text-center text-[13px] leading-relaxed text-muted-foreground">
            {channel.owner ? "No one has replied to this post." : `Write to ${ownerName} about this post. Only they see it.`}
          </p>
        )}
        {list.map((r) => (
          <div key={r.id} className="flex gap-2.5">
            <Avatar seed={r.mine ? me.seed : seedFor(r.contact_id)} choice={r.mine ? me.choice : contacts.find((x) => x.id === r.contact_id)?.avatar} size={28} />
            <div className="min-w-0 flex-1">
              <div className="flex items-baseline gap-2">
                <span className="truncate text-[13px] font-semibold">{r.mine ? "You" : nameOf(r.contact_id)}</span>
                <span className="text-[11.5px] text-muted-foreground tabular">{clock(r.sent_at)}</span>
              </div>
              <p className="mt-0.5 text-[13.5px] leading-relaxed break-words whitespace-pre-wrap">{r.body}</p>
              {channel.owner && r.contact_id && (
                <button type="button" onClick={() => onOpenContact(r.contact_id as string)} className="mt-1 cursor-pointer text-[12.5px] font-medium text-primary hover:underline">
                  Answer in your conversation
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
      {!channel.owner && !channel.closed && (
        <form
          className="sticky bottom-0 bg-pane px-4 pt-2 pb-4"
          onSubmit={(e) => {
            e.preventDefault();
            void send();
          }}
        >
          <div className="flex items-end gap-1.5 rounded-2xl bg-[var(--composer)] p-2 shadow-[var(--composer-shadow)]">
            <textarea
              autoFocus
              value={text}
              rows={2}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                }
              }}
              placeholder={`Reply to ${ownerName}`}
              aria-label="Private reply"
              className="max-h-40 min-w-0 flex-1 resize-none bg-transparent px-1.5 py-1.5 text-[13.5px] leading-relaxed outline-none placeholder:truncate placeholder:text-muted-foreground"
            />
            <button
              type="submit"
              disabled={busy || !text.trim()}
              aria-label="Send private reply"
              className="press grid size-8 shrink-0 cursor-pointer place-items-center rounded-full bg-primary text-primary-foreground disabled:cursor-not-allowed disabled:opacity-40"
            >
              <SendHorizontal size={15} />
            </button>
          </div>
          <p className="mt-1.5 text-center text-[11.5px] text-muted-foreground">Only {ownerName} sees this.</p>
        </form>
      )}
    </aside>
  );
}

function ChannelInfo({
  channel,
  contacts,
  ownerName,
  floating,
  onClose,
  onGone,
  onOpenContact,
}: {
  channel: ChannelView;
  contacts: Contact[];
  ownerName: string;
  floating: boolean;
  onClose: () => void;
  onGone: () => void;
  onOpenContact: (id: string) => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const [adding, setAdding] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [title, setTitle] = useState(channel.title);
  const [about, setAbout] = useState(channel.about);
  const run = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      await qc.invalidateQueries({ queryKey: ["channels"] });
    } catch (e) {
      toast("error", (e as ApiError).message);
    }
  };
  const subs = channel.subscribers ?? [];
  const addable = contacts.filter((c) => c.state === "ready" && !subs.some((x) => x.contact_id === c.id));
  const save = () => {
    const t = title.trim();
    if (!t) return setTitle(channel.title);
    if (t !== channel.title || about.trim() !== channel.about) void run(() => patch(`/api/channels/${channel.id}`, { title: t, about }));
  };
  const nameOf = (id: string) => {
    const c = contacts.find((x) => x.id === id);
    return c ? contactName(c) : "Someone you removed";
  };
  const seedFor = (id: string) => {
    const c = contacts.find((x) => x.id === id);
    return c ? seedOf(c) : `contact:${id}`;
  };
  return (
    <aside className={asideClass(floating)} aria-label="Channel info">
      <div className="flex h-16 shrink-0 items-center justify-between px-4">
        <span className="text-[14.5px] font-semibold">Channel info</span>
        <IconButton label="Close channel info" onClick={onClose}>
          <X size={17} />
        </IconButton>
      </div>
      <div className="flex flex-col items-center px-5 pb-5 text-center">
        <ChannelAvatar title={channel.title} size={76} />
        {channel.owner ? (
          <>
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onBlur={save}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
              aria-label="Channel name"
              maxLength={80}
              className="mt-3 w-full rounded-lg bg-transparent px-2 py-1 text-center text-[17px] font-semibold outline-none hover:bg-muted focus:bg-muted"
            />
            <textarea
              value={about}
              onChange={(e) => setAbout(e.target.value)}
              onBlur={save}
              rows={2}
              maxLength={300}
              placeholder="Add a description"
              aria-label="Channel description"
              className="mt-1 w-full resize-none rounded-lg bg-transparent px-2 py-1 text-center text-[13px] leading-relaxed text-muted-foreground outline-none placeholder:text-muted-foreground/70 hover:bg-muted focus:bg-muted"
            />
          </>
        ) : (
          <>
            <div className="mt-3 text-[17px] font-semibold">{channel.title}</div>
            {channel.about && <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">{channel.about}</p>}
          </>
        )}
        <div className="mt-1 text-[12.5px] text-muted-foreground">{channel.owner ? `Your channel · ${followers(channel.subscriber_count ?? 0)}` : `Channel from ${ownerName}`}</div>
      </div>

      {channel.owner ? (
        <section className="border-t border-line px-5 py-4">
          <div className="flex items-center justify-between">
            <h3 className="text-[13px] font-semibold">Followers</h3>
            {addable.length > 0 && (
              <button type="button" onClick={() => setAdding((v) => !v)} className="cursor-pointer text-[12.5px] font-medium text-primary hover:underline">
                {adding ? "Done" : "Invite people"}
              </button>
            )}
          </div>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">Only you see this list. Followers never see each other or how many follow.</p>
          {subs.length === 0 && !adding && <p className="mt-3 text-[13px] text-muted-foreground">No one yet. Invite people from your conversations.</p>}
          <ul className="mt-2 flex flex-col">
            {subs.map((x) => (
              <li key={x.contact_id} className="flex h-10 items-center gap-2.5">
                <Avatar seed={seedFor(x.contact_id)} size={28} />
                <button type="button" onClick={() => onOpenContact(x.contact_id)} className="min-w-0 flex-1 cursor-pointer truncate text-left text-[13.5px] hover:underline">
                  {nameOf(x.contact_id)}
                </button>
                {!x.joined && <span className="text-[11.5px] text-muted-foreground">Invited</span>}
                <button
                  type="button"
                  onClick={() => void run(() => del(`/api/channels/${channel.id}/subscribers/${x.contact_id}`))}
                  className="cursor-pointer rounded-md px-1.5 py-0.5 text-[12px] text-muted-foreground hover:bg-muted hover:text-destructive"
                  aria-label={`${x.joined ? "Remove" : "Withdraw the invitation for"} ${nameOf(x.contact_id)}`}
                >
                  {x.joined ? "Remove" : "Withdraw"}
                </button>
              </li>
            ))}
          </ul>
          {adding && (
            <ul className="mt-2 flex flex-col rounded-xl bg-muted p-1">
              {addable.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    onClick={() => void run(() => post(`/api/channels/${channel.id}/subscribers`, { contact_ids: [c.id] }))}
                    className="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2 text-left text-[13px] hover:bg-pane"
                  >
                    <Avatar seed={seedOf(c)} size={24} />
                    <span className="flex-1 truncate">{contactName(c)}</span>
                    <span className="text-[12px] text-primary">Invite</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : (
        <section className="border-t border-line px-5 py-4 text-[13px] leading-relaxed text-muted-foreground">
          Only {ownerName} knows you follow this channel. Other followers can't see you, and you can't see them.
          {channel.from && (
            <button type="button" onClick={() => onOpenContact(channel.from as string)} className="mt-2 block cursor-pointer font-medium text-primary hover:underline">
              Open your conversation with {ownerName}
            </button>
          )}
        </section>
      )}

      <section className="flex flex-col gap-4 border-t border-line px-5 py-4">
        <div className="flex items-center gap-3">
          <div className="min-w-0 flex-1">
            <h3 className="text-[13px] font-semibold">Mute</h3>
            <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">Unread counts stay grey and don't add to the badge.</p>
          </div>
          <Switch checked={channel.muted} label="Mute this channel" onChange={(v) => void run(() => patch(`/api/channels/${channel.id}`, { muted: v }))} />
        </div>
        {!channel.owner && !channel.invited && !channel.closed && (
          <div className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              <h3 className="text-[13px] font-semibold">Read receipts</h3>
              <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">Let {ownerName} see when you've read posts. Off unless you turn it on, and only {ownerName} is told.</p>
            </div>
            <Switch checked={channel.receipts} label="Share read receipts" onChange={(v) => void run(() => patch(`/api/channels/${channel.id}`, { receipts: v }))} />
          </div>
        )}
      </section>
      <section className="mt-auto border-t border-line px-3 py-3">
        <button
          type="button"
          onClick={() => setConfirm(true)}
          className="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2 text-[13.5px] text-destructive hover:bg-[color-mix(in_srgb,var(--destructive)_8%,transparent)]"
        >
          {channel.owner ? "Close channel" : channel.closed ? "Delete channel" : "Stop following"}
        </button>
      </section>
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title={channel.owner ? `Close ${channel.title}?` : channel.closed ? `Delete ${channel.title}?` : `Stop following ${channel.title}?`}
        description={
          channel.owner
            ? "Followers are told it closed and keep the posts they have. Posts and replies are deleted from this computer."
            : channel.closed
              ? "Its posts are deleted from this computer."
              : `${ownerName} is told you stopped following, and the posts are deleted from this computer.`
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
              await run(() => del(`/api/channels/${channel.id}`));
              onGone();
            }}
          >
            {channel.owner ? "Close channel" : channel.closed ? "Delete channel" : "Stop following"}
          </Button>
        </div>
      </Dialog>
    </aside>
  );
}

export function NewChannelDialog({ open, contacts, onClose, onCreated }: { open: boolean; contacts: Contact[]; onClose: () => void; onCreated: (id: string) => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState("");
  const [about, setAbout] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ready = contacts.filter((c) => c.state === "ready");
  const close = () => {
    setTitle("");
    setAbout("");
    setPicked(new Set());
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title="New channel" description="Post updates to people who choose to follow. Followers never see each other, and replies come only to you." width={460}>
      <form
        className="flex flex-col gap-5"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            const c = await post<ChannelView>("/api/channels", { title, about, invite: [...picked] });
            await qc.invalidateQueries({ queryKey: ["channels"] });
            close();
            onCreated(c.id);
          } catch (err) {
            setError((err as ApiError).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label="Channel name" value={title} onChange={(e) => setTitle(e.target.value)} autoFocus placeholder="For example, Safety alerts" maxLength={80} />
        <Field label="Description" hint="Optional. Shown on the invitation." value={about} onChange={(e) => setAbout(e.target.value)} placeholder="What followers can expect" maxLength={300} />
        <div>
          <div className="text-[13px] font-medium">Invite</div>
          {ready.length === 0 ? (
            <p className="mt-1.5 text-[13px] text-muted-foreground">Start a conversation with someone first; then you can invite them. You can also invite people later.</p>
          ) : (
            <ul className="scroll-pane mt-2 flex max-h-56 flex-col rounded-xl bg-muted p-1">
              {ready.map((c) => {
                const on = picked.has(c.id);
                return (
                  <li key={c.id}>
                    <label
                      className={`flex h-11 cursor-pointer items-center gap-2.5 rounded-lg px-2 transition-colors duration-150 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring ${on ? "bg-[color-mix(in_srgb,var(--primary)_12%,transparent)]" : "hover:bg-pane"}`}
                    >
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
          <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">Each person gets an invitation in Sotto and decides whether to follow.</p>
        </div>
        {error && <Notice tone="danger">{error}</Notice>}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!title.trim()}>
            {picked.size > 0 ? `Create and invite ${picked.size}` : "Create channel"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
