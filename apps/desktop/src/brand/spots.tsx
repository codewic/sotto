// Spot illustrations for empty, onboarding and success states. Decorative only:
// the heading and text around them carry the message. Same shapes and tones as the mark.

const S = 136;

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <svg width={S} height={S} viewBox="0 0 136 136" fill="none" aria-hidden="true">
      <circle cx="68" cy="72" r="54" fill="var(--muted)" />
      {children}
    </svg>
  );
}

/** No conversations yet: two bubbles, not yet connected. */
export function SpotConversations() {
  return (
    <Frame>
      <g className="spot-float">
        <path d="M30 44h40a8 8 0 0 1 8 8v18a8 8 0 0 1-8 8H48l-11 9v-9h-7a8 8 0 0 1-8-8V52a8 8 0 0 1 8-8Z" fill="var(--mark-light)" />
        <rect x="34" y="59" width="30" height="4" rx="2" fill="var(--mark-deep)" />
      </g>
      <g className="spot-drift">
        <path d="M74 70h32a7 7 0 0 1 7 7v14a7 7 0 0 1-7 7h-4v8l-9-8H74a7 7 0 0 1-7-7V77a7 7 0 0 1 7-7Z" fill="var(--mark-deep)" />
        <circle cx="82" cy="84" r="2.5" fill="var(--mark-light)" />
        <circle cx="90" cy="84" r="2.5" fill="var(--mark-light)" />
        <circle cx="98" cy="84" r="2.5" fill="var(--mark-light)" />
      </g>
    </Frame>
  );
}

/** Empty drop: a tray with an envelope hovering above it. */
export function SpotDrop() {
  return (
    <Frame>
      <path d="M28 82h22l5 9h26l5-9h22v16a8 8 0 0 1-8 8H36a8 8 0 0 1-8-8Z" fill="var(--mark-deep)" />
      <path d="M28 82 37 58h62l9 24" stroke="var(--mark-mid)" strokeWidth="4" strokeLinejoin="round" />
      <g className="spot-float">
        <rect x="46" y="30" width="44" height="30" rx="5" fill="var(--mark-light)" />
        <path d="m48 33 20 14 20-14" stroke="var(--mark-deep)" strokeWidth="3.5" strokeLinecap="round" strokeLinejoin="round" />
      </g>
    </Frame>
  );
}

/** Safe review: a document held inside a dashed sandbox. */
export function SpotSandbox() {
  return (
    <Frame>
      <rect x="30" y="34" width="76" height="76" rx="14" stroke="var(--mark-mid)" strokeWidth="3" strokeDasharray="7 7" className="spot-pulse" />
      <g className="spot-float">
        <path d="M52 46h22l12 12v36a5 5 0 0 1-5 5H52a5 5 0 0 1-5-5V51a5 5 0 0 1 5-5Z" fill="var(--mark-light)" />
        <path d="M74 46v12h12" fill="var(--mark-mid)" />
        <rect x="54" y="68" width="24" height="4" rx="2" fill="var(--mark-deep)" />
        <rect x="54" y="78" width="18" height="4" rx="2" fill="var(--mark-deep)" opacity="0.6" />
      </g>
    </Frame>
  );
}

/** Locked: a key resting against a padlock. */
export function SpotLocked() {
  return (
    <Frame>
      <rect x="42" y="62" width="52" height="40" rx="10" fill="var(--mark-deep)" />
      <path d="M52 62V52a16 16 0 0 1 32 0v10" stroke="var(--mark-mid)" strokeWidth="7" strokeLinecap="round" />
      <g className="spot-pulse">
        <rect x="64" y="74" width="8" height="16" rx="4" fill="var(--mark-light)" />
      </g>
    </Frame>
  );
}

/** Cases: a folder with pages lifting out of it. */
export function SpotCases() {
  return (
    <Frame>
      <g className="spot-float">
        <rect x="48" y="38" width="40" height="50" rx="5" fill="var(--mark-light)" transform="rotate(-6 68 63)" />
        <rect x="56" y="48" width="22" height="4" rx="2" fill="var(--mark-deep)" transform="rotate(-6 68 63)" />
      </g>
      <path d="M28 60a6 6 0 0 1 6-6h22l7 7h39a6 6 0 0 1 6 6v33a6 6 0 0 1-6 6H34a6 6 0 0 1-6-6Z" fill="var(--mark-deep)" />
    </Frame>
  );
}

/** Done: a bubble with a check, used for success moments. */
export function SpotDone() {
  return (
    <Frame>
      <g className="spot-float">
        <path d="M40 40h56a10 10 0 0 1 10 10v26a10 10 0 0 1-10 10H66l-14 12V86H40a10 10 0 0 1-10-10V50a10 10 0 0 1 10-10Z" fill="var(--mark-deep)" />
        <path d="m54 63 9 9 18-18" stroke="var(--mark-light)" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" />
      </g>
    </Frame>
  );
}
