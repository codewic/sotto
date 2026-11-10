// Navigation glyphs: 20 x 20, 1.6 stroke, one currentColor fill at about 18% opacity.
// Each moves one meaningful part on hover (see styles.css).

type P = { className?: string };
const base = { width: 20, height: 20, viewBox: "0 0 20 20", fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };

export function GlyphConversations({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <path d="M3.5 5.5a2 2 0 0 1 2-2h7a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H8l-3 2.5v-2.5h0a2 2 0 0 1-1.5-2Z" fill="currentColor" fillOpacity="0.18" />
      <g className="g-lift">
        <path d="M14.5 8h.5a1.5 1.5 0 0 1 1.5 1.5v3.5a1.5 1.5 0 0 1-1.5 1.5h-.5v2l-2.5-2h-2.5" />
      </g>
    </svg>
  );
}

export function GlyphDrop({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <path d="M3 11.5h3.5l1.2 2h4.6l1.2-2H17v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" fill="currentColor" fillOpacity="0.18" />
      <path d="M3 11.5 5 5.5h10l2 6" />
      <g className="g-nudge-down">
        <path d="M10 2.5v6M7.8 6.5 10 8.7l2.2-2.2" />
      </g>
    </svg>
  );
}

export function GlyphCases({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <path d="M2.8 6.5V15a1.5 1.5 0 0 0 1.5 1.5h11.4a1.5 1.5 0 0 0 1.5-1.5V8a1.5 1.5 0 0 0-1.5-1.5Z" fill="currentColor" fillOpacity="0.18" />
      <g className="g-lift">
        <path d="M2.8 6.5V5a1.5 1.5 0 0 1 1.5-1.5h3.4l1.6 1.8h5" />
      </g>
      <path d="M8.5 11.2h3" />
    </svg>
  );
}

export function GlyphReview({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <path d="M5 2.5h6l3.5 3.5v5" />
      <path d="M5 2.5a1.5 1.5 0 0 0-1.5 1.5v12a1.5 1.5 0 0 0 1.5 1.5h4" fill="currentColor" fillOpacity="0.18" />
      <path d="M11 2.5V6h3.5" />
      <g className="g-nudge-right">
        <circle cx="13" cy="13.5" r="2.6" fill="currentColor" fillOpacity="0.18" />
        <path d="m15 15.5 2 2" />
      </g>
    </svg>
  );
}

export function GlyphSettings({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <g className="g-turn">
        <path
          d="M10 2.8l1.2 1.6 2-.4.5 1.9 1.9.5-.4 2L16.8 10l-1.6 1.2.4 2-1.9.5-.5 1.9-2-.4L10 17.2l-1.2-1.6-2 .4-.5-1.9-1.9-.5.4-2L3.2 10l1.6-1.2-.4-2 1.9-.5.5-1.9 2 .4Z"
          fill="currentColor"
          fillOpacity="0.18"
        />
      </g>
      <circle cx="10" cy="10" r="2.4" />
    </svg>
  );
}

export function GlyphShield({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <path d="M10 2.5 16 5v4.6c0 3.6-2.6 6.4-6 7.9-3.4-1.5-6-4.3-6-7.9V5Z" fill="currentColor" fillOpacity="0.18" />
      <g className="g-tilt">
        <path d="m7.4 10 1.8 1.8 3.6-3.6" />
      </g>
    </svg>
  );
}

export function GlyphLock({ className }: P) {
  return (
    <svg {...base} className={className} aria-hidden="true">
      <rect x="4.5" y="8.5" width="11" height="8" rx="2" fill="currentColor" fillOpacity="0.18" />
      <g className="g-lift">
        <path d="M7 8.5V6.5a3 3 0 0 1 6 0v2" />
      </g>
    </svg>
  );
}
