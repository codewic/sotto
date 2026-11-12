import { Check, Copy } from "lucide-react";
import { type ReactNode, useState } from "react";

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded-md bg-muted px-1.5 font-sans text-[11px] font-medium text-muted-foreground">{children}</kbd>;
}

/** Status chips: no border, soft fill; the accent tint only for positive state. */
export function Chip({ tone = "neutral", children }: { tone?: "neutral" | "accent" | "danger"; children: ReactNode }) {
  const t =
    tone === "accent"
      ? "bg-[color-mix(in_srgb,var(--primary)_12%,transparent)] text-primary"
      : tone === "danger"
        ? "bg-[color-mix(in_srgb,var(--destructive)_10%,transparent)] text-destructive"
        : "bg-muted text-muted-foreground";
  return <span className={`inline-flex h-6 items-center gap-1 rounded-full px-2.5 text-[12px] font-medium whitespace-nowrap ${t}`}>{children}</span>;
}

export function Segmented<T extends string>({ value, onChange, options, label }: { value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="no-scrollbar inline-flex max-w-full overflow-x-auto rounded-full bg-muted p-1">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={`press h-8 shrink-0 cursor-pointer rounded-full px-3.5 text-[13px] font-medium whitespace-nowrap ${value === o.value ? "bg-pane text-foreground shadow-pane" : "text-muted-foreground hover:text-foreground"}`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`press relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full ${checked ? "bg-primary" : "bg-muted-strong"}`}
    >
      <span className={`inline-block size-5 rounded-full bg-pane shadow-sm transition-transform duration-200 ease-[var(--ease-smooth)] ${checked ? "translate-x-[18px]" : "translate-x-0.5"}`} />
    </button>
  );
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <div className={`skeleton rounded-lg ${className}`} aria-hidden="true" />;
}

/** Empty states teach: what is empty, why, and what to do next. */
export function Empty({ art, title, children, action }: { art: ReactNode; title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mx-auto flex max-w-[400px] flex-col items-center px-6 py-10 text-center">
      {art}
      <h3 className="mt-4 text-[16px] font-semibold tracking-tight">{title}</h3>
      <div className="mt-1.5 text-[13.5px] leading-relaxed text-muted-foreground">{children}</div>
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setDone(true);
        setTimeout(() => setDone(false), 1600);
      }}
      className="press inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-full bg-muted px-3 text-[13px] font-medium hover:bg-muted-strong"
    >
      {done ? <Check size={14} className="text-primary" /> : <Copy size={14} />}
      {done ? "Copied" : label}
    </button>
  );
}

/** A quiet inline notice. Neutral surface; a small glyph carries any colour. */
export function Notice({ tone = "neutral", children }: { tone?: "neutral" | "accent" | "danger"; children: ReactNode }) {
  const dot = tone === "accent" ? "bg-primary" : tone === "danger" ? "bg-destructive" : "bg-muted-foreground";
  return (
    <div className="flex items-start gap-2.5 rounded-xl bg-muted px-3.5 py-3 text-[13px] leading-relaxed text-muted-foreground">
      <span aria-hidden="true" className={`mt-[7px] size-1.5 shrink-0 rounded-full ${dot}`} />
      <div>{children}</div>
    </div>
  );
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

export function dayLabel(day: number): string {
  return new Date(day * 86_400_000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
}

export function timeLabel(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

export function ttlLabel(s: number | null): string {
  if (!s) return "Off";
  if (s % 86_400 === 0) return `${s / 86_400} day${s === 86_400 ? "" : "s"}`;
  if (s % 3600 === 0) return `${s / 3600} hour${s === 3600 ? "" : "s"}`;
  return `${Math.round(s / 60)} minutes`;
}
