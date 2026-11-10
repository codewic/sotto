import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { chooseSavePath, patch, post } from "../api";
import { Mark } from "../brand/Mark";
import type { RiskMode } from "../types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { Chip, CopyButton, Notice, Segmented, Switch } from "../ui/bits";
import { useToast } from "../ui/toast";
import { EraseDialog } from "./Unlock";
import { PaneHeader, useNav } from "./Shell";

const modes: Record<RiskMode, { title: string; lines: string[] }> = {
  standard: { title: "Standard", lines: ["Tor, end-to-end encryption, disappearing messages", "Locks after the time you choose"] },
  high: { title: "High risk", lines: ["Adds cover traffic so your activity is harder to time", "Locks after one idle minute"] },
  extreme: { title: "Extreme", lines: ["Everything in High risk", "Keep this case on this computer only; review documents here, never on a phone"] },
};

export function Settings() {
  const qc = useQueryClient();
  const toast = useToast();
  const { status } = useNav();
  const s = status.settings!;
  const p = status.profile!;
  const [name, setName] = useState(p.display_name);
  const [relay, setRelay] = useState(p.default_relay);
  const [socks, setSocks] = useState(p.transport.kind === "tor" ? p.transport.socks : "127.0.0.1:9050");
  const [dialog, setDialog] = useState<"passcode" | "duress" | "erase" | "travel" | null>(null);

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
      <PaneHeader title="Settings" />
      <div className="scroll-pane flex-1">
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

          <Section title="Identity" description="Sources and colleagues never see an account. This is how you appear when you choose to share a name.">
            <form
              className="flex flex-col gap-6"
              onSubmit={(e) => {
                e.preventDefault();
                void save({ display_name: name }, "Name saved.");
              }}
            >
              <Field label="Display name" value={name} onChange={(e) => setName(e.target.value)} hint="Only shared with people you invite, and only if you choose to." />
              <div>
                <div className="text-[13px] font-medium">Newsroom identity fingerprint</div>
                <div className="mt-1.5 flex items-center gap-3">
                  <code className="rounded-lg bg-muted px-2.5 py-1.5 font-mono text-[12.5px] tabular">{p.fingerprint}</code>
                  <CopyButton text={p.fingerprint} />
                </div>
                <p className="mt-1.5 text-[13px] text-muted-foreground">Your outlet publishes this on its website so sources can check they're really talking to you.</p>
              </div>
              <div>
                <Button type="submit" size="sm">
                  Save name
                </Button>
              </div>
            </form>
          </Section>

          <Section title="Connection" description="Changes apply the next time you unlock Sotto.">
            <form
              className="flex flex-col gap-6"
              onSubmit={(e) => {
                e.preventDefault();
                void save({ default_relay: relay, transport: { kind: "tor", socks } }, "Connection saved. Lock and unlock Sotto to use it.");
              }}
            >
              <Field label="Tor address" value={socks} onChange={(e) => setSocks(e.target.value)} spellCheck={false} hint="tor service: 127.0.0.1:9050 · Tor Browser: 127.0.0.1:9150" />
              <Field label="Relay address" value={relay} onChange={(e) => setRelay(e.target.value)} spellCheck={false} hint="New conversations use this relay. Existing ones keep theirs." />
              {p.transport.kind === "direct" && <Notice tone="danger">You're connecting directly, without Tor. Relays can see your network address. Save a Tor address to fix this.</Notice>}
              <div className="flex flex-wrap gap-2">
                {(status.relays ?? []).map((r) => (
                  <Chip key={r.url} tone={r.status === "online" ? "accent" : r.status === "offline" ? "danger" : "neutral"}>
                    {r.status === "online" ? "Online" : r.status === "offline" ? "Unreachable" : "Connecting"} · {r.url.replace("ws://", "").slice(0, 28)}
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

          <Section title="Drops" description="How submissions are handled once they reach this desk.">
            <Row title="Delete from the drop after download" text="The server keeps nothing once your desk has it. Turn off only if colleagues also read this drop.">
              <Switch checked={s.delete_after_download} onChange={(v) => save({ delete_after_download: v })} label="Delete from the drop after download" />
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
                <p className="mt-1">Source protection for journalism. Open source, no analytics, no telemetry. Built by Vorynza.</p>
                <p className="mt-2">
                  What Sotto can't protect against: spyware on your devices, details in the information that only a few people know, and someone watching both ends of the Tor network at once.
                </p>
              </div>
            </div>
          </Section>
        </div>
      </div>
      <PasscodeDialog open={dialog === "passcode"} onClose={() => setDialog(null)} />
      <DuressDialog open={dialog === "duress"} onClose={() => setDialog(null)} />
      <EraseDialog open={dialog === "erase"} onClose={() => setDialog(null)} />
      <TravelDialog open={dialog === "travel"} onClose={() => setDialog(null)} />
    </>
  );
}

function Section({ title, description, children }: { title: string; description: string; children: React.ReactNode }) {
  return (
    <section className="grid grid-cols-1 gap-4 border-b border-line pb-10 last:border-0 md:grid-cols-[220px_1fr] md:gap-8">
      <div>
        <h2 className="text-[14.5px] font-semibold tracking-tight">{title}</h2>
        {description && <p className="mt-1 text-[13px] text-muted-foreground">{description}</p>}
      </div>
      <div>{children}</div>
    </section>
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
