// Appearance: themes, accents, wallpapers, text size, density and accessibility.
//
// Kept on this computer only, outside the vault, so the lock screen opens in the right theme.
// It holds nothing about anyone's work, and "Erase this device" removes it too.
// public/splash.js applies the same attributes before the first paint so nothing flashes.

import { useSyncExternalStore } from "react";

export type Mode = "system" | "light" | "dark";
export type LightTheme = "light" | "soft";
export type DarkTheme = "dim" | "dark" | "midnight";
export type ThemeName = LightTheme | DarkTheme;
export type Accent = "violet" | "blue" | "teal" | "green" | "amber" | "rose" | "graphite";
export type Wallpaper = "none" | "notebook" | "graph" | "dots" | "waves" | "contours" | "desk" | "tint";
export type Bubbles = "ink" | "accent" | "tint";
export type Density = "comfortable" | "compact";

export type Appearance = {
  mode: Mode;
  light: LightTheme;
  dark: DarkTheme;
  accent: Accent;
  wallpaper: Wallpaper;
  /** Wallpaper strength: 1 subtle, 2 medium, 3 strong. */
  wallStrength: 1 | 2 | 3;
  bubbles: Bubbles;
  /** Message text size in px. */
  textSize: number;
  density: Density;
  /** Interface zoom factor. */
  zoom: number;
  contrast: boolean;
  reduceMotion: boolean;
};

export const THEMES: { id: ThemeName; label: string; scheme: "light" | "dark"; hint: string }[] = [
  { id: "light", label: "Light", scheme: "light", hint: "Clean white panes" },
  { id: "soft", label: "Soft", scheme: "light", hint: "Less glare under bright lights" },
  { id: "dim", label: "Dim", scheme: "dark", hint: "Lifted greys for long reading" },
  { id: "dark", label: "Dark", scheme: "dark", hint: "Deep greys" },
  { id: "midnight", label: "Midnight", scheme: "dark", hint: "True black, the least light" },
];

export const ACCENTS: { id: Accent; label: string; light: string; dark: string }[] = [
  { id: "violet", label: "Violet", light: "#5b47c7", dark: "#a896ff" },
  { id: "blue", label: "Blue", light: "#2160d8", dark: "#7eaaff" },
  { id: "teal", label: "Teal", light: "#08777f", dark: "#4fd1d9" },
  { id: "green", label: "Green", light: "#1f7a3e", dark: "#62d38c" },
  { id: "amber", label: "Amber", light: "#a3560b", dark: "#f2b35b" },
  { id: "rose", label: "Rose", light: "#c0365e", dark: "#ff8fae" },
  { id: "graphite", label: "Graphite", light: "#3f3f46", dark: "#d4d4d8" },
];

export const WALLPAPERS: { id: Wallpaper; label: string }[] = [
  { id: "none", label: "None" },
  { id: "notebook", label: "Notebook" },
  { id: "graph", label: "Graph" },
  { id: "dots", label: "Dots" },
  { id: "waves", label: "Waves" },
  { id: "contours", label: "Contours" },
  { id: "desk", label: "Desk" },
  { id: "tint", label: "Tint" },
];

export const ZOOMS = [0.9, 1, 1.1, 1.25] as const;
export const TEXT_MIN = 13;
export const TEXT_MAX = 18;

/** Window colour behind the page for each theme, so resizing never flashes the wrong colour. */
const BACKGROUNDS: Record<ThemeName, string> = { light: "#f3f3f3", soft: "#e7e7ea", dim: "#16171a", dark: "#0b0b0c", midnight: "#000000" };

const KEY = "sotto.appearance";

export const DEFAULTS: Appearance = {
  mode: "system",
  light: "light",
  dark: "dark",
  accent: "violet",
  wallpaper: "contours",
  wallStrength: 2,
  bubbles: "ink",
  textSize: 14.5,
  density: "comfortable",
  zoom: 1,
  contrast: false,
  reduceMotion: false,
};

const pick = <T extends string>(v: unknown, allowed: readonly T[], fallback: T): T => (allowed.includes(v as T) ? (v as T) : fallback);

function sanitize(raw: unknown): Appearance {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const num = (v: unknown, lo: number, hi: number, d: number) => (typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d);
  return {
    mode: pick(r.mode, ["system", "light", "dark"], DEFAULTS.mode),
    light: pick(r.light, ["light", "soft"], DEFAULTS.light),
    dark: pick(r.dark, ["dim", "dark", "midnight"], DEFAULTS.dark),
    accent: pick(r.accent, ACCENTS.map((a) => a.id), DEFAULTS.accent),
    wallpaper: pick(r.wallpaper, WALLPAPERS.map((w) => w.id), DEFAULTS.wallpaper),
    wallStrength: ([1, 2, 3] as const).includes(r.wallStrength as 1) ? (r.wallStrength as 1 | 2 | 3) : DEFAULTS.wallStrength,
    bubbles: pick(r.bubbles, ["ink", "accent", "tint"], DEFAULTS.bubbles),
    textSize: num(r.textSize, TEXT_MIN, TEXT_MAX, DEFAULTS.textSize),
    density: pick(r.density, ["comfortable", "compact"], DEFAULTS.density),
    zoom: (ZOOMS as readonly number[]).includes(r.zoom as number) ? (r.zoom as number) : DEFAULTS.zoom,
    contrast: typeof r.contrast === "boolean" ? r.contrast : DEFAULTS.contrast,
    reduceMotion: typeof r.reduceMotion === "boolean" ? r.reduceMotion : DEFAULTS.reduceMotion,
  };
}

