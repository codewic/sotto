import { Eye, EyeOff } from "lucide-react";
import { type InputHTMLAttributes, type TextareaHTMLAttributes, useId, useState } from "react";

// House style: underline inputs with an animated accent line on focus.
// Every password field gets a show/hide toggle automatically.

const input =
  "peer w-full border-0 border-b border-line bg-transparent px-0.5 pt-1 pb-2 text-[15px] text-foreground placeholder:text-muted-foreground/80 outline-none hover:border-foreground/40 disabled:opacity-60";

function Underline() {
  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute right-0 bottom-0 left-0 h-0.5 origin-left scale-x-0 bg-primary transition-transform duration-300 ease-[var(--ease-smooth)] peer-focus:scale-x-100"
    />
  );
}

function Shell({ id, label, hint, error, children }: { id: string; label: string; hint?: string; error?: string | null; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-[13px] font-medium text-foreground">
        {label}
      </label>
      {children}
      {error ? (
        <p id={`${id}-error`} role="alert" className="flex items-center gap-1.5 text-[13px] text-destructive">
          <span aria-hidden="true" className="size-1.5 shrink-0 rounded-full bg-destructive" />
          {error}
        </p>
      ) : hint ? (
        <p id={`${id}-hint`} className="text-[13px] text-muted-foreground">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export function Field({ label, hint, error, type = "text", className = "", ...rest }: InputHTMLAttributes<HTMLInputElement> & { label: string; hint?: string; error?: string | null }) {
  const id = useId();
  const [shown, setShown] = useState(false);
  const isPassword = type === "password";
  return (
    <Shell id={id} label={label} hint={hint} error={error}>
      <div className="relative">
        <input
          {...rest}
          id={id}
          type={isPassword && shown ? "text" : type}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${id}-error` : hint ? `${id}-hint` : undefined}
          className={`${input} ${isPassword ? "pr-9" : ""} ${className}`}
        />
        <Underline />
        {isPassword && (
          <button
            type="button"
            onClick={() => setShown((s) => !s)}
            aria-label={shown ? "Hide passcode" : "Show passcode"}
            aria-pressed={shown}
            className="press absolute top-0 right-0 inline-flex size-8 cursor-pointer items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            {shown ? <EyeOff size={16} /> : <Eye size={16} />}
          </button>
        )}
      </div>
    </Shell>
  );
}

export function TextArea({ label, hint, error, className = "", ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement> & { label: string; hint?: string; error?: string | null }) {
  const id = useId();
  return (
    <Shell id={id} label={label} hint={hint} error={error}>
      <div className="relative">
        <textarea
          {...rest}
          id={id}
          aria-invalid={error ? true : undefined}
          className={`${input} resize-none leading-relaxed ${className}`}
        />
        <Underline />
      </div>
    </Shell>
  );
}
