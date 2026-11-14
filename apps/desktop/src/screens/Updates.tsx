import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpCircle, RefreshCw } from "lucide-react";
import { useState } from "react";
import { type ApiError, get, post } from "../api";
import type { CallView } from "../types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { CopyButton, Notice } from "../ui/bits";
import { useToast } from "../ui/toast";
import { Rich } from "../ui/Rich";

export type UpdateView = {
  current: string;
  state: "idle" | "checking" | "current" | "available" | "downloading" | "ready" | "error";
  version: string | null;
  notes: string;
  error: string | null;
  checked_at: number | null;
  /** While downloading: how much has arrived, in percent. */
  progress: number | null;
  /** This copy can replace itself (an AppImage, .deb or Windows install). */
  installable: boolean;
};

const RELEASES_PAGE = "https://github.com/codewic/sotto-releases/releases/latest";

export function useUpdates() {
  return useQuery({ queryKey: ["updates"], queryFn: () => get<UpdateView>("/api/updates") });
}

/** In the sidebar, only when a new version is waiting. */
export function UpdateNotice() {
  const u = useUpdates().data;
  const [open, setOpen] = useState(false);
  if (!u || !(u.state === "ready" || (u.state === "available" && !u.installable))) return null;
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="press flex h-10 cursor-pointer items-center gap-3 rounded-xl bg-[color-mix(in_srgb,var(--primary)_12%,transparent)] px-3 text-left text-[13.5px] font-medium text-primary hover:bg-[color-mix(in_srgb,var(--primary)_18%,transparent)]"
        aria-label={`Sotto ${u.version} is ready`}
      >
        <ArrowUpCircle size={17} className="shrink-0" />
        <span className="hidden min-w-0 flex-1 truncate md:inline">{u.state === "ready" ? "Update ready" : "Update available"}</span>
      </button>
      <UpdateDialog open={open} onClose={() => setOpen(false)} u={u} />
    </>
  );
}

function UpdateDialog({ open, onClose, u }: { open: boolean; onClose: () => void; u: UpdateView }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const calls = useQuery({ queryKey: ["calls"], queryFn: () => get<CallView[]>("/api/calls") });
  const onCall = (calls.data ?? []).some((c) => c.phase !== "ended");
  const canInstall = u.state === "ready" && !!window.sotto?.installUpdate;
  return (
    <Dialog open={open} onClose={onClose} title={`Sotto ${u.version}`} description={canInstall ? "Downloaded and checked against Sotto's release signature. Restarting takes a few seconds and keeps you where you are." : "A new version is out."} width={500}>
      <div className="flex flex-col gap-4">
        {u.notes && (
          <div className="scroll-pane max-h-60 overflow-auto rounded-xl bg-muted px-4 py-3 text-[13.5px] leading-relaxed whitespace-pre-wrap">
            <Rich text={u.notes} />
          </div>
        )}
        {!canInstall && (
          <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <span className="min-w-0 flex-1">This copy can't update itself. Download the new version from the releases page; links aren't opened from Sotto, so copy it into your browser.</span>
            <CopyButton text={RELEASES_PAGE} label="Copy link" />
          </div>
        )}
        {onCall && canInstall && <Notice>Finish your call first; restarting would end it.</Notice>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Later
          </Button>
          {canInstall && (
            <Button
              variant="primary"
              loading={busy}
              disabled={onCall}
              onClick={async () => {
                setBusy(true);
                const r = await window.sotto?.installUpdate?.();
                if (r && !r.ok) toast("error", r.error ?? "The update couldn't be installed.");
                setBusy(false);
              }}
            >
              Restart to update
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}

function when(sec: number | null) {
  if (!sec) return "not yet";
  const d = Date.now() / 1000 - sec;
  if (d < 90) return "just now";
  if (d < 3600) return `${Math.round(d / 60)} min ago`;
  if (d < 86_400) return `${Math.round(d / 3600)} h ago`;
  return new Date(sec * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/** Settings, About: the version, what the updater last did, and a manual check. */
export function UpdatePanel({ extreme }: { extreme: boolean }) {
  const qc = useQueryClient();
  const toast = useToast();
  const q = useUpdates();
  const [open, setOpen] = useState(false);
  const u = q.data;
  const checking = u?.state === "checking" || u?.state === "downloading";
  const line = !u
    ? ""
    : u.state === "current"
      ? `Up to date. Checked ${when(u.checked_at)}.`
      : u.state === "ready"
        ? `Version ${u.version} is downloaded, checked and ready.`
        : u.state === "available"
          ? `Version ${u.version} is available.`
          : u.state === "downloading"
            ? `Downloading version ${u.version}… ${u.progress ?? 0}%`
            : u.state === "checking"
              ? "Checking…"
              : u.state === "error"
                ? (u.error ?? "The last check failed.")
                : extreme
                  ? "In Extreme mode Sotto checks only when you ask."
                  : "Sotto checks for signed updates about every 6 hours.";
  return (
    <div className="mt-3 flex flex-wrap items-center gap-3">
      <span className={`min-w-0 flex-1 text-[13px] tabular-nums ${u?.state === "error" ? "text-destructive" : "text-muted-foreground"}`} aria-live="polite">
        {line}
      </span>
      {(u?.state === "ready" || u?.state === "available") && (
        <Button size="sm" variant="primary" onClick={() => setOpen(true)}>
          See what's new
        </Button>
      )}
      <Button
        size="sm"
        icon={<RefreshCw size={14} />}
        loading={checking}
        onClick={async () => {
          try {
            await post("/api/updates/check");
            await qc.invalidateQueries({ queryKey: ["updates"] });
          } catch (e) {
            toast("error", (e as ApiError).message);
          }
        }}
      >
        Check for updates
      </Button>
      {u && <UpdateDialog open={open} onClose={() => setOpen(false)} u={u} />}
    </div>
  );
}
