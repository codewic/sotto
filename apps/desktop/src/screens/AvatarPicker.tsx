import { useQuery, useQueryClient } from "@tanstack/react-query";
import { RotateCw, Shuffle } from "lucide-react";
import { useEffect, useState } from "react";
import { type ApiError, get, put } from "../api";
import { Avatar, HUE_COUNT, SHAPE_COUNT, choiceId, defaultChoice, hueOf, parseChoice } from "../ui/Avatar";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { useToast } from "../ui/toast";
import { useNav } from "./Shell";

/** Your own avatar choice (null: the default made from your identity). */
export function useMyAvatar() {
  const seed = useNav().status.profile?.fingerprint ?? "me";
  const q = useQuery({ queryKey: ["me-avatar"], queryFn: () => get<{ id: string | null }>("/api/me/avatar") });
  return { seed, choice: q.data?.id ?? null };
}

const SHAPE_NAMES = ["Orbit", "Peaks", "Tide", "Halves", "Stack", "Leaf", "Kite", "Arch", "Bloom", "Crescent", "Comet", "Cell", "Bolt", "Sprout", "Rings", "Dots", "Sail", "Prism", "Spark", "Steps"];

/** Your avatar in Settings, with the picker behind "Change". */
export function MyAvatar() {
  const { seed, choice } = useMyAvatar();
  const [open, setOpen] = useState(false);
  return (
    <div className="flex items-center gap-4">
      <Avatar seed={seed} choice={choice} size={64} label="Your avatar" />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium">Avatar</div>
        <p className="mt-0.5 text-[13px] text-muted-foreground">{choice ? "From the collection. People you talk to see it." : "Made for you from your identity. Pick another from the collection any time."}</p>
      </div>
      <Button size="sm" onClick={() => setOpen(true)}>
        Change
      </Button>
      <AvatarPicker open={open} onClose={() => setOpen(false)} seed={seed} current={choice} />
    </div>
  );
}

function AvatarPicker({ open, onClose, seed, current }: { open: boolean; onClose: () => void; seed: string; current: string | null }) {
  const qc = useQueryClient();
  const toast = useToast();
  const start = parseChoice(current ?? defaultChoice(seed)) ?? { shape: 0, hueIndex: 0, turn: 0 };
  const [shape, setShape] = useState(start.shape);
  const [hue, setHue] = useState(start.hueIndex);
  const [turn, setTurn] = useState(start.turn);
  const [busy, setBusy] = useState(false);
  // Start from what's shown each time the picker opens.
  useEffect(() => {
    if (!open) return;
    const c = parseChoice(current ?? defaultChoice(seed)) ?? { shape: 0, hueIndex: 0, turn: 0 };
    setShape(c.shape);
    setHue(c.hueIndex);
    setTurn(c.turn);
  }, [open, current, seed]);
  const id = choiceId(shape, hue, turn);
  const save = async (value: string | null) => {
    setBusy(true);
    try {
      await put("/api/me/avatar", { id: value });
      await qc.invalidateQueries({ queryKey: ["me-avatar"] });
      onClose();
    } catch (e) {
      toast("error", (e as ApiError).message);
    } finally {
      setBusy(false);
    }
  };
  const shuffle = () => {
    const r = crypto.getRandomValues(new Uint32Array(3));
    setShape(r[0] % SHAPE_COUNT);
    setHue(r[1] % HUE_COUNT);
    setTurn(r[2] % 4);
  };
  return (
    <Dialog open={open} onClose={onClose} title="Choose your avatar" description={`${SHAPE_COUNT} shapes, ${HUE_COUNT} colours, four turns each. Only your choice is sent to the people you talk to, never an image.`} width={560}>
      <div className="flex flex-col gap-5">
        <div className="flex items-center gap-5">
          <Avatar seed={seed} choice={id} size={96} label="Preview" />
          <div className="flex flex-col gap-2">
            <div className="text-[15px] font-semibold">{SHAPE_NAMES[shape] ?? "Shape"}</div>
            <div className="flex gap-2">
              <Button size="sm" icon={<RotateCw size={14} />} onClick={() => setTurn((t) => (t + 1) % 4)}>
                Turn
              </Button>
              <Button size="sm" icon={<Shuffle size={14} />} onClick={shuffle}>
                Surprise me
              </Button>
            </div>
          </div>
        </div>
        <div>
          <div className="text-[13px] font-medium">Colour</div>
          <div className="mt-2 flex flex-wrap gap-2" role="radiogroup" aria-label="Colour">
            {Array.from({ length: HUE_COUNT }, (_, i) => (
              <button
                key={i}
                type="button"
                role="radio"
                aria-checked={hue === i}
                aria-label={`Colour ${i + 1}`}
                onClick={() => setHue(i)}
                className={`press size-8 cursor-pointer rounded-full transition-[box-shadow] duration-150 ${hue === i ? "shadow-[0_0_0_2px_var(--pane),0_0_0_4px_var(--foreground)]" : ""}`}
                style={{ background: `oklch(var(--av-mid-l) var(--av-mid-c) ${hueOf(i)})` }}
              />
            ))}
          </div>
        </div>
        <div>
          <div className="text-[13px] font-medium">Shape</div>
          <div className="mt-2 grid grid-cols-5 gap-2 sm:grid-cols-10" role="radiogroup" aria-label="Shape">
            {Array.from({ length: SHAPE_COUNT }, (_, i) => (
              <button
                key={i}
                type="button"
                role="radio"
                aria-checked={shape === i}
                aria-label={SHAPE_NAMES[i]}
                title={SHAPE_NAMES[i]}
                onClick={() => setShape(i)}
                className={`press grid cursor-pointer place-items-center rounded-xl p-1 transition-colors duration-150 ${shape === i ? "bg-[color-mix(in_srgb,var(--primary)_14%,transparent)]" : "hover:bg-muted"}`}
              >
                <Avatar seed={seed} choice={choiceId(i, hue, turn)} size={40} />
              </button>
            ))}
          </div>
        </div>
        <div className="flex items-center justify-between gap-2">
          <Button variant="ghost" disabled={busy || !current} onClick={() => void save(null)}>
            Use my default
          </Button>
          <div className="flex gap-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" loading={busy} onClick={() => void save(id)}>
              Use this avatar
            </Button>
          </div>
        </div>
      </div>
    </Dialog>
  );
}
