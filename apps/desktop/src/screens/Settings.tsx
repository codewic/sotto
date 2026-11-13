import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { type ApiError, chooseSavePath, del, get, patch, post } from "../api";
import { Mark } from "../brand/Mark";
import type { CallSettings, ChannelView, GroupView, HookView, RiskMode } from "../types";
import { Rich } from "../ui/Rich";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { Chip, CopyButton, Notice, Segmented, Switch } from "../ui/bits";
import { useToast } from "../ui/toast";
import { forgetAppearance } from "../theme/appearance";
import { Appearance } from "./Appearance";
import { EraseDialog } from "./Unlock";
import { MyAvatar } from "./AvatarPicker";
import { UpdatePanel } from "./Updates";
import { TagPanel, type TagView } from "./ConversationDialogs";
import { PaneHeader, useNav } from "./Shell";

const modes: Record<RiskMode, { title: string; lines: string[] }> = {
  standard: { title: "Standard", lines: ["Tor, end-to-end encryption, disappearing messages", "Locks after the time you choose"] },
  high: { title: "High risk", lines: ["Adds cover traffic so your activity is harder to time", "Locks after one idle minute"] },
  extreme: { title: "Extreme", lines: ["Everything in High risk", "Keep this project on this computer only; review documents here, never on a phone"] },
};

type Tab = "general" | "appearance";
/** Settings reopens on the section you last used. */
let lastTab: Tab = "general";

