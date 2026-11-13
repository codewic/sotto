// The Sotto mark: a speech bubble with a hush line, on a 48 x 48 tile, two tones of violet,
// light from the top left. Built on a 4 px grid.

export function Mark({ size = 32, className = "" }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" className={`mark-hover ${className}`} aria-hidden="true">
      <rect width="48" height="48" rx="12" fill="var(--mark-deep)" />
      <path d="M0 12A12 12 0 0 1 12 0h24a12 12 0 0 1 12 12v4C30 14 14 22 0 30Z" fill="#fff" opacity="0.08" />
      <g className="mark-bubble">
        <path d="M14 12h20a5 5 0 0 1 5 5v11a5 5 0 0 1-5 5H23l-7 6v-6h-2a5 5 0 0 1-5-5V17a5 5 0 0 1 5-5Z" fill="var(--mark-light)" />
        <rect className="mark-hush" x="17" y="21.5" width="14" height="3" rx="1.5" fill="var(--mark-deep)" />
      </g>
    </svg>
  );
}

/** Loading: the bubble outline drawing itself, in the accent colour. No tile behind it. */
export function MarkIndicator({ size = 20, label, inherit = false }: { size?: number; label?: string; inherit?: boolean }) {
  return (
    <span role="status" aria-live="polite" className={`relative inline-flex items-center gap-2 ${inherit ? "" : "text-primary"}`}>
      <svg width={size} height={size} viewBox="0 0 48 48" fill="none" className="mark-draw" aria-hidden="true">
        <path
          d="M14 13h20a5 5 0 0 1 5 5v11a5 5 0 0 1-5 5H23l-7 6v-6h-2a5 5 0 0 1-5-5V18a5 5 0 0 1 5-5Z M17 23.5h14"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
      {label ? <span className="text-muted-foreground text-[13px]">{label}</span> : <span className="sr-only">Loading…</span>}
    </span>
  );
}
