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
  invite_expires: number | null;
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
  kind: "text" | "file" | "system";
  text: string | null;
  file: FileView | null;
  sent_at: number;
  received_at: number;
  expires_at: number | null;
  delivery: "sending" | "sent" | "delivered" | "failed" | "received";
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
  origin: "conversation" | "drop";
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
