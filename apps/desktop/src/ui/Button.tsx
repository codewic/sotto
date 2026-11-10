import type { ButtonHTMLAttributes, ReactNode } from "react";
import { MarkIndicator } from "../brand/Mark";

type Variant = "primary" | "secondary" | "ghost" | "danger-outline" | "danger";
type Size = "sm" | "md";

const variants: Record<Variant, string> = {
  primary: "bg-primary text-primary-foreground hover:bg-[color-mix(in_srgb,var(--primary)_88%,var(--foreground))]",
  secondary: "bg-muted text-foreground hover:bg-muted-strong",
  ghost: "bg-transparent text-muted-foreground hover:bg-muted hover:text-foreground",
  // Destructive entry points are outline-style with destructive text; solid only on the final confirm.
  "danger-outline":
    "bg-transparent text-destructive shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--destructive)_45%,transparent)] hover:bg-[color-mix(in_srgb,var(--destructive)_8%,transparent)]",
  danger: "bg-destructive text-destructive-foreground hover:opacity-90",
};

const sizes: Record<Size, string> = {
  sm: "h-8 px-3 text-[13px] gap-1.5",
  md: "h-10 px-4 text-sm gap-2",
};

export function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  icon,
  children,
  className = "",
  disabled,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: Size; loading?: boolean; icon?: ReactNode }) {
  return (
    <button
      {...rest}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      className={`press inline-flex shrink-0 cursor-pointer items-center justify-center rounded-full font-medium whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50 ${variants[variant]} ${sizes[size]} ${className}`}
    >
      {loading ? <MarkIndicator size={16} inherit /> : icon}
      {children}
    </button>
  );
}

export function IconButton({ label, children, className = "", ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      {...rest}
      aria-label={label}
      title={label}
      className={`press inline-flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40 ${className}`}
    >
      {children}
    </button>
  );
}
