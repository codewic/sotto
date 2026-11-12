// Chat wallpapers, hand-built in SVG. Every one is drawn from theme tokens (--wall-bg,
// --wall-ink, --wall-accent), so it follows the theme, accent and contrast with no extra art.
// Decorative only: the layer is hidden from assistive tech and never takes pointer events.

import { useId } from "react";
import type { Wallpaper as Kind } from "./appearance";

const ink = { stroke: "var(--wall-ink)", fill: "none" } as const;
const STRENGTH = { 1: 0.45, 2: 0.72, 3: 1 } as const;

/** `scale` shrinks the pattern for small previews so they show what the full chat will. */
export function Wallpaper({ kind, strength, scale = 1, className = "" }: { kind: Kind; strength: 1 | 2 | 3; scale?: number; className?: string }) {
  const id = useId().replace(/:/g, "");
  if (kind === "none") return null;
  const o = STRENGTH[strength];
  if (kind === "tint") {
    return (
      <div
        aria-hidden="true"
        className={`pointer-events-none absolute inset-0 ${className}`}
        style={{
          opacity: o,
          background:
            "radial-gradient(120% 90% at 100% 0%, color-mix(in oklab, var(--primary) 16%, transparent), transparent 60%), radial-gradient(90% 70% at 0% 100%, color-mix(in oklab, var(--primary) 9%, transparent), transparent 65%)",
        }}
      />
    );
  }
  return (
    <svg aria-hidden="true" className={`pointer-events-none absolute inset-0 size-full ${className}`} style={{ opacity: o }} preserveAspectRatio="xMidYMid slice">
      <defs>{patterns[kind]?.(id, scale)}</defs>
      {kind === "contours" ? (
        <svg viewBox="0 0 1200 900" preserveAspectRatio="xMidYMid slice" width="100%" height="100%">
          <path d={CONTOURS} style={{ ...ink, strokeWidth: 1.1 }} vectorEffect="non-scaling-stroke" />
          <path d={CONTOUR_INDEX} style={{ stroke: "var(--wall-accent)", fill: "none", strokeWidth: 1.4 }} vectorEffect="non-scaling-stroke" />
        </svg>
      ) : (
        <rect width="100%" height="100%" fill={`url(#${id})`} />
      )}
      {kind === "notebook" && <line x1={20 * scale} x2={20 * scale} y1="0" y2="100%" style={{ stroke: "var(--wall-accent)", strokeWidth: 1.25 }} />}
    </svg>
  );
}

const patterns: Partial<Record<Kind, (id: string, k: number) => React.ReactNode>> = {
  // A notebook: ruled lines and a margin.
  notebook: (id, k) => (
    <pattern id={id} width="16" height="30" patternUnits="userSpaceOnUse" patternTransform={`scale(${k})`}>
      <line x1="0" x2="16" y1="29.5" y2="29.5" style={{ ...ink, strokeWidth: 1 }} />
    </pattern>
  ),
  // Graph paper: fine squares, every fifth line a little stronger.
  graph: (id, k) => (
    <pattern id={id} width="120" height="120" patternUnits="userSpaceOnUse" patternTransform={`scale(${k})`}>
      {[24, 48, 72, 96].map((v) => (
        <g key={v} style={{ ...ink, strokeWidth: 0.6, opacity: 0.7 }}>
          <line x1={v} x2={v} y1="0" y2="120" />
          <line y1={v} y2={v} x1="0" x2="120" />
        </g>
      ))}
      <path d="M120 0V120M0 120H120" style={{ ...ink, strokeWidth: 1.1 }} />
    </pattern>
  ),
  dots: (id, k) => (
    <pattern id={id} width="20" height="20" patternUnits="userSpaceOnUse" patternTransform={`scale(${k})`}>
      <circle cx="10" cy="10" r="1.25" style={{ fill: "var(--wall-ink)" }} />
    </pattern>
  ),
  // Gentle swell, every other row shifted half a wave.
  waves: (id, k) => (
    <pattern id={id} width="120" height="44" patternUnits="userSpaceOnUse" patternTransform={`scale(${k})`}>
      <path d="M0 11Q30 3 60 11T120 11" style={{ ...ink, strokeWidth: 1.2 }} />
      <path d="M-60 33Q-30 25 0 33T60 33T120 33T180 33" style={{ ...ink, strokeWidth: 1.2 }} />
    </pattern>
  ),
  // Things from a work desk, scattered and turned. Each object is one simple outline.
  desk: (id, k) => (
    <pattern id={id} width="300" height="300" patternUnits="userSpaceOnUse" patternTransform={`scale(${k})`}>
      <g style={{ ...ink, strokeWidth: 1.4, strokeLinecap: "round", strokeLinejoin: "round" }}>
        {DESK.map(([d, x, y, r], i) => (
          <path key={i} d={d} transform={`translate(${x} ${y}) rotate(${r} 12 12)`} />
        ))}
      </g>
    </pattern>
  ),
};

