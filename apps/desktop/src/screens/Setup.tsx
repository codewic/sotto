import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { useState } from "react";
import { post } from "../api";
import { Dialog } from "../ui/Dialog";
import { TextArea } from "../ui/Field";
import { Mark } from "../brand/Mark";
import { GlyphShield } from "../brand/glyphs";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { Notice, Segmented } from "../ui/bits";

type Step = "welcome" | "passcode" | "connection";

const protects = [
  ["No phone number, email or username", "Your identity is a set of keys on this device. No server knows who you are."],
  ["Nobody can map who talks to whom", "Each conversation uses its own one-way mailboxes on relays that can't see sender or receiver."],
  ["Everything travels over Tor", "Relays never learn your network location."],
  ["Files are opened in a sealed sandbox", "A hostile document can't reach this computer. You read a clean copy."],
];

const limits = [
  "Spyware already on this computer or phone.",
  "Details in the information itself that only a few people know.",
  "Someone watching both ends of the Tor network at once.",
];

export function Setup() {
  const qc = useQueryClient();
  const [step, setStep] = useState<Step>("welcome");
  const [pass, setPass] = useState("");
  const [confirm, setConfirm] = useState("");
  const [duressOn, setDuressOn] = useState(false);
  const [duress, setDuress] = useState("");
  const [kind, setKind] = useState<"tor" | "direct">("tor");
  const [socks, setSocks] = useState("127.0.0.1:9050");
  const [relay, setRelay] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [restore, setRestore] = useState(false);

  const passError =
    pass.length > 0 && pass.length < 8 ? "Use at least 8 characters." : confirm.length > 0 && confirm !== pass ? "The two passcodes don't match." : null;
  const duressError = duressOn && duress.length > 0 ? (duress.length < 8 ? "Use at least 8 characters." : duress === pass ? "This must differ from your passcode." : null) : null;
  const passReady = pass.length >= 8 && confirm === pass && (!duressOn || (duress.length >= 8 && duress !== pass));

  async function create() {
    setBusy(true);
    setError(null);
    try {
      await post("/api/setup", {
        passcode: pass,
        duress: duressOn ? duress : null,
        relay: relay.trim() || null,
        transport: kind === "tor" ? { kind: "tor", socks } : { kind: "direct" },
      });
      setPass("");
      setConfirm("");
      setDuress("");
      await qc.invalidateQueries();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="scroll-pane grid h-full place-items-center p-4">
      <div className="w-full max-w-[560px] rounded-2xl bg-pane p-8 shadow-pane sm:p-10">
        <div className="mb-8 flex items-center gap-3">
          <Mark size={36} />
          <span className="text-[15px] font-semibold tracking-tight">Sotto</span>
          <span className="ml-auto text-[12px] text-muted-foreground tabular">Step {step === "welcome" ? 1 : step === "passcode" ? 2 : 3} of 3</span>
        </div>

        {step === "welcome" && (
          <>
            <h1 className="text-[26px] leading-tight font-semibold tracking-tight">Work together without leaving a trail</h1>
            <p className="mt-2 text-[14.5px] text-muted-foreground">Messages, calls, files and shared projects for your team, end-to-end encrypted, with no accounts, phone numbers or emails tying any of it to you.</p>
            <ul className="mt-7 flex flex-col gap-4">
              {protects.map(([t, d]) => (
                <li key={t} className="flex gap-3">
                  <span className="mt-0.5 text-primary">
                    <GlyphShield />
                  </span>
                  <div>
                    <div className="text-[14px] font-medium">{t}</div>
                    <div className="text-[13.5px] text-muted-foreground">{d}</div>
                  </div>
                </li>
              ))}
            </ul>
            <details className="mt-6 rounded-xl bg-muted px-4 py-3">
              <summary className="cursor-pointer text-[13.5px] font-medium">What Sotto can't protect against</summary>
              <ul className="mt-2 flex list-disc flex-col gap-1 pl-5 text-[13.5px] text-muted-foreground">
                {limits.map((l) => (
                  <li key={l}>{l}</li>
                ))}
              </ul>
              <p className="mt-2 text-[13px] text-muted-foreground">If you think a device is compromised, use a different one.</p>
            </details>
            <div className="mt-8 flex items-center justify-between">
              <Button variant="ghost" onClick={() => setRestore(true)}>
                Restore from a travel backup
              </Button>
              <Button variant="primary" onClick={() => setStep("passcode")}>
                Set up this device
              </Button>
            </div>
          </>
        )}

        {step === "passcode" && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (passReady) setStep("connection");
            }}
          >
            <h1 className="text-[22px] font-semibold tracking-tight">Choose a passcode</h1>
            <p className="mt-1.5 text-[14px] text-muted-foreground">It unlocks Sotto on this device only. It's separate from your computer's password, and there is no way to recover it.</p>
            <div className="mt-7 flex flex-col gap-6">
              <Field
                label="Passcode"
                type="password"
                autoComplete="new-password"
                autoFocus
                value={pass}
                onChange={(e) => setPass(e.target.value)}
                hint="At least 8 characters. A short sentence is easy to remember and hard to guess."
              />
              <Field label="Type it again" type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} error={passError} />
            </div>
            <div className="mt-7 rounded-xl bg-muted p-4">
              <label className="flex cursor-pointer items-start gap-3">
                <input type="checkbox" className="mt-1 size-4 accent-[var(--primary)]" checked={duressOn} onChange={(e) => setDuressOn(e.target.checked)} />
                <span>
                  <span className="block text-[14px] font-medium">Add a duress passcode</span>
                  <span className="block text-[13px] text-muted-foreground">
                    If someone forces you to unlock Sotto, use this instead. It opens an empty Sotto and permanently destroys your real data.
                  </span>
                </span>
              </label>
              {duressOn && (
                <div className="mt-5">
                  <Field label="Duress passcode" type="password" autoComplete="off" value={duress} onChange={(e) => setDuress(e.target.value)} error={duressError} />
                </div>
              )}
            </div>
            <div className="mt-8 flex items-center justify-between">
              <Button type="button" variant="ghost" icon={<ArrowLeft size={16} />} onClick={() => setStep("welcome")}>
                Back
              </Button>
              <Button variant="primary" type="submit" disabled={!passReady}>
                Continue
              </Button>
            </div>
          </form>
        )}

        {step === "connection" && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void create();
            }}
          >
            <h1 className="text-[22px] font-semibold tracking-tight">Connect through Tor</h1>
            <p className="mt-1.5 text-[14px] text-muted-foreground">
              Sotto sends everything through Tor so relays never learn where you are. Tor Browser or the tor service must be running on this computer.
            </p>
            <div className="mt-6">
              <Segmented
                label="Connection"
                value={kind}
                onChange={setKind}
                options={[
                  { value: "tor", label: "Tor" },
                  { value: "direct", label: "Direct (testing only)" },
                ]}
              />
            </div>
            <div className="mt-6 flex flex-col gap-6">
              {kind === "tor" ? (
                <Field label="Tor address" value={socks} onChange={(e) => setSocks(e.target.value)} spellCheck={false} hint="The tor service uses 127.0.0.1:9050. Tor Browser uses 127.0.0.1:9150." />
              ) : (
                <Notice tone="danger">Direct connections reveal your network address to the relay. Use them only for testing on your own machine.</Notice>
              )}
              <Field
                label="Relay address"
                placeholder="ws://…onion"
                value={relay}
                onChange={(e) => setRelay(e.target.value)}
                spellCheck={false}
                hint="Your organization gives you this. Leave it empty to use the default. You can change it later in Settings."
              />
            </div>
            {error && (
              <div className="mt-6">
                <Notice tone="danger">{error}</Notice>
              </div>
            )}
            <div className="mt-8 flex items-center justify-between">
              <Button type="button" variant="ghost" icon={<ArrowLeft size={16} />} onClick={() => setStep("passcode")}>
                Back
              </Button>
              <Button variant="primary" type="submit" loading={busy}>
                {busy ? "Creating your vault…" : "Create my vault"}
              </Button>
            </div>
          </form>
        )}
      </div>
      <RestoreDialog open={restore} onClose={() => setRestore(false)} />
    </div>
  );
}

function RestoreDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const qc = useQueryClient();
  const [path, setPath] = useState("");
  const [words, setWords] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <Dialog open={open} onClose={onClose} title="Restore from a travel backup" description="Then unlock with the passcode you used before you travelled.">
      <form
        className="flex flex-col gap-5"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await post("/api/travel/restore", { path: path.trim(), words });
            await qc.invalidateQueries();
            onClose();
          } catch (err) {
            setError((err as Error).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label="Backup file" value={path} onChange={(e) => setPath(e.target.value)} placeholder="/media/usb/sotto-travel-backup.bin" spellCheck={false} />
        <TextArea label="Recovery phrase" rows={3} value={words} onChange={(e) => setWords(e.target.value)} spellCheck={false} autoComplete="off" error={error} hint="All 24 words, in order." />
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" loading={busy} disabled={!path.trim() || words.trim().split(/\s+/).length < 24}>
            Restore
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
