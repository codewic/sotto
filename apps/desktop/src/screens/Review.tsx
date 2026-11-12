import { useQuery } from "@tanstack/react-query";
import { Download, FileText, GitCompare, Palette } from "lucide-react";
import { useEffect, useState } from "react";
import { chooseSavePath, get, imageUrl, post } from "../api";
import { SpotSandbox } from "../brand/spots";
import type { FileEntry, Passage, Report } from "../types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Chip, Empty, Notice, Skeleton, bytes, timeLabel } from "../ui/bits";
import { useToast } from "../ui/toast";
import { PaneHeader } from "./Shell";

export function Review({ initial }: { initial?: string }) {
  const files = useQuery({ queryKey: ["files"], queryFn: () => get<FileEntry[]>("/api/files") });
  const [key, setKey] = useState<string | null>(initial ?? null);
  const [filter, setFilter] = useState<"all" | "quarantined" | "clean">("all");
  useEffect(() => {
    if (initial) setKey(initial);
  }, [initial]);
  const incoming = (files.data ?? []).filter((f) => !f.outgoing);
  const shown = incoming.filter((f) => filter === "all" || f.review.state === filter);
  const waiting = incoming.filter((f) => f.review.state === "quarantined").length;

  return (
    <div className="flex h-full min-h-0">
      <section className="flex w-[320px] shrink-0 flex-col border-r border-line">
        <PaneHeader title="Safe review" subtitle={waiting ? `${waiting} file${waiting === 1 ? "" : "s"} waiting in quarantine` : "Every received file, opened only in the sandbox"} />
        <div className="flex gap-1.5 px-4 pt-3">
          {(["all", "quarantined", "clean"] as const).map((f) => (
            <button key={f} onClick={() => setFilter(f)} className={`press h-7 cursor-pointer rounded-full px-3 text-[12.5px] ${filter === f ? "bg-foreground text-pane" : "bg-muted text-muted-foreground hover:text-foreground"}`}>
              {f === "all" ? "All" : f === "quarantined" ? "Quarantined" : "Clean"}
            </button>
          ))}
        </div>
        <div className="scroll-pane flex-1 px-2 py-2">
          {files.isPending && <Skeleton className="m-2 h-14" />}
          {incoming.length > 0 && shown.length === 0 && (
            <div className="px-3 py-6 text-center text-[13px] text-muted-foreground">
              No {filter} files.{" "}
              <button className="cursor-pointer font-medium text-foreground underline underline-offset-4" onClick={() => setFilter("all")}>
                Show all
              </button>
            </div>
          )}
          {shown.map((f) => (
            <button key={f.key} onClick={() => setKey(f.key)} className={`press flex w-full cursor-pointer items-center gap-3 rounded-xl px-3 py-2.5 text-left ${key === f.key ? "bg-muted" : "hover:bg-muted/60"}`}>
              <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-muted-strong text-muted-foreground">
                <FileText size={17} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[14px] font-medium">{f.name}</span>
                <span className="block truncate text-[12.5px] text-muted-foreground">
                  {f.where} · {bytes(f.size)}
                </span>
              </span>
              <ReviewChip state={f.review.state} />
            </button>
          ))}
        </div>
      </section>
      <section className="flex min-w-0 flex-1 flex-col">
        {key ? (
          <ReportView key={key} reviewKey={key} file={incoming.find((f) => f.key === key)} files={incoming} />
        ) : (
          <div className="grid flex-1 place-items-center">
            <Empty art={<SpotSandbox />} title={incoming.length ? "Choose a file to review" : "No files received yet"}>
              Files you receive stay quarantined. Reviewing one renders it inside a sealed sandbox with no network, then rebuilds a clean copy from its pixels, so scripts, hidden text and tracking data can't come along.
            </Empty>
          </div>
        )}
      </section>
    </div>
  );
}

function ReviewChip({ state }: { state: FileEntry["review"]["state"] }) {
  if (state === "clean") return <Chip tone="accent">Clean</Chip>;
  if (state === "failed") return <Chip tone="danger">Can't open</Chip>;
  if (state === "quarantined") return <Chip>Quarantined</Chip>;
  return null;
}

function reviewPath(f: FileEntry) {
  if (f.review_path) return f.review_path;
  return f.origin === "conversation" ? `/api/contacts/${f.contact_id}/messages/${f.message_id}/review` : `/api/drops/${f.drop_id}/sources/${f.source_id}/items/${f.item}/review`;
}