export function Settings({ initial }: { initial?: string }) {
  const [tab, setTabState] = useState<Tab>(initial === "appearance" ? "appearance" : initial === "calls" || initial === "connection" ? "general" : lastTab);
  // Opened from "Set up calls first": go straight to the Calls section.
  useEffect(() => {
    if (initial === "calls") document.getElementById("settings-calls")?.scrollIntoView({ block: "start" });
    if (initial === "connection") document.getElementById("settings-connection")?.scrollIntoView({ block: "start" });
  }, [initial]);
  const setTab = (t: Tab) => {
    lastTab = t;
    setTabState(t);
  };
  const qc = useQueryClient();
  const toast = useToast();
  const { status } = useNav();
  const s = status.settings!;
  const p = status.profile!;
  const [name, setName] = useState(p.display_name);
  const [relay, setRelay] = useState(p.default_relay);
  const [socks, setSocks] = useState(p.transport.kind === "tor" ? p.transport.socks : "127.0.0.1:9050");
  const [via, setVia] = useState<"direct" | "tor">(p.transport.kind);
  const shipped = status.shipped_relay ?? "";
  const [dialog, setDialog] = useState<"passcode" | "duress" | "erase" | "travel" | null>(null);

  const tabs = (short: boolean) => (
    <Segmented
      label="Settings section"
      value={tab}
      onChange={setTab}
      options={[
        { value: "general", label: short ? "Protection" : "Protection and account" },
        { value: "appearance", label: "Appearance" },
      ]}
    />
  );

  async function save(body: Record<string, unknown>, done?: string) {
    try {
      await patch("/api/settings", body);
      await qc.invalidateQueries({ queryKey: ["status"] });
      if (done) toast("success", done);
    } catch (e) {
      toast("error", (e as Error).message);
    }
  }

  return (
    <>
      <PaneHeader title="Settings" actions={<div className="hidden sm:block">{tabs(false)}</div>} />
      <div className="scroll-pane flex-1">
        <div className="px-6 pt-4 sm:hidden">{tabs(true)}</div>
        {tab === "appearance" ? (
          <Appearance />
        ) : (
        <div className="mx-auto flex max-w-[760px] flex-col gap-10 px-8 py-8">
          <Section title="Protection level" description="Match Sotto to the risk of the work in front of you. You can change this any time.">
            <Segmented
              label="Protection level"
              value={s.risk_mode}
              onChange={(m) => save({ risk_mode: m }, `${modes[m].title} protection is on.`)}
              options={(Object.keys(modes) as RiskMode[]).map((m) => ({ value: m, label: modes[m].title }))}
            />
            <ul className="mt-4 flex flex-col gap-1 text-[13.5px] text-muted-foreground">
              {modes[s.risk_mode].lines.map((l) => (
                <li key={l} className="flex gap-2">
                  <span aria-hidden="true" className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
                  {l}
                </li>
              ))}
            </ul>
            {s.risk_mode === "standard" && (
              <div className="mt-5 flex items-center gap-3">
                <span className="text-[13.5px]">Lock after</span>
                <Segmented
                  label="Auto-lock"
                  value={String(s.auto_lock_minutes)}
                  onChange={(v) => save({ auto_lock_minutes: Number(v) })}
                  options={[
                    { value: "5", label: "5 min" },
                    { value: "15", label: "15 min" },
                    { value: "60", label: "1 hour" },
                  ]}
                />
              </div>
            )}
          </Section>

          <Section title="Identity" description="Nobody you talk to sees an account. This is how you appear when you choose to share a name.">
            <form
              className="flex flex-col gap-6"
              onSubmit={(e) => {
                e.preventDefault();
                void save({ display_name: name }, "Name saved.");
              }}
            >
              <MyAvatar />
              <Field label="Display name" value={name} onChange={(e) => setName(e.target.value)} hint="Only shared with people you invite, and only if you choose to." />
              <div>
                <div className="mb-2 text-[13px] font-medium">Tag</div>
                <TagPanel />
                <TagRelease />
              </div>
              <div>
                <div className="text-[13px] font-medium">Organization identity fingerprint</div>
                <div className="mt-1.5 flex items-center gap-3">
                  <code className="rounded-lg bg-muted px-2.5 py-1.5 font-mono text-[12.5px] tabular">{p.fingerprint}</code>
                  <CopyButton text={p.fingerprint} />
                </div>
                <p className="mt-1.5 text-[13px] text-muted-foreground">Your organization can publish this on its website, so people writing to you can check it's really you.</p>
              </div>
              <div>
                <Button type="submit" size="sm">
                  Save name
                </Button>
              </div>
            </form>
          </Section>

          <Section id="settings-connection" title="Connection" description="How Sotto reaches the relay that carries your sealed messages. Changes apply the next time you unlock Sotto.">
            <form
              className="flex flex-col gap-6"
              onSubmit={(e) => {
                e.preventDefault();
                void save({ default_relay: relay, transport: via === "tor" ? { kind: "tor", socks } : { kind: "direct" } }, "Connection saved. Lock and unlock Sotto to use it.");
              }}
            >
              <div className="flex flex-col gap-2">
                <Segmented
                  label="Connect"
                  value={via}
                  onChange={setVia}
                  options={[
                    { value: "direct", label: "Directly" },
                    { value: "tor", label: "Through Tor" },
                  ]}
                />
                <p className="text-[12.5px] text-muted-foreground">
                  {via === "direct"
                    ? "The relay sees your network address, never your messages or who you talk to."
                    : "The relay never learns where you are. Tor Browser or the tor service must be running on this computer."}
                </p>
              </div>
              {via === "tor" && <Field label="Tor address" value={socks} onChange={(e) => setSocks(e.target.value)} spellCheck={false} hint="tor service: 127.0.0.1:9050 · Tor Browser: 127.0.0.1:9150" />}
              {via === "tor" && status.tor_running === false && p.transport.kind === "tor" && <Notice tone="danger">Nothing answers at {p.transport.socks}, so Sotto can't connect. Start Tor Browser or the tor service, or connect directly.</Notice>}
              {via === "direct" && s.risk_mode !== "standard" && <Notice tone="danger">In {s.risk_mode === "high" ? "High risk" : "Extreme"} mode, connect through Tor so relays can't see your network address.</Notice>}
              <Field
                label="Relay address"
                value={relay}
                onChange={(e) => setRelay(e.target.value)}
                spellCheck={false}
                hint={relay === shipped ? "Sotto's own relay. If your organization runs one, enter its address. New conversations use it; existing ones keep theirs." : "New conversations use this relay. Existing ones keep theirs."}
              />
              {shipped && relay !== shipped && (
                <div>
                  <Button type="button" size="sm" variant="ghost" onClick={() => setRelay(shipped)}>
                    Use Sotto's relay
                  </Button>
                </div>
              )}
              <div className="flex flex-wrap gap-2">
                {(status.relays ?? []).map((r) => (
                  <Chip key={r.url} tone={r.status === "online" ? "accent" : r.status === "offline" ? "danger" : "neutral"}>
                    {r.status === "online" ? "Online" : r.status === "offline" ? "Unreachable" : "Connecting"} · {r.url.replace(/^wss?:\/\//, "").slice(0, 28)}
                  </Chip>
                ))}
              </div>
              <div>
                <Button type="submit" size="sm">
                  Save connection
                </Button>
              </div>
            </form>
          </Section>

          <CallsSection risk={s.risk_mode} />
          <IntegrationsSection />

          <Section title="Anonymous inbox" description="How messages and files from anonymous senders are handled once they reach this device.">
            <Row title="Delete from the inbox server after download" text="The server keeps nothing once this device has it. Turn off only if colleagues also read this inbox.">
              <Switch checked={s.delete_after_download} onChange={(v) => save({ delete_after_download: v })} label="Delete from the inbox server after download" />
            </Row>
          </Section>

          <Section title="Passcodes" description="Your passcode is the only way into this vault. There is no reset.">
            <div className="flex flex-wrap gap-2">
              <Button size="sm" onClick={() => setDialog("passcode")}>
                Change passcode
              </Button>
              <Button size="sm" onClick={() => setDialog("duress")}>
                Duress passcode
              </Button>
            </div>
            {status.sandbox === false && (
              <div className="mt-4">
                <Notice tone="danger">Safe review isn't available: this computer is missing the sandbox it needs (bubblewrap). Files stay quarantined until it is installed.</Notice>
              </div>
            )}
          </Section>

          <Section title="Travel mode" description="Before a border crossing or any place your devices may be searched.">
            <Row title="Take Sotto off this device for a while" text="Seals everything into one backup that only a recovery phrase opens, then erases this device. Restore it when you're safe.">
              <Button size="sm" onClick={() => setDialog("travel")}>
                Start travel mode
              </Button>
            </Row>
          </Section>

          <Section title="Danger zone" description="For when this device is about to be taken.">
            <Row title="Erase Sotto from this device" text="Destroys every key, conversation and file here at once. It can't be undone.">
              <Button size="sm" variant="danger-outline" onClick={() => setDialog("erase")}>
                Erase this device
              </Button>
            </Row>
          </Section>

          <Section title="About" description="">
            <div className="flex items-start gap-4">
              <Mark size={40} />
              <div className="text-[13.5px] text-muted-foreground">
                <div className="font-medium text-foreground">Sotto {status.version}</div>
                <UpdatePanel extreme={s.risk_mode === "extreme"} />
                <p className="mt-1">A private, end-to-end encrypted workspace for teams. Open source, no analytics, no telemetry. Built by Vorynza.</p>
                <p className="mt-2">
                  What Sotto can't protect against: spyware on your devices, details in the information that only a few people know, and someone watching both ends of the Tor network at once.
                </p>
              </div>
            </div>
          </Section>
        </div>
        )}
      </div>
      <PasscodeDialog open={dialog === "passcode"} onClose={() => setDialog(null)} />
      <DuressDialog open={dialog === "duress"} onClose={() => setDialog(null)} />
      <EraseDialog open={dialog === "erase"} onClose={() => setDialog(null)} />
      <TravelDialog open={dialog === "travel"} onClose={() => setDialog(null)} />
    </>
  );
}

function Section({ id, title, description, children }: { id?: string; title: string; description: string; children: React.ReactNode }) {
  return (
    <section id={id} className="scroll-mt-6 grid grid-cols-1 gap-4 border-b border-line pb-10 last:border-0 md:grid-cols-[220px_1fr] md:gap-8">
      <div>
        <h2 className="text-[14.5px] font-semibold tracking-tight">{title}</h2>
        {description && <p className="mt-1 text-[13px] text-muted-foreground">{description}</p>}
      </div>
      <div>{children}</div>
    </section>
  );
}

function CallsSection({ risk }: { risk: RiskMode }) {
  const qc = useQueryClient();
  const toast = useToast();
  const settings = useQuery({ queryKey: ["call-settings", risk], queryFn: () => get<CallSettings>("/api/calls/settings") });
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [test, setTest] = useState<{ state: "idle" | "testing" | "ok" | "failed"; text?: string }>({ state: "idle" });
  const saved = settings.data?.server ?? "";
  const value = draft ?? saved;

  async function runTest() {
    setTest({ state: "testing" });
    try {
      const r = await post<{ media: string }>("/api/calls/test");
      setTest({ state: "ok", text: `Connected. Calls will use ${r.media}.` });
    } catch (e) {
      setTest({ state: "failed", text: (e as ApiError).message });
    }
  }

  return (
    <Section id="settings-calls" title="Calls" description="Voice calls go through a call server. They're end-to-end encrypted: the server can't hear them.">
      <form
        className="flex flex-col gap-5"
        onSubmit={async (e) => {
          e.preventDefault();
          setError(null);
          try {
            await patch("/api/calls/settings", { server: value });
            setDraft(null);
            await qc.invalidateQueries({ queryKey: ["call-settings"] });
            toast("success", value.trim() ? "Call server saved." : "Calls turned off.");
            if (value.trim()) void runTest();
          } catch (err) {
            setError((err as ApiError).message);
          }
        }}
      >
        <Field
          label="Call server"
          value={value}
          onChange={(e) => {
            setDraft(e.target.value);
            setTest({ state: "idle" });
          }}
          placeholder="https://calls.example.org"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          error={error}
          hint={
            settings.data?.is_default && draft === null
              ? "Sotto's own call server, ready to use. If your organization runs its own, enter its address instead. Leave it empty to turn calls off."
              : "Your organization's call server. Leave it empty to turn calls off."
          }
        />
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" size="sm" disabled={draft === null || draft.trim() === saved}>
            Save call server
          </Button>
          <Button type="button" size="sm" variant="ghost" disabled={!saved || draft !== null || test.state === "testing"} loading={test.state === "testing"} onClick={() => void runTest()}>
            Test connection
          </Button>
          {settings.data && !settings.data.is_default && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={async () => {
                setError(null);
                await patch("/api/calls/settings", { server: settings.data?.default_server });
                setDraft(null);
                await qc.invalidateQueries({ queryKey: ["call-settings"] });
                toast("success", "Using Sotto's call server.");
                void runTest();
              }}
            >
              Use Sotto's call server
            </Button>
          )}
          {test.state === "ok" && (
            <span className="inline-flex items-center gap-1.5 text-[13px] text-muted-foreground" role="status">
              <Check size={14} className="text-primary" aria-hidden="true" />
              {test.text}
            </span>
          )}
        </div>
        {test.state === "failed" && <Notice tone="danger">{test.text}</Notice>}
        <Notice>
          {risk === "extreme"
            ? "Calls are off in Extreme mode: the call server would see your network address. Messages still go over Tor."
            : "The call server sees your network address and when calls happen, never what's said. The other person never sees your address."}
          {risk === "high" && " In High risk mode Sotto reminds you of this before your first call."}
        </Notice>
      </form>
    </Section>
  );
}

function Row({ title, text, children }: { title: string; text: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6">
      <div>
        <div className="text-[14px] font-medium">{title}</div>
        <div className="text-[13px] text-muted-foreground">{text}</div>
      </div>
      {children}
    </div>
  );
}

function PasscodeDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const toast = useToast();
  const [oldP, setOld] = useState("");
  const [newP, setNew] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const close = () => {
    setOld("");
    setNew("");
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title="Change passcode">
      <form
        className="flex flex-col gap-6"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await post("/api/passcode", { old: oldP, new: newP });
            toast("success", "Passcode changed.");
            close();
          } catch (err) {
            setError((err as Error).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label="Current passcode" type="password" autoComplete="current-password" value={oldP} onChange={(e) => setOld(e.target.value)} autoFocus />
        <Field label="New passcode" type="password" autoComplete="new-password" value={newP} onChange={(e) => setNew(e.target.value)} error={error} hint="At least 8 characters." />
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" loading={busy} disabled={!oldP || newP.length < 8}>
            Change passcode
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function DuressDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const toast = useToast();
  const [pass, setPass] = useState("");
  const [duress, setDuress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setPass("");
    setDuress("");
    setError(null);
    onClose();
  };
  async function submit(remove: boolean) {
    setError(null);
    try {
      await post("/api/duress", { passcode: pass, duress: remove ? null : duress });
      toast("success", remove ? "Duress passcode removed." : "Duress passcode set.");
      close();
    } catch (err) {
      setError((err as Error).message);
    }
  }
  return (
    <Dialog open={open} onClose={close} title="Duress passcode" description="Entering it at the unlock screen opens an empty Sotto and permanently destroys your real data. Nobody can tell from the outside whether one is set.">
      <div className="flex flex-col gap-6">
        <Field label="Your passcode" type="password" value={pass} onChange={(e) => setPass(e.target.value)} autoFocus />
        <Field label="New duress passcode" type="password" value={duress} onChange={(e) => setDuress(e.target.value)} error={error} hint="Leave empty and choose Remove to turn it off." />
        <div className="flex justify-between gap-2">
          <Button variant="danger-outline" disabled={!pass} onClick={() => submit(true)}>
            Remove
          </Button>
          <Button variant="primary" disabled={!pass || duress.length < 8} onClick={() => submit(false)}>
            Set duress passcode
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function TravelDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [words, setWords] = useState<string[] | null>(null);
  const [step, setStep] = useState<"intro" | "words" | "check">("intro");
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [busy, setBusy] = useState(false);
  const picks = [2, 11, 20];
  const close = () => {
    setWords(null);
    setStep("intro");
    setAnswers({});
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title="Travel mode" width={560}>
      {step === "intro" && (
        <div className="flex flex-col gap-4">
          <ol className="flex list-decimal flex-col gap-2 pl-5 text-[13.5px] text-muted-foreground">
            <li>Sotto shows you a 24-word recovery phrase. Write it on paper and keep it apart from the backup.</li>
            <li>Everything is sealed into one backup file you save somewhere off this device.</li>
            <li>This device is erased. Messages wait on relays for up to 14 days.</li>
          </ol>
          <Button
            variant="primary"
            loading={busy}
            onClick={async () => {
              setBusy(true);
              try {
                const r = await post<{ words: string }>("/api/travel/prepare");
                setWords(r.words.split(" "));
                setStep("words");
              } catch (e) {
                toast("error", (e as Error).message);
              } finally {
                setBusy(false);
              }
            }}
          >
            Show my recovery phrase
          </Button>
        </div>
      )}
      {step === "words" && words && (
        <div className="flex flex-col gap-4">
          <ol className="grid grid-cols-3 gap-2">
            {words.map((w, i) => (
              <li key={i} className="flex items-baseline gap-2 rounded-lg bg-muted px-2.5 py-1.5">
                <span className="w-5 text-[11px] text-muted-foreground tabular">{i + 1}</span>
                <span className="font-mono text-[13.5px] font-medium">{w}</span>
              </li>
            ))}
          </ol>
          <Notice tone="danger">Without these words the backup can never be opened, by you or anyone else.</Notice>
          <Button variant="primary" onClick={() => setStep("check")}>
            I've written them down
          </Button>
        </div>
      )}
      {step === "check" && words && (
        <form
          className="flex flex-col gap-5"
          onSubmit={async (e) => {
            e.preventDefault();
            const path = await chooseSavePath("sotto-travel-backup.bin");
            if (!path) return;
            setBusy(true);
            try {
              await post("/api/travel/start", { path, check: picks.map((i) => [i, answers[i] ?? ""]) });
              forgetAppearance();
              toast("success", "Backup saved and this device erased. Restore it from the setup screen when you're safe.");
              await qc.invalidateQueries();
            } catch (err) {
              toast("error", (err as Error).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          <p className="text-[13.5px] text-muted-foreground">Type three of the words to confirm you have them.</p>
          {picks.map((i) => (
            <Field key={i} label={`Word ${i + 1}`} value={answers[i] ?? ""} onChange={(e) => setAnswers({ ...answers, [i]: e.target.value })} autoComplete="off" spellCheck={false} />
          ))}
          <Button variant="danger" type="submit" loading={busy} disabled={picks.some((i) => !answers[i]?.trim())}>
            Save backup and erase this device
          </Button>
        </form>
      )}
    </Dialog>
  );
}

function ago(sec: number | null) {
  if (!sec) return "never used";
  const d = Date.now() / 1000 - sec;
  if (d < 60) return "used just now";
  if (d < 3600) return `used ${Math.round(d / 60)} min ago`;
  if (d < 86_400) return `used ${Math.round(d / 3600)} h ago`;
  return `used ${new Date(sec * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" })}`;
}

/** How a script reaches the integration: env vars, the CLI, and plain curl. */
function hookUsage(h: HookView) {
  const url = h.url ?? `<address of this Sotto app>/hooks/${h.id}`;
  const token = h.token ?? "<the token you copied>";
  const curl = url.startsWith("unix:")
    ? `curl --unix-socket ${url.slice(5, url.lastIndexOf("/hooks/"))} -H "Authorization: Bearer $SOTTO_HOOK_TOKEN" --data "Deploy 412 finished" http://localhost${url.slice(url.lastIndexOf("/hooks/"))}`
    : `curl -H "Authorization: Bearer $SOTTO_HOOK_TOKEN" --data "Deploy 412 finished" "$SOTTO_HOOK_URL"`;
  return [
    "```sh",
    `export SOTTO_HOOK_URL=${url}`,
    `export SOTTO_HOOK_TOKEN=${token}`,
    "",
    "# with the sotto-hook command",
    'sotto-hook "Deploy 412 finished"',
    "make test 2>&1 | tail -20 | sotto-hook --code sh",
    "",
    "# or with curl",
    curl,
    "```",
  ].join("\n");
}

function IntegrationsSection() {
  const qc = useQueryClient();
  const toast = useToast();
  const hooks = useQuery({ queryKey: ["hooks"], queryFn: () => get<HookView[]>("/api/hooks") });
  const [creating, setCreating] = useState(false);
  const [removing, setRemoving] = useState<HookView | null>(null);
  const list = hooks.data ?? [];
  return (
    <Section
      id="settings-integrations"
      title="Integrations"
      description="Let scripts, CI jobs and monitoring post into a group or a channel. Each integration has its own token, and its posts are labelled so nobody mistakes them for a person."
    >
      <div className="flex flex-col gap-3">
        {list.length === 0 && !hooks.isPending && (
          <p className="text-[13px] text-muted-foreground">No integrations yet. Make one to post build results, alerts or reports from your own tools. They talk only to Sotto on this computer, which encrypts and sends them.</p>
        )}
        {list.length > 0 && (
          <ul className="flex flex-col divide-y divide-line rounded-xl bg-muted/40">
            {list.map((h) => (
              <li key={h.id} className="flex items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[14px] font-medium">{h.name}</div>
                  <div className="truncate text-[12.5px] text-muted-foreground">
                    Posts to {h.target.title ?? "a removed " + h.target.kind} · {h.posts} {h.posts === 1 ? "post" : "posts"}, {ago(h.last_used)}
                  </div>
                </div>
                <Button size="sm" variant="danger-outline" onClick={() => setRemoving(h)}>
                  Remove
                </Button>
              </li>
            ))}
          </ul>
        )}
        <div>
          <Button icon={<Plus size={15} />} onClick={() => setCreating(true)}>
            New integration
          </Button>
        </div>
      </div>
      <NewHookDialog open={creating} onClose={() => setCreating(false)} />
      <Dialog open={!!removing} onClose={() => setRemoving(null)} title={`Remove ${removing?.name ?? "this integration"}?`} description="Scripts using its token stop working straight away. Messages it already posted stay.">
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setRemoving(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={async () => {
              const h = removing;
              setRemoving(null);
              if (!h) return;
              await del(`/api/hooks/${h.id}`).then(
                () => qc.invalidateQueries({ queryKey: ["hooks"] }),
                (e: ApiError) => toast("error", e.message),
              );
            }}
          >
            Remove integration
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

function NewHookDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const qc = useQueryClient();
  const groups = useQuery({ queryKey: ["groups"], queryFn: () => get<GroupView[]>("/api/groups"), enabled: open });
  const channels = useQuery({ queryKey: ["channels"], queryFn: () => get<ChannelView[]>("/api/channels"), enabled: open });
  const targets = [
    ...(groups.data ?? []).filter((g) => !g.removed).map((g) => ({ kind: "group" as const, id: g.id, title: g.title })),
    ...(channels.data ?? []).filter((c) => c.owner).map((c) => ({ kind: "channel" as const, id: c.id, title: c.title })),
  ];
  const [name, setName] = useState("");
  const [target, setTarget] = useState<string | null>(null);
  const [made, setMade] = useState<HookView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setName("");
    setTarget(null);
    setMade(null);
    setError(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} title={made ? `${made.name} is ready` : "New integration"} description={made ? "Copy the token now. Sotto keeps only a fingerprint of it, so it can't be shown again." : "Name it after what will post, for example CI alerts or Uptime."} width={made ? 620 : 480}>
      {made ? (
        <div className="flex flex-col gap-4">
          <div className="text-[13px]">
            <Rich text={hookUsage(made)} />
          </div>
          <p className="text-[12.5px] leading-relaxed text-muted-foreground">
            Keep the token in your CI's secret store, not in code. Posts work while Sotto is open and unlocked, up to 30 a minute. Build the command with <code className="font-mono">cargo build --release -p sotto-hook</code>, or use curl.
          </p>
          <div className="flex justify-end">
            <Button variant="primary" onClick={close}>
              Done
            </Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-5"
          onSubmit={async (e) => {
            e.preventDefault();
            const t = targets.find((x) => x.id === target);
            if (!t) return;
            setBusy(true);
            setError(null);
            try {
              const h = await post<HookView>("/api/hooks", { name, kind: t.kind, id: t.id });
              await qc.invalidateQueries({ queryKey: ["hooks"] });
              setMade(h);
            } catch (err) {
              setError((err as ApiError).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} autoFocus placeholder="For example, CI alerts" maxLength={40} />
          <div>
            <div className="text-[13px] font-medium">Posts to</div>
            {targets.length === 0 ? (
              <p className="mt-1.5 text-[13px] text-muted-foreground">Make a group, or a channel of your own, first.</p>
            ) : (
              <ul className="scroll-pane mt-2 flex max-h-56 flex-col rounded-xl bg-muted p-1" role="radiogroup" aria-label="Posts to">
                {targets.map((t) => (
                  <li key={t.id}>
                    <button
                      type="button"
                      role="radio"
                      aria-checked={target === t.id}
                      onClick={() => setTarget(t.id)}
                      className={`flex h-10 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13.5px] transition-colors duration-150 ${target === t.id ? "bg-[color-mix(in_srgb,var(--primary)_12%,transparent)]" : "hover:bg-pane"}`}
                    >
                      <span className="flex-1 truncate">{t.title}</span>
                      <span className="text-[12px] text-muted-foreground">{t.kind === "group" ? "Group" : "Your channel"}</span>
                      <span aria-hidden="true" className={`grid size-5 shrink-0 place-items-center rounded-full ${target === t.id ? "bg-primary text-primary-foreground" : "bg-muted-strong text-transparent"}`}>
                        <Check size={12} strokeWidth={3} />
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {error && <Notice tone="danger">{error}</Notice>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" loading={busy} disabled={!name.trim() || !target}>
              Create integration
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

/** Give the tag up (it's free for anyone after that, and unused invites are closed). */
function TagRelease() {
  const qc = useQueryClient();
  const toast = useToast();
  const tag = useQuery({ queryKey: ["tag"], queryFn: () => get<TagView>("/api/tag") });
  const [confirm, setConfirm] = useState(false);
  if (!tag.data?.tag) return null;
  return (
    <>
      <button type="button" onClick={() => setConfirm(true)} className="mt-2 cursor-pointer text-[12.5px] text-muted-foreground hover:text-destructive hover:underline">
        Release this tag
      </button>
      <Dialog open={confirm} onClose={() => setConfirm(false)} title={`Release @${tag.data.tag}?`} description="Nobody can reach you with it any more, and anyone can claim it after you. People you already talk to aren't affected.">
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setConfirm(false)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={async () => {
              setConfirm(false);
              await del("/api/tag").then(
                () => qc.invalidateQueries({ queryKey: ["tag"] }),
                (e: ApiError) => toast("error", e.message),
              );
            }}
          >
            Release tag
          </Button>
        </div>
      </Dialog>
    </>
  );
}
