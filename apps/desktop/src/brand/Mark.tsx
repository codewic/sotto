// The Sotto mark: a message bubble with a padlock shackle and three quiet dots. Freestanding, no tile.
// Copied exactly from brand/marks/sotto-light.svg and sotto-dark.svg; the two differ only in the bubble
// body, which follows the scheme through --sotto-mark-body. The colours never follow the chosen accent.

export function Mark({ size = 32, className = "" }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" className={`mark-hover ${className}`} aria-hidden="true">
      <path className="mark-shackle" d="M17 21v-5a7 7 0 0 1 14 0v5" fill="none" stroke="#8F7FFF" strokeWidth="4.5" strokeLinecap="round" />
      <g className="mark-bubble">
        <path d="M9 23a4 4 0 0 1 4-4h22a4 4 0 0 1 4 4v12a4 4 0 0 1-4 4H22l-7 5.5V39h-2a4 4 0 0 1-4-4z" fill="var(--sotto-mark-body)" />
        <g fill="#C9BFFF">
          <circle cx="17.5" cy="29" r="2" />
          <circle cx="24" cy="29" r="2" />
          <circle cx="30.5" cy="29" r="2" />
        </g>
      </g>
    </svg>
  );
}

/** Loading: the mark's outline (shackle and bubble) drawing itself, in the accent colour. */
export function MarkIndicator({ size = 20, label, inherit = false }: { size?: number; label?: string; inherit?: boolean }) {
  return (
    <span role="status" aria-live="polite" className={`relative inline-flex items-center gap-2 ${inherit ? "" : "text-primary"}`}>
      {/* pathLength lets both strokes draw over the same span as the 120 dash in styles.css. */}
      <svg
        width={size}
        height={size}
        viewBox="0 0 48 48"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="mark-draw"
        aria-hidden="true"
      >
        <path pathLength={120} d="M17 19v-3a7 7 0 0 1 14 0v3" />
        <path pathLength={120} d="M9 23a4 4 0 0 1 4-4h22a4 4 0 0 1 4 4v12a4 4 0 0 1-4 4H22l-7 5.5V39h-2a4 4 0 0 1-4-4z" />
      </svg>
      {label ? <span className="text-muted-foreground text-[13px]">{label}</span> : <span className="sr-only">Loading…</span>}
    </span>
  );
}