function ReportView({ reviewKey, file, files }: { reviewKey: string; file?: FileEntry; files: FileEntry[] }) {
  const toast = useToast();
  const report = useQuery({ queryKey: ["review", reviewKey], queryFn: () => get<Report>(`/api/reviews/${encodeURIComponent(reviewKey)}`), retry: false });
  const [busy, setBusy] = useState(false);
  const [compare, setCompare] = useState(false);

  async function run(greyscale: boolean) {
    if (!file) return;
    setBusy(true);
    try {
      await post(reviewPath(file), { greyscale });
      await report.refetch();
      toast("success", greyscale ? "Rebuilt in greyscale. Colour tracking dots can't survive this." : "Clean copy ready.");
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function exportClean() {
    const base = (file?.name ?? "document").replace(/\.[^.]+$/, "");
    const path = await chooseSavePath(`${base} (clean copy).pdf`);
    if (!path) return;
    try {
      await post(`/api/reviews/${encodeURIComponent(reviewKey)}/export`, { path });
      toast("success", "Clean copy saved. It holds only page images: no text layer, metadata or links.");
    } catch (e) {
      toast("error", (e as Error).message);
    }
  }

  if (report.isPending) {
    return (
      <>
        <PaneHeader title={file?.name ?? "Review"} loading />
        <div className="p-6">
          <Skeleton className="h-64" />
        </div>
      </>
    );
  }
  if (report.isError || !report.data) {
    return (
      <>
        <PaneHeader title={file?.name ?? "Review"} subtitle={file ? `${file.where} · received ${timeLabel(file.at * 1000)}` : undefined} loading={busy} />
        <div className="grid flex-1 place-items-center">
          <Empty
            art={<SpotSandbox />}
            title="This file is still quarantined"
            action={
              file && (
                <Button variant="primary" loading={busy} onClick={() => run(false)}>
                  {busy ? "Opening in the sandbox…" : "Review safely"}
                </Button>
              )
            }
          >
            Nothing has opened it yet. Reviewing takes a few seconds and never touches the rest of this computer.
          </Empty>
        </div>
      </>
    );
  }
  const r = report.data;
  const findings = r.metadata.length + r.printer_dots.length + r.hidden_text.length;
  const others = files.filter((f) => f.key !== reviewKey && f.review.state === "clean");
  return (
    <>
      <PaneHeader
        title={file?.name ?? "Clean copy"}
        subtitle={`${r.pages} page${r.pages === 1 ? "" : "s"} · ${findings ? `${findings} finding${findings === 1 ? "" : "s"} before sharing` : "Nothing found that points to who sent it"}`}
        loading={busy}
        actions={
          <>
            <Button size="sm" variant="ghost" icon={<GitCompare size={14} />} onClick={() => setCompare(true)} disabled={!r.has_text}>
              Compare versions
            </Button>
            <Button size="sm" variant="ghost" icon={<Palette size={14} />} onClick={() => run(true)} disabled={!file}>
              Greyscale
            </Button>
            <Button size="sm" variant="primary" icon={<Download size={14} />} onClick={exportClean}>
              Export clean copy
            </Button>
          </>
        }
      />
      <div className="scroll-pane flex flex-1 gap-6 p-6">
        <div className="flex min-w-0 flex-1 flex-col items-center gap-4">
          {Array.from({ length: r.pages }, (_, i) => (
            <PageImage key={i} reviewKey={reviewKey} n={i + 1} />
          ))}
        </div>
        <aside className="flex w-[300px] shrink-0 flex-col gap-4">
          <h2 className="text-[13px] font-semibold tracking-tight">Before you share</h2>
          <Finding title="Hidden details" ok={r.metadata.length === 0} okText="No author, device or software details found.">
            <ul className="mt-2 flex flex-col gap-1.5">
              {r.metadata.map((m) => (
                <li key={m.key} className="text-[12.5px]">
                  <span className="font-medium">{m.key}</span> <span className="text-muted-foreground">{m.value}</span>
                </li>
              ))}
            </ul>
            <p className="mt-2 text-[12.5px] text-muted-foreground">Removed from the clean copy.</p>
          </Finding>
          <Finding title="Printer tracking dots" ok={r.printer_dots.length === 0} okText="No tracking dot pattern found.">
            <p className="mt-2 text-[12.5px] text-muted-foreground">
              Faint yellow dots on page{r.printer_dots.length === 1 ? "" : "s"} {r.printer_dots.map((d) => d.page).join(", ")}. Colour printers add them to identify the printer and time. They were removed from the clean copy; rebuild in greyscale to be certain.
            </p>
          </Finding>
          <Finding title="Text under black boxes" ok={r.hidden_text.length === 0} okText="No fake redactions found.">
            {r.hidden_text.map((h) => (
              <p key={h.page} className="mt-2 text-[12.5px] text-muted-foreground">
                Page {h.page}: {h.count} word{h.count === 1 ? "" : "s"} still readable under a black box, including <span className="font-mono text-foreground">{h.sample.slice(0, 4).join(" ")}</span>. The clean copy keeps only pixels, so this text is gone from it.
              </p>
            ))}
          </Finding>
          <Notice>
            Before sharing, think about timing and quoting. If few people saw this document, sharing it soon after they did, or quoting its exact wording, can point to them. Paraphrase where you can.
          </Notice>
        </aside>
      </div>
      <CompareDialog open={compare} onClose={() => setCompare(false)} a={reviewKey} options={others} />
    </>
  );
}

function Finding({ title, ok, okText, children }: { title: string; ok: boolean; okText: string; children: React.ReactNode }) {
  return (
    <div className="rounded-xl bg-muted p-3.5">
      <div className="flex items-center gap-2 text-[13.5px] font-medium">
        <span aria-hidden="true" className={`size-2 rounded-full ${ok ? "bg-primary" : "bg-destructive"}`} />
        {title}
        <span className="ml-auto text-[12px] font-normal text-muted-foreground">{ok ? "Clear" : "Found"}</span>
      </div>
      {ok ? <p className="mt-1.5 text-[12.5px] text-muted-foreground">{okText}</p> : children}
    </div>
  );
}

function PageImage({ reviewKey, n }: { reviewKey: string; n: number }) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let url: string | null = null;
    imageUrl(`/api/reviews/${encodeURIComponent(reviewKey)}/pages/${n}`)
      .then((u) => {
        url = u;
        setSrc(u);
      })
      .catch(() => setFailed(true));
    return () => {
      if (url) URL.revokeObjectURL(url);
    };
  }, [reviewKey, n]);
  if (failed) return <div className="grid h-40 w-full max-w-[680px] place-items-center rounded-xl bg-muted text-[13px] text-muted-foreground">Page {n} couldn't be shown.</div>;
  if (!src) return <Skeleton className="aspect-[1/1.3] w-full max-w-[680px]" />;
  return <img src={src} alt={`Page ${n} of the clean copy`} className="w-full max-w-[680px] rounded-lg shadow-pane" />;
}