const OBJ = {
  pen: "M4 20l2.5-6.5L16 4l4 4-9.5 9.5L4 20zM14 6l4 4",
  notebook: "M6 3h12a1.5 1.5 0 0 1 1.5 1.5v15A1.5 1.5 0 0 1 18 21H6zM6 3v18M10 8h6M10 12h6M10 16h4M4 7h2M4 12h2M4 17h2",
  key: "M8 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM12 12h9M18 12v3M21 12v2",
  envelope: "M5 6h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2zM3.6 7.2 12 13l8.4-5.8",
  clip: "M9 7v9a3 3 0 0 0 6 0V6a4.5 4.5 0 0 0-9 0v10.5",
  cup: "M5 9h11v6a5 5 0 0 1-5 5h-1a5 5 0 0 1-5-5zM16 11h1.5a2.5 2.5 0 0 1 0 5H16M8.5 3c0 1.5 1 1.5 1 3M12.5 3c0 1.5 1 1.5 1 3",
  lens: "M10 4a6 6 0 1 1 0 12 6 6 0 0 1 0-12zM14.5 14.5 20 20",
  bubble: "M5 5h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-7l-4 3.5V16H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2zM8 10.5h8",
  lock: "M7 11h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2zM8.5 11V8a3.5 3.5 0 0 1 7 0v3",
  page: "M7 3h7l4 4v14H7zM14 3v4h4M10 12h5M10 15.5h5",
  mic: "M12 3a3 3 0 0 1 3 3v5a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM6 11a6 6 0 0 0 12 0M12 17v4M9 21h6",
  camera: "M5.5 7H8l1.5-2.5h5L16 7h2.5A2.5 2.5 0 0 1 21 9.5v8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5v-8A2.5 2.5 0 0 1 5.5 7zM12 10a3.5 3.5 0 1 1 0 7 3.5 3.5 0 0 1 0-7z",
};

const DESK: [string, number, number, number][] = [
  [OBJ.pen, 22, 18, -12],
  [OBJ.bubble, 104, 36, 8],
  [OBJ.key, 196, 14, 24],
  [OBJ.notebook, 254, 84, -8],
  [OBJ.cup, 40, 104, 6],
  [OBJ.lens, 140, 116, -18],
  [OBJ.envelope, 214, 160, 10],
  [OBJ.clip, 18, 196, 28],
  [OBJ.page, 100, 204, -6],
  [OBJ.lock, 170, 242, 14],
  [OBJ.mic, 250, 250, -14],
  [OBJ.camera, 60, 262, 4],
];

/** Topographic contours: rings of a few hills, wobbled by low harmonics so they read as terrain. */
function ring(cx: number, cy: number, r: number, seed: number) {
  const pts: string[] = [];
  const n = 120;
  for (let i = 0; i <= n; i++) {
    const t = (i / n) * Math.PI * 2;
    const k = 1 + 0.12 * Math.sin(2 * t + seed) + 0.07 * Math.sin(3 * t + seed * 1.7) + 0.035 * Math.sin(5 * t + seed * 2.3);
    pts.push(`${(cx + Math.cos(t) * r * k).toFixed(1)} ${(cy + Math.sin(t) * r * k * 0.82).toFixed(1)}`);
  }
  return `M${pts.join("L")}Z`;
}

const HILLS: [number, number, number, number][] = [
  // cx, cy, rings, seed
  [260, 230, 9, 0.4],
  [930, 330, 11, 2.1],
  [520, 760, 8, 4.2],
  [1150, 860, 6, 1.3],
];

const CONTOURS = HILLS.flatMap(([cx, cy, n, s]) => Array.from({ length: n }, (_, i) => (i % 4 === 3 ? "" : ring(cx, cy, 26 + i * 34, s + i * 0.06)))).join("");
const CONTOUR_INDEX = HILLS.flatMap(([cx, cy, n, s]) => Array.from({ length: n }, (_, i) => (i % 4 === 3 ? ring(cx, cy, 26 + i * 34, s + i * 0.06) : ""))).join("");
