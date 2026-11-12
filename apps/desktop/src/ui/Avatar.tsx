import type { ReactNode } from "react";

/**
 * Everyone's default avatar: a small hand-drawn mark, unique-looking per person and the same
 * every time. One of ten shapes, in two tones of one of twelve hues, turned one of four ways,
 * all chosen from a hash of a stable seed (an identity fingerprint where there is one). Lightness
 * and chroma come from theme tokens, so it sits right in light and dark.
 */

const HUES = [12, 32, 52, 78, 108, 148, 172, 196, 222, 252, 286, 322];

type Shape = (fg: string, mid: string) => ReactNode;

const SHAPES: Shape[] = [
  // Orbit: a planet and its moon.
  (fg, mid) => (
    <>
      <circle cx="19" cy="22" r="9" fill={fg} />
      <circle cx="29.5" cy="11.5" r="4" fill={mid} />
    </>
  ),
  // Peaks: two mountains, one behind the other.
  (fg, mid) => (
    <>
      <path d="M15 31 24.5 17 34 31Z" fill={mid} />
      <path d="M6 31 15.5 13 25 31Z" fill={fg} />
    </>
  ),
  // Tide: a swell under a lighter crest.
  (fg, mid) => (
    <>
      <path d="M5 15.5q7.5-7 15 0t15 0" fill="none" stroke={mid} strokeWidth="3.2" strokeLinecap="round" />
      <path d="M5 23q7.5-7 15 0t15 0V31H5Z" fill={fg} />
    </>
  ),
  // Halves: a circle cut on the diagonal.
  (fg, mid) => (
    <>
      <path d="M12.2 27.8A11 11 0 0 1 27.8 12.2Z" fill={fg} />
      <path d="M27.8 12.2A11 11 0 0 1 12.2 27.8Z" fill={mid} />
    </>
  ),
  // Stack: three staggered bars.
  (fg, mid) => (
    <>
      <rect x="8" y="10.5" width="17" height="5.5" rx="2.75" fill={mid} />
      <rect x="13" y="17.5" width="19" height="5.5" rx="2.75" fill={fg} />
      <rect x="8" y="24.5" width="13" height="5.5" rx="2.75" fill={mid} />
    </>
  ),
  // Leaf, with its vein.
  (fg, mid) => (
    <>
      <path d="M9 31C9 17 18 9 31 9 31 23 23 31 9 31Z" fill={fg} />
      <path d="M12 28 26 14" stroke={mid} strokeWidth="2.4" strokeLinecap="round" />
    </>
  ),
  // Kite: a diamond inside a diamond.
  (fg, mid) => (
    <>
      <path d="M20 6.5 33.5 20 20 33.5 6.5 20Z" fill={mid} />
      <path d="M20 13.5 26.5 20 20 26.5 13.5 20Z" fill={fg} />
    </>
  ),
  // Arch, with a stone under it.
  (fg, mid) => (
    <>
      <path d="M8.5 31V20a11.5 11.5 0 0 1 23 0v11h-6V20a5.5 5.5 0 0 0-11 0v11Z" fill={fg} />
      <circle cx="20" cy="27.5" r="3" fill={mid} />
    </>
  ),
  // Bloom: four petals round a centre.
  (fg, mid) => (
    <>
      <circle cx="20" cy="12.5" r="5" fill={mid} />
      <circle cx="27.5" cy="20" r="5" fill={mid} />
      <circle cx="20" cy="27.5" r="5" fill={mid} />
      <circle cx="12.5" cy="20" r="5" fill={mid} />
      <circle cx="20" cy="20" r="4.5" fill={fg} />
    </>
  ),
  // Crescent and a star.
  (fg, mid) => (
    <>
      <path d="M23.5 8.5A12 12 0 1 0 31.5 27 9.5 9.5 0 1 1 23.5 8.5Z" fill={fg} />
      <circle cx="28.5" cy="13" r="2.6" fill={mid} />
    </>
  ),
  // Comet: a head and its trail.
  (fg, mid) => (
    <>
      <path d="M23 17 9.5 30.5" stroke={mid} strokeWidth="5" strokeLinecap="round" />
      <path d="M20 13.5 12 21.5" stroke={mid} strokeWidth="2.6" strokeLinecap="round" opacity="0.7" />
      <circle cx="26.5" cy="13.5" r="6" fill={fg} />
    </>
  ),
  // Cell: a hexagon in a hexagon.
  (fg, mid) => (
    <>
      <path d="M20 7.5 30.8 13.75v12.5L20 32.5 9.2 26.25v-12.5Z" fill={mid} />
      <path d="M20 14 25.2 17v6L20 26 14.8 23v-6Z" fill={fg} />
    </>
  ),
  // Bolt.
  (fg, mid) => (
    <>
      <circle cx="29" cy="29" r="3" fill={mid} />
      <path d="M22.5 6 11 22.5h7.5L15.5 34 28 17h-7.5l2-11Z" fill={fg} />
    </>
  ),
  // Sprout: two leaves on a stem.
  (fg, mid) => (
    <>
      <path d="M20 33V19" stroke={mid} strokeWidth="2.8" strokeLinecap="round" />
      <path d="M20 20C20 13 15 9 8 9c0 7 5 11 12 11Z" fill={fg} />
      <path d="M20 23c0-6 4-9.5 11-9.5 0 6-4 9.5-11 9.5Z" fill={mid} />
    </>
  ),
  // Rings: a dot inside a ring.
  (fg, mid) => (
    <>
      <circle cx="20" cy="20" r="11.5" fill="none" stroke={mid} strokeWidth="3.4" />
      <circle cx="20" cy="20" r="5.5" fill={fg} />
    </>
  ),
  // Dots: a three by three grid.
  (fg, mid) =>
    [0, 1, 2].flatMap((r) =>
      [0, 1, 2].map((c) => <circle key={`${r}${c}`} cx={11 + c * 9} cy={11 + r * 9} r={(r + c) % 2 ? 2.8 : 3.4} fill={(r + c) % 2 ? mid : fg} />),
    ),
  // Sail: a boat under two sails.
  (fg, mid) => (
    <>
      <path d="M20.5 7v20H9Z" fill={fg} />
      <path d="M22.5 11v16H32Z" fill={mid} />
      <path d="M8 29h25l-3.5 4.5h-18Z" fill={fg} />
    </>
  ),
  // Prism: a triangle in two tones.
  (fg, mid) => (
    <>
      <path d="M20 7 33.5 31.5h-27Z" fill={mid} />
      <path d="M20 7v24.5H6.5Z" fill={fg} />
    </>
  ),
  // Spark: a four-point star and a small one.
  (fg, mid) => (
    <>
      <path d="M18 6.5c1.2 6.5 3.5 8.8 10 10-6.5 1.2-8.8 3.5-10 10-1.2-6.5-3.5-8.8-10-10 6.5-1.2 8.8-3.5 10-10Z" fill={fg} />
      <path d="M28.5 23.5c.6 3 1.7 4.1 4.7 4.7-3 .6-4.1 1.7-4.7 4.7-.6-3-1.7-4.1-4.7-4.7 3-.6 4.1-1.7 4.7-4.7Z" fill={mid} />
    </>
  ),
  // Steps: three blocks climbing.
  (fg, mid) => (
    <>
      <rect x="7" y="23" width="8.5" height="9" rx="2" fill={mid} />
      <rect x="15.75" y="15.5" width="8.5" height="16.5" rx="2" fill={fg} />
      <rect x="24.5" y="8" width="8.5" height="24" rx="2" fill={mid} />
    </>
  ),
];