function CompareDialog({ open, onClose, a, options }: { open: boolean; onClose: () => void; a: string; options: FileEntry[] }) {
  const [b, setB] = useState<string | null>(null);
  const [result, setResult] = useState<Passage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      open={open}
      onClose={() => {
        setResult(null);
        setB(null);
        setError(null);
        onClose();
      }}
      title="Compare two versions"
      description="Organisations sometimes give each person a slightly different copy to catch leaks. Passages that differ are the ones that could identify whoever passed it on: paraphrase them, don't quote them."
      width={620}
    >
      {options.length === 0 ? (
        <Notice>Review a second version of this document first. Both need a text layer; scans and photos can't be compared.</Notice>
      ) : (
        <>
          <div className="flex flex-col gap-1">
            {options.map((o) => (
              <button key={o.key} onClick={() => setB(o.key)} className={`press flex h-11 cursor-pointer items-center justify-between rounded-xl px-4 text-left text-[14px] ${b === o.key ? "bg-muted font-medium" : "hover:bg-muted/60"}`}>
                <span className="truncate">{o.name}</span>
                <span className="text-[12px] text-muted-foreground">{o.where}</span>
              </button>
            ))}
          </div>
          <div className="mt-4 flex justify-end">
            <Button
              variant="primary"
              disabled={!b}
              loading={busy}
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  setResult(await post<Passage[]>("/api/reviews/compare", { a, b }));
                } catch (e) {
                  setError((e as Error).message);
                } finally {
                  setBusy(false);
                }
              }}
            >
              Compare
            </Button>
          </div>
          {error && (
            <div className="mt-4">
              <Notice tone="danger">{error}</Notice>
            </div>
          )}
          {result && (
            <div className="mt-5 max-h-72 overflow-y-auto rounded-xl bg-muted p-4">
              {result.length === 0 ? (
                <p className="text-[13px] text-muted-foreground">The two versions have identical text.</p>
              ) : (
                <ul className="flex flex-col gap-2">
                  {result.map((p, i) => (
                    <li key={i} className="text-[13px]">
                      <span className="mr-2 text-[11.5px] font-medium tracking-wide text-muted-foreground">{p.kind === "removed" ? "Only in this file" : "Only in the other"}</span>
                      <span className="font-mono">{p.text}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </>
      )}
    </Dialog>
  );
}
