import { createContext, type ReactNode, useCallback, useContext, useState } from "react";

// Toasts sit on a neutral surface; only the small status glyph carries colour.

type Kind = "info" | "success" | "error";
type Toast = { id: number; kind: Kind; text: string };

const Ctx = createContext<(kind: Kind, text: string) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const push = useCallback((kind: Kind, text: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t.slice(-2), { id, kind, text }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 7000 : 4000);
  }, []);
  return (
    <Ctx.Provider value={push}>
      {children}
      <div className="pointer-events-none fixed right-4 bottom-4 flex w-[360px] max-w-[calc(100vw-32px)] flex-col gap-2" style={{ zIndex: "var(--z-toast)" }} aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className="pointer-events-auto flex items-start gap-3 rounded-xl bg-pane px-4 py-3 text-[13.5px] shadow-float" style={{ animation: "toast-in 220ms var(--ease-smooth)" }} role={t.kind === "error" ? "alert" : "status"}>
            <StatusGlyph kind={t.kind} />
            <span className="leading-snug">{t.text}</span>
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}

export function StatusGlyph({ kind }: { kind: Kind }) {
  const color = kind === "error" ? "var(--destructive)" : kind === "success" ? "var(--primary)" : "var(--muted-foreground)";
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" className="mt-0.5 shrink-0" aria-hidden="true">
      <circle cx="8" cy="8" r="7" fill={color} opacity="0.16" />
      {kind === "success" && <path d="m5 8.2 2 2 4-4.2" stroke={color} strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round" />}
      {kind === "error" && <path d="M8 4.6v4M8 11.2v.2" stroke={color} strokeWidth="1.8" strokeLinecap="round" />}
      {kind === "info" && <path d="M8 7.4v4M8 4.8v.2" stroke={color} strokeWidth="1.8" strokeLinecap="round" />}
    </svg>
  );
}

export const useToast = () => useContext(Ctx);
