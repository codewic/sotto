export type RiskMode = "standard" | "high" | "extreme";

export type Status = {
  state: "new" | "locked" | "unlocked";
  version: string;
  profile?: {
    display_name: string;
    default_relay: string;
    transport: { kind: "tor"; socks: string } | { kind: "direct" };
    fingerprint: string;
  };
  settings?: Settings;
  relays?: { url: string; status: "connecting" | "online" | "offline" }[];
  sandbox?: boolean;
};

export type Settings = {
  risk_mode: RiskMode;
  auto_lock_minutes: number;
  delete_after_download: boolean;
};

export type Contact = {
  id: string;
  nickname: string;
  their_name: string;
  state: "invited" | "ready";
  verified: boolean;
  ttl_secs: number | null;
  unread: number;
  created_at: number;
  last_activity: number;
  listed_identity: boolean;
  safety_number: string | null;
  their_fingerprint: string | null;
  /** They reached you through this tag and you haven't accepted them yet. */
  request_tag?: string | null;
  /** Their pick from the avatar collection (`v1:shape:hue:turn`), if any. */
  avatar?: string | null;
  invite_expires: number | null;
  pinned: boolean;
  archived: boolean;
  read_receipts: boolean;
  preview: string;
  preview_at: number;
  preview_outgoing: boolean;
};

export type ReviewState = { state: "own" | "quarantined" | "clean" | "failed"; pages: number | null; reason: string | null };

export type FileView = {
  name: string;
  mime: string;
  size: number;
  status: "uploading" | "sent" | "downloading" | "ready" | "failed";
  review: ReviewState;
  caption: string;
};

export type Message = {
  id: string;
  contact_id: string;
  outgoing: boolean;
  kind: "text" | "file" | "system" | "secret";
  text: string | null;
  file: FileView | null;
  sent_at: number;
  received_at: number;
  expires_at: number | null;
  delivery: "sending" | "sent" | "delivered" | "failed" | "received";
  reply_to: string | null;
  reactions: { emoji: string; count: number; mine: boolean }[];
  edited: boolean;
  read: boolean;
  /** Group messages: who wrote it. */
  author?: string;
  /** 1:1 timeline only: a one-time secret shown in place. */
  secret?: SecretView;
};

export type GroupMember = { name: string; owner: boolean; admin: boolean; pending: boolean; you: boolean; contact_id: string | null };

export type GroupView = {
  id: string;
  title: string;
  owner: boolean;
  removed: boolean;
  members: GroupMember[];
  member_count: number;
  /** Named an admin by the creator (never true on the creator's own device). */
  am_admin: boolean;
  /** Pinned message ids, oldest first. */
  pins: string[];
  muted: boolean;
  ttl_secs: number | null;
  unread: number;
  /** Unread messages that @mention you (counted even when the group is muted). */
  mentions: number;
  last_activity: number;
  preview: string;
  preview_at: number;
  pinned: boolean;
  archived: boolean;
};

export type DropView = {
  id: string;
  name: string;
  url: string;
  newsroom_key: string;
  journalist_key: string;
  last_sync: number;
  sources: number;
  unread: number;
};

export type SourceView = { id: string; pseudonym: string; first_day: number; last_day: number; unread: number };

export type ItemView = {
  name: string;
  day: number;
  kind: "text" | "file" | "reply";
  text: string | null;
  file_name: string | null;
  mime: string | null;
  size: number | null;
  review: ReviewState;
  received_at: number;
  key: string | null;
};

export type FileEntry = {
  origin: "conversation" | "drop" | "group" | "channel";
  /** Groups and channels: where to ask for a review. */
  review_path?: string;
  key: string;
  where: string;
  name: string;
  mime: string;
  size: number;
  status: string;
  review: ReviewState;
  at: number;
  outgoing: boolean;
  contact_id?: string;
  message_id?: string;
  drop_id?: string;
  source_id?: string;
  item?: string;
};

export type Report = {
  key: string;
  kind: string;
  pages: number;
  metadata: { key: string; value: string }[];
  printer_dots: { page: number; specks: number }[];
  hidden_text: { page: number; count: number; sample: string[] }[];
  has_text: boolean;
};

export type Passage = { kind: "removed" | "added"; text: string };

export type CallView = {
  id: string;
  contact_id: string;
  direction: "outgoing" | "incoming";
  phase: "calling" | "ringing" | "active" | "ended";
  started_at: number;
  answered_at: number | null;
  reason: string | null;
  group: boolean;
  invited: string[];
};

export type CallSettings = { server: string; high_risk_ack: boolean; allowed: boolean; needs_notice: boolean };

export type ChannelFollower = { contact_id: string; joined: boolean; since: number };

export type ChannelView = {
  id: string;
  title: string;
  about: string;
  owner: boolean;
  /** Following side: the conversation the channel comes from. */
  from: string | null;
  invited: boolean;
  closed: boolean;
  removed: boolean;
  /** Owner only; a follower's device never has the audience. */
  subscribers: ChannelFollower[] | null;
  subscriber_count: number | null;
  unread: number;
  last_activity: number;
  preview: string;
  preview_at: number;
  pinned: boolean;
  archived: boolean;
  created_at: number;
  muted: boolean;
  /** Following side: whether the owner may see when you've read posts. */
  receipts: boolean;
};

/** `seen`: owner only, how many followers said they read it (only those who turned receipts on). */
export type ChannelPost = { id: string; body: string; file: FileView | null; sent_at: number; edited: boolean; replies: number; seen: number | null };

export type ChannelReply = { id: string; post_id: string; contact_id: string | null; mine: boolean; body: string; sent_at: number };

/** A one-time secret in a 1:1 conversation. The text is never listed; it comes back once, from "open". */
export type SecretView = { id: string; label: string; mine: boolean; sent_at: number; expires_at: number; opened_at: number | null; state: "waiting" | "opened" | "expired" };

export type HookView = {
  id: string;
  name: string;
  target: { kind: "group" | "channel"; id: string; title: string | null };
  /** What a script sets as SOTTO_HOOK_URL. */
  url: string | null;
  created_at: number;
  last_used: number | null;
  posts: number;
  /** Only in the reply that creates it. */
  token?: string;
};
