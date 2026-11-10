import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { post } from "../api";
import { Mark } from "../brand/Mark";
import { SpotLocked } from "../brand/spots";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";

export function Unlock() {
  const qc = useQueryClient();
  const [pass, setPass] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [erase, setErase] = useState(false);

  async function unlock(e: React.FormEvent) {
    e.preventDefault();
    if (!pass) return;
    setBusy(true);
    setError(null);
    try {
      await post("/api/unlock", { passcode: pass });
      setPass("");
      await qc.invalidateQueries();
    } catch (err) {
      setError((err as Error).message);
      setPass("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="grid h-full place-items-center p-4">
      <div className="flex w-full max-w-[380px] flex-col items-center text-center">
        <SpotLocked />
        <div className="mt-3 flex items-center gap-2">
          <Mark size={22} />
          <span className="text-[14px] font-semibold">Sotto</span>
        </div>
        <h1 className="mt-3 text-[22px] font-semibold tracking-tight">Sotto is locked</h1>
        <p className="mt-1 text-[13.5px] text-muted-foreground">Your conversations and files stay encrypted until you enter your passcode.</p>
        <form onSubmit={unlock} className="mt-7 w-full text-left">
          <Field label="Passcode" type="password" autoFocus autoComplete="current-password" value={pass} onChange={(e) => setPass(e.target.value)} error={error} />
          <Button variant="primary" type="submit" className="mt-6 w-full" loading={busy} disabled={!pass}>
            {busy ? "Unlocking…" : "Unlock"}
          </Button>
        </form>
        <button type="button" onClick={() => setErase(true)} className="mt-8 cursor-pointer text-[12.5px] text-muted-foreground underline-offset-4 hover:text-destructive hover:underline">
          Emergency: erase Sotto from this device
        </button>
      </div>
      <EraseDialog open={erase} onClose={() => setErase(false)} />
    </div>
  );
}

export function EraseDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const qc = useQueryClient();
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      open={open}
      onClose={() => {
        setTyped("");
        onClose();
      }}
      title="Erase Sotto from this device?"
      description="Every conversation, file, case and key on this device is destroyed at once. This can't be undone, and nothing is kept anywhere else."
    >
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          try {
            await post("/api/panic");
            await qc.invalidateQueries();
            onClose();
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label='Type "erase" to confirm' value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" spellCheck={false} />
        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="danger" disabled={typed.trim().toLowerCase() !== "erase"} loading={busy}>
            Erase everything
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