function read(): Appearance {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) return sanitize(JSON.parse(raw));
  } catch {
    // Storage can be unavailable; the defaults still give a correct page.
  }
  // First run: honour the system's own accessibility choice.
  return { ...DEFAULTS, contrast: matchMedia("(prefers-contrast: more)").matches };
}

const darkQuery = matchMedia("(prefers-color-scheme: dark)");
let current = read();
const listeners = new Set<() => void>();

export function resolveTheme(a: Appearance, systemDark = darkQuery.matches): ThemeName {
  const dark = a.mode === "dark" || (a.mode === "system" && systemDark);
  return dark ? a.dark : a.light;
}

export const schemeOf = (t: ThemeName): "light" | "dark" => (t === "light" || t === "soft" ? "light" : "dark");

/** Every attribute that defines a theme. Put these on any element to scope a whole theme to it. */
export function themeAttrs(a: Appearance, theme: ThemeName = resolveTheme(a)): Record<string, string> {
  return {
    "data-theme": theme,
    "data-scheme": schemeOf(theme),
    "data-accent": a.accent,
    "data-bubbles": a.bubbles,
    "data-wallpaper": a.wallpaper,
    "data-density": a.density,
    ...(a.contrast ? { "data-contrast": "more" } : {}),
  };
}

function apply(a: Appearance) {
  const root = document.documentElement;
  const theme = resolveTheme(a);
  for (const [k, v] of Object.entries(themeAttrs(a, theme))) root.setAttribute(k, v);
  if (!a.contrast) root.removeAttribute("data-contrast");
  if (a.reduceMotion) root.setAttribute("data-motion", "reduce");
  else root.removeAttribute("data-motion");
  root.style.setProperty("--msg-size", `${a.textSize}px`);
  root.style.removeProperty("background"); // set by splash.js for the first paint only
  // Interface size: the app zooms the whole page natively; a browser preview falls back to CSS zoom.
  if (window.sotto?.setZoom) {
    void window.sotto.setZoom(a.zoom);
    root.style.removeProperty("zoom");
  } else if (a.zoom === 1) {
    root.style.removeProperty("zoom");
  } else {
    root.style.setProperty("zoom", String(a.zoom));
  }
  // Native parts (save dialogs, scrollbars, the window behind the page) follow the theme too.
  void window.sotto?.setAppearance?.({ scheme: schemeOf(theme), background: BACKGROUNDS[theme] });
}

apply(current);
darkQuery.addEventListener("change", () => {
  if (current.mode === "system") apply(current);
});

export function getAppearance() {
  return current;
}

const prefersReduced = () => current.reduceMotion || matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Change appearance and keep it. Pass `from` (a click point) when the theme itself may change:
 * the new theme then opens out from that point.
 */
export function setAppearance(patch: Partial<Appearance>, from?: { x: number; y: number }) {
  const next = sanitize({ ...current, ...patch });
  const before = resolveTheme(current);
  current = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Not kept, but still applied for this session.
  }
  const themeChanged = resolveTheme(next) !== before;
  const doc = document as Document & { startViewTransition?: (cb: () => void) => { ready: Promise<void> } };
  if (themeChanged && doc.startViewTransition && !prefersReduced()) {
    const x = from?.x ?? innerWidth / 2;
    const y = from?.y ?? innerHeight / 2;
    const r = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y));
    const t = doc.startViewTransition(() => apply(next));
    t.ready
      .then(() =>
        document.documentElement.animate(
          { clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${r}px at ${x}px ${y}px)`] },
          { duration: 460, easing: "cubic-bezier(0.16, 1, 0.3, 1)", pseudoElement: "::view-transition-new(root)" },
        ),
      )
      .catch(() => {});
  } else {
    apply(next);
  }
  listeners.forEach((l) => l());
}

/** Light becomes dark and dark becomes light (leaving "follow system"). */
export function toggleScheme(from?: { x: number; y: number }) {
  setAppearance({ mode: schemeOf(resolveTheme(current)) === "dark" ? "light" : "dark" }, from);
}

export function stepZoom(dir: 1 | -1 | 0) {
  if (dir === 0) return setAppearance({ zoom: 1 });
  const i = ZOOMS.indexOf(current.zoom as (typeof ZOOMS)[number]);
  setAppearance({ zoom: ZOOMS[Math.min(ZOOMS.length - 1, Math.max(0, (i < 0 ? 1 : i) + dir))] });
}

export function resetAppearance(from?: { x: number; y: number }) {
  setAppearance({ ...DEFAULTS }, from);
}

/** Used by Erase and Travel mode: leave no trace of how Sotto looked. */
export function forgetAppearance() {
  try {
    localStorage.removeItem(KEY);
  } catch {
    // Nothing kept.
  }
  current = { ...DEFAULTS };
  apply(current);
  listeners.forEach((l) => l());
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

function subscribeSystem(l: () => void) {
  darkQuery.addEventListener("change", l);
  return () => darkQuery.removeEventListener("change", l);
}

export function useAppearance() {
  const a = useSyncExternalStore(subscribe, getAppearance);
  // Re-render on system theme changes too, so "follow system" stays right everywhere.
  const systemDark = useSyncExternalStore(subscribeSystem, () => darkQuery.matches);
  return { ...a, resolved: resolveTheme(a, systemDark), systemDark };
}
