import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Lock, Plus, ShieldCheck, Trash2, Users } from "lucide-react";
import { useState } from "react";
import { del, get, patch, post } from "../api";
import { SpotCases } from "../brand/spots";
import type { Contact } from "../types";
import { Button, IconButton } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { Chip, Empty, Notice, Segmented, Skeleton, Switch, timeLabel } from "../ui/bits";
import { useToast } from "../ui/toast";
import { PaneHeader } from "./Shell";

type Role = "owner" | "editor" | "reporter" | "lawyer";
type CaseItem = {
  id: string;
  title: string;
  owner: boolean;
  removed: boolean;
  created_at: number;
  destroy_at: number | null;
  legal_hold: boolean;
  members: { name: string; role: Role; pending: boolean }[];
  insurance: { deadline: number; interval_days: number; recipients: number } | null;
  entries?: { id: string; author: string; at: number; kind: "note" | "source" | "audit" | "system"; text: string; alias: string | null }[];
};
type Grant = { read_id: string; case_title: string; from: string; deadline: number; received_at: number };

const date = (s: number) => new Date(s * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });

export function Cases() {
  const cases = useQuery({ queryKey: ["cases"], queryFn: () => get<CaseItem[]>("/api/cases") });
  const grants = useQuery({ queryKey: ["cases", "grants"], queryFn: () => get<Grant[]>("/api/grants") });
  const [selected, setSelected] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [opened, setOpened] = useState<Grant | null>(null);
  const list = cases.data ?? [];

  return (
    <div className="flex h-full min-h-0">
      <section className="flex w-[300px] shrink-0 flex-col border-r border-line">
        <PaneHeader
          title="Cases"
          actions={
            <IconButton label="New case" onClick={() => setCreating(true)}>
              <Plus size={16} />
            </IconButton>
          }
        />
        <div className="scroll-pane flex-1 px-2 py-2">
          {cases.isPending && <Skeleton className="m-2 h-14" />}
          {list.map((c) => (
            <button key={c.id} onClick={() => setSelected(c.id)} className={`press flex w-full cursor-pointer flex-col rounded-xl px-3 py-2.5 text-left ${selected === c.id ? "bg-muted" : "hover:bg-muted/60"}`}>
              <span className="truncate text-[14px] font-medium">{c.title}</span>
              <span className="mt-0.5 flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
                {c.owner ? "You own this case" : "Member"} · {c.members.filter((m) => !m.pending).length} on the team
                {c.legal_hold && <Lock size={12} aria-label="Legal hold" />}
              </span>
            </button>
          ))}
          {(grants.data?.length ?? 0) > 0 && (
            <div className="mt-4 px-3">
              <div className="mb-1.5 text-[12px] font-medium text-muted-foreground">Insurance files you hold</div>
              {grants.data!.map((g) => (
                <button key={g.read_id} onClick={() => setOpened(g)} className="press flex w-full cursor-pointer flex-col rounded-xl px-3 py-2 text-left hover:bg-muted/60">
                  <span className="truncate text-[13.5px] font-medium">{g.case_title}</span>
                  <span className="text-[12px] text-muted-foreground">
                    From {g.from} · locked until {date(g.deadline)}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>
      </section>
      <section className="flex min-w-0 flex-1 flex-col">
        {selected && list.find((c) => c.id === selected) ? (
          <CaseView key={selected} id={selected} onGone={() => setSelected(null)} />
        ) : (
          <div className="grid flex-1 place-items-center">
            <Empty
              art={<SpotCases />}
              title={list.length ? "Choose a case" : "No cases yet"}
              action={
                <Button variant="primary" icon={<Plus size={16} />} onClick={() => setCreating(true)}>
                  New case
                </Button>
              }
            >
              A case keeps an investigation's notes and source pseudonyms in one end-to-end encrypted space for the reporters, editors and lawyers working on it. Sources are never named here.
            </Empty>
          </div>
        )}
      </section>
      <NewCaseDialog open={creating} onClose={() => setCreating(false)} onCreated={setSelected} />
      <GrantDialog grant={opened} onClose={() => setOpened(null)} />
    </div>
  );
}

function CaseView({ id, onGone }: { id: string; onGone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const c = useQuery({ queryKey: ["cases", id], queryFn: () => get<CaseItem>(`/api/cases/${id}`) });
  const [mode, setMode] = useState<"note" | "source">("note");
  const [text, setText] = useState("");
  const [alias, setAlias] = useState("");
  const [busy, setBusy] = useState(false);
  const [dialog, setDialog] = useState<"members" | "protect" | "destroy" | null>(null);
  if (!c.data) return <PaneHeader title="Case" loading />;
  const k = c.data;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      if (mode === "note") await post(`/api/cases/${id}/notes`, { text });
      else await post(`/api/cases/${id}/sources`, { alias, note: text });
      setText("");
      setAlias("");
      await qc.invalidateQueries({ queryKey: ["cases"] });
    } catch (err) {
      toast("error", (err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <PaneHeader
        title={
          <span className="flex items-center gap-2">
            {k.title}
            {k.legal_hold && <Chip>Legal hold</Chip>}
            {k.insurance && <Chip tone="accent">Insurance file</Chip>}
          </span>
        }
        subtitle={k.destroy_at && !k.legal_hold ? `Destroyed automatically on ${date(k.destroy_at)}` : `Opened ${date(k.created_at)}`}
        actions={
          <>
            <Button size="sm" variant="ghost" icon={<Users size={14} />} onClick={() => setDialog("members")}>
              Team · {k.members.filter((m) => !m.pending).length}
            </Button>
            <Button size="sm" variant="ghost" icon={<ShieldCheck size={14} />} onClick={() => setDialog("protect")}>
              Protection
            </Button>
            <IconButton label="Destroy case" onClick={() => setDialog("destroy")}>
              <Trash2 size={16} />
            </IconButton>
          </>
        }
      />
      <div className="scroll-pane flex-1 px-6 py-5">
        <ol className="flex flex-col gap-3">
          {k.entries?.map((e) =>
            e.kind === "system" || e.kind === "audit" ? (
              <li key={e.id} className="text-center text-[12px] text-muted-foreground">
                {e.kind === "audit" ? `${e.author} ${e.text}` : e.text} · {timeLabel(e.at)}
              </li>
            ) : (
              <li key={e.id} className="rounded-2xl bg-muted px-4 py-3">
                <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
                  <span className="font-medium text-foreground">{e.author}</span>
                  {e.kind === "source" && <Chip tone="accent">Source · {e.alias}</Chip>}
                  <span className="ml-auto tabular">{timeLabel(e.at)}</span>
                </div>
                {e.text && <p className="mt-1.5 text-[14px] leading-relaxed whitespace-pre-wrap">{e.text}</p>}
              </li>
            ),
          )}
        </ol>
      </div>
      {k.removed ? (
        <div className="px-6 pb-5">
          <Notice>You were removed from this case. You can read what you had, but can't add to it.</Notice>
        </div>
      ) : (
        <form onSubmit={submit} className="shrink-0 px-6 pt-2 pb-5">
          <div className="rounded-2xl bg-muted p-3">
            <div className="mb-2 flex items-center justify-between">
              <Segmented
                label="Entry type"
                value={mode}
                onChange={setMode}
                options={[
                  { value: "note", label: "Note" },
                  { value: "source", label: "Source" },
                ]}
              />
              <span className="text-[12px] text-muted-foreground">Seen only by this case's team</span>
            </div>
            {mode === "source" && (
              <input value={alias} onChange={(e) => setAlias(e.target.value)} placeholder="Pseudonym, for example Source QV4. Never a real name." aria-label="Source pseudonym" className="mb-1 w-full bg-transparent px-1 py-1.5 text-[14px] font-medium outline-none placeholder:text-muted-foreground" />
            )}
            <textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={2}
              placeholder={mode === "note" ? "Add a note for the team" : "What the team should know about this source (role, reliability), without identifying them"}
              aria-label={mode === "note" ? "Note" : "Source note"}
              className="w-full resize-none bg-transparent px-1 py-1 text-[14px] leading-relaxed outline-none placeholder:text-muted-foreground"
            />
            <div className="flex justify-end">
              <Button size="sm" variant="primary" type="submit" loading={busy} disabled={mode === "note" ? !text.trim() : !alias.trim()}>
                {mode === "note" ? "Add note" : "Add source"}
              </Button>
            </div>
          </div>
        </form>
      )}
      <MembersDialog open={dialog === "members"} onClose={() => setDialog(null)} k={k} />
      <ProtectDialog open={dialog === "protect"} onClose={() => setDialog(null)} k={k} />
      <Dialog
        open={dialog === "destroy"}
        onClose={() => setDialog(null)}
        title={`Destroy ${k.title}?`}
        description={k.owner ? "Its key is destroyed on every member's desk, which makes every note and record unreadable everywhere. This can't be undone." : "Your copy is destroyed and you leave the case. The rest of the team keeps theirs."}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDialog(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={async () => {
              await del(`/api/cases/${id}`);
              await qc.invalidateQueries({ queryKey: ["cases"] });
              onGone();
            }}
          >
            Destroy case
          </Button>
        </div>
      </Dialog>
    </>
  );
}

function NewCaseDialog({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (id: string) => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <Dialog open={open} onClose={onClose} title="New case" description="Use a working title that wouldn't give the story away if someone saw it.">
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            const c = await post<CaseItem>("/api/cases", { title });
            await qc.invalidateQueries({ queryKey: ["cases"] });
            onCreated(c.id);
            setTitle("");
            onClose();
          } catch (err) {
            setError((err as Error).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label="Working title" value={title} onChange={(e) => setTitle(e.target.value)} autoFocus error={error} placeholder="Harbour" />
        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" loading={busy} disabled={!title.trim()}>
            Open case
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function MembersDialog({ open, onClose, k }: { open: boolean; onClose: () => void; k: CaseItem }) {
  const qc = useQueryClient();
  const toast = useToast();
  const contacts = useQuery({ queryKey: ["contacts"], queryFn: () => get<Contact[]>("/api/contacts"), enabled: open });
  const [contact, setContact] = useState("");
  const [role, setRole] = useState<Role>("editor");
  const ready = (contacts.data ?? []).filter((c) => c.state === "ready");
  return (
    <Dialog open={open} onClose={onClose} title="Team" description="Everyone here can read the whole case. Only the owner adds or removes people." width={520}>
      <ul className="flex flex-col divide-y divide-line">
        {k.members.map((m) => (
          <li key={m.name} className="flex items-center justify-between py-2.5">
            <span className="text-[14px]">
              {m.name} <span className="text-muted-foreground capitalize">· {m.role}</span>
              {m.pending && <span className="ml-1.5 text-[12px] text-muted-foreground">(waiting for their desk)</span>}
            </span>
            {k.owner && m.role !== "owner" && (
              <Button
                size="sm"
                variant="danger-outline"
                onClick={async () => {
                  try {
                    await del(`/api/cases/${k.id}/members/${encodeURIComponent(m.name)}`);
                    await qc.invalidateQueries({ queryKey: ["cases"] });
                  } catch (e) {
                    toast("error", (e as Error).message);
                  }
                }}
              >
                Remove
              </Button>
            )}
          </li>
        ))}
      </ul>
      {k.owner && (
        <form
          className="mt-5 flex flex-col gap-4 rounded-xl bg-muted p-4"
          onSubmit={async (e) => {
            e.preventDefault();
            try {
              await post(`/api/cases/${k.id}/members`, { contact_id: contact, role });
              await qc.invalidateQueries({ queryKey: ["cases"] });
              setContact("");
            } catch (err) {
              toast("error", (err as Error).message);
            }
          }}
        >
          <label className="flex flex-col gap-1.5 text-[13px] font-medium">
            Add a colleague
            <select value={contact} onChange={(e) => setContact(e.target.value)} className="h-10 rounded-lg bg-pane px-3 text-[14px] font-normal outline-none focus:ring-2 focus:ring-primary/40">
              <option value="">Choose from your conversations…</option>
              {ready.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.nickname || c.their_name}
                </option>
              ))}
            </select>
          </label>
          <Segmented
            label="Role"
            value={role}
            onChange={setRole}
            options={[
              { value: "editor", label: "Editor" },
              { value: "reporter", label: "Reporter" },
              { value: "lawyer", label: "Lawyer" },
            ]}
          />
          <div className="flex justify-end">
            <Button size="sm" variant="primary" type="submit" disabled={!contact}>
              Add to case
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

function ProtectDialog({ open, onClose, k }: { open: boolean; onClose: () => void; k: CaseItem }) {
  const qc = useQueryClient();
  const toast = useToast();
  const contacts = useQuery({ queryKey: ["contacts"], queryFn: () => get<Contact[]>("/api/contacts"), enabled: open });
  const [interval, setInterval] = useState("7");
  const [picked, setPicked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const run = async (fn: () => Promise<unknown>, done: string) => {
    setBusy(true);
    try {
      await fn();
      await qc.invalidateQueries({ queryKey: ["cases"] });
      toast("success", done);
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const days = k.destroy_at ? Math.max(1, Math.round((k.destroy_at - Date.now() / 1000) / 86_400)) : 0;
  return (
    <Dialog open={open} onClose={onClose} title="Protection" width={560}>
      <div className="flex flex-col gap-7">
        <section>
          <h3 className="text-[14px] font-semibold">Automatic destruction</h3>
          <p className="mt-0.5 text-[13px] text-muted-foreground">On the date, every desk destroys the case key. Nothing can bring it back.</p>
          <div className="mt-3">
            <Segmented
              label="Destroy after"
              value={String([0, 30, 90, 365].reduce((a, b) => (Math.abs(b - days) < Math.abs(a - days) ? b : a), 0))}
              onChange={(v) => (k.owner ? run(() => patch(`/api/cases/${k.id}`, { destroy_in_days: Number(v) }), "Destruction date updated.") : toast("info", "Only the owner can change this."))}
              options={[
                { value: "0", label: "Never" },
                { value: "30", label: "30 days" },
                { value: "90", label: "90 days" },
                { value: "365", label: "1 year" },
              ]}
            />
          </div>
          <label className="mt-4 flex items-center justify-between gap-4">
            <span>
              <span className="block text-[14px] font-medium">Legal hold</span>
              <span className="block text-[13px] text-muted-foreground">Pauses automatic destruction while lawyers need the record.</span>
            </span>
            <Switch checked={k.legal_hold} onChange={(v) => (k.owner ? run(() => patch(`/api/cases/${k.id}`, { legal_hold: v }), v ? "Legal hold on." : "Legal hold off.") : toast("info", "Only the owner can change this."))} label="Legal hold" />
          </label>
        </section>
        <section>
          <h3 className="text-[14px] font-semibold">Insurance file</h3>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            If you stop checking in, colleagues you choose can open a copy of this case's notes. The relay keeps it locked until then; they hold the key. Unlocking Sotto counts as a check-in.
          </p>
          {k.insurance ? (
            <div className="mt-3 flex flex-col gap-3">
              <Notice tone="accent">
                Locked until {date(k.insurance.deadline)}. Held by {k.insurance.recipients} colleague{k.insurance.recipients === 1 ? "" : "s"}; checks in extend it by {k.insurance.interval_days} days.
              </Notice>
              <div className="flex gap-2">
                <Button size="sm" variant="primary" loading={busy} onClick={() => run(() => post(`/api/cases/${k.id}/insurance/checkin`), "Checked in.")}>
                  Check in now
                </Button>
                <Button size="sm" variant="danger-outline" onClick={() => run(() => del(`/api/cases/${k.id}/insurance`), "Insurance file removed from the relay.")}>
                  Stop insurance file
                </Button>
              </div>
            </div>
          ) : k.owner ? (
            <div className="mt-3 flex flex-col gap-3">
              <Segmented
                label="Release if I don't check in for"
                value={interval}
                onChange={setInterval}
                options={[
                  { value: "3", label: "3 days" },
                  { value: "7", label: "7 days" },
                  { value: "14", label: "14 days" },
                ]}
              />
              <div className="flex flex-wrap gap-1.5">
                {(contacts.data ?? [])
                  .filter((c) => c.state === "ready")
                  .map((c) => (
                    <button
                      key={c.id}
                      type="button"
                      aria-pressed={picked.includes(c.id)}
                      onClick={() => setPicked((p) => (p.includes(c.id) ? p.filter((x) => x !== c.id) : [...p, c.id]))}
                      className={`press h-8 cursor-pointer rounded-full px-3 text-[13px] ${picked.includes(c.id) ? "bg-foreground text-pane" : "bg-muted text-muted-foreground"}`}
                    >
                      {c.nickname || c.their_name}
                    </button>
                  ))}
              </div>
              <div>
                <Button size="sm" variant="primary" loading={busy} disabled={!picked.length} onClick={() => run(() => post(`/api/cases/${k.id}/insurance`, { interval_days: Number(interval), contact_ids: picked }), "Insurance file set up.")}>
                  Set up insurance file
                </Button>
              </div>
            </div>
          ) : (
            <p className="mt-3 text-[13px] text-muted-foreground">Only the case owner can set this up.</p>
          )}
        </section>
      </div>
    </Dialog>
  );
}

function GrantDialog({ grant, onClose }: { grant: Grant | null; onClose: () => void }) {
  const [result, setResult] = useState<{ locked: boolean; deadline?: number; entries?: { id: string; author: string; text: string; kind: string; alias: string | null }[] } | null>(null);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  return (
    <Dialog
      open={!!grant}
      onClose={() => {
        setResult(null);
        onClose();
      }}
      title={grant ? `Insurance file: ${grant.case_title}` : ""}
      description={grant ? `${grant.from} asked you to hold this. It opens only if they stop checking in.` : undefined}
      width={600}
    >
      {!result ? (
        <Button
          variant="primary"
          loading={busy}
          onClick={async () => {
            setBusy(true);
            try {
              setResult(await post(`/api/grants/${grant!.read_id}/open`));
            } catch (e) {
              toast("error", (e as Error).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          Try to open
        </Button>
      ) : result.locked ? (
        <Notice>Still locked until {date(result.deadline!)}. {grant?.from} has been checking in, which is a good sign.</Notice>
      ) : (
        <ol className="flex max-h-96 flex-col gap-2 overflow-y-auto">
          {result.entries?.map((e) => (
            <li key={e.id} className="rounded-xl bg-muted px-3 py-2 text-[13.5px]">
              <span className="font-medium">{e.author}</span> {e.alias && <Chip>Source · {e.alias}</Chip>}
              <p className="whitespace-pre-wrap">{e.text}</p>
            </li>
          ))}
        </ol>
      )}
    </Dialog>
  );
}