export const SHAPE_COUNT = SHAPES.length;
export const HUE_COUNT = HUES.length;

export type Parts = { hue: number; shape: number; turn: number };

/** A chosen avatar's id (`v1:shape:hue:turn`), or null if it isn't one from the collection. */
export function parseChoice(id: string | null | undefined): { shape: number; hueIndex: number; turn: number } | null {
  const m = /^v1:(\d+):(\d+):(\d)$/.exec(id ?? "");
  if (!m) return null;
  const [shape, hueIndex, turn] = [Number(m[1]), Number(m[2]), Number(m[3])];
  return shape < SHAPES.length && hueIndex < HUES.length && turn < 4 ? { shape, hueIndex, turn } : null;
}

export const choiceId = (shape: number, hueIndex: number, turn: number) => `v1:${shape}:${hueIndex}:${turn}`;

/** The hue in degrees for a hue index. */
export const hueOf = (i: number) => HUES[i % HUES.length];

/** FNV-1a: small, stable, and spreads similar seeds apart. */
function hash(s: string) {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** A contact's seed: their identity fingerprint once known, so their default mark never changes. */
export const seedOf = (c: { id: string; their_fingerprint: string | null }) => c.their_fingerprint ?? c.id;

/** What an avatar looks like: the person's choice if they made one, otherwise from their seed. */
export function avatarParts(seed: string, choice?: string | null): Parts {
  const c = parseChoice(choice);
  if (c) return { hue: HUES[c.hueIndex], shape: c.shape, turn: c.turn * 90 };
  const h = hash(seed || "?");
  return { hue: HUES[h % HUES.length], shape: (h >>> 8) % SHAPES.length, turn: ((h >>> 16) % 4) * 90 };
}

/** The default (seed-based) avatar as a collection id, so the picker can start from it. */
export function defaultChoice(seed: string) {
  const h = hash(seed || "?");
  return choiceId((h >>> 8) % SHAPES.length, h % HUES.length, (h >>> 16) % 4);
}

export function Avatar({
  seed,
  choice,
  size = 40,
  label,
  square = false,
  children,
}: {
  seed: string;
  /** A chosen avatar id; falls back to the seed's default. */
  choice?: string | null;
  size?: number;
  label?: string;
  square?: boolean;
  children?: ReactNode;
}) {
  const { hue, shape, turn } = avatarParts(seed, choice);
  const tone = (l: string, c: string) => `oklch(var(${l}) var(${c}) ${hue})`;
  return (
    <span className="relative inline-grid shrink-0" style={{ width: size, height: size }} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <svg viewBox="0 0 40 40" width={size} height={size} className={square ? "rounded-[30%]" : "rounded-full"}>
        <rect width="40" height="40" fill={tone("--av-bg-l", "--av-bg-c")} />
        <g transform={`rotate(${turn} 20 20)`}>{SHAPES[shape](tone("--av-fg-l", "--av-fg-c"), tone("--av-mid-l", "--av-mid-c"))}</g>
      </svg>
      {children}
    </span>
  );
}
