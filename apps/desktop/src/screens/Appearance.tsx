import { Check, RotateCcw } from "lucide-react";
import type { ReactNode } from "react";
import {
  ACCENTS,
  type Appearance as Look,
  DEFAULTS,
  TEXT_MAX,
  TEXT_MIN,
  THEMES,
  type ThemeName,
  WALLPAPERS,
  ZOOMS,
  resetAppearance,
  schemeOf,
  setAppearance,
  themeAttrs,
  useAppearance,
} from "../theme/appearance";
import { Wallpaper } from "../theme/Wallpaper";
import type { Message } from "../types";
import { Button } from "../ui/Button";
import { Avatar } from "../ui/Avatar";
import { Kbd, Segmented, Switch } from "../ui/bits";
import { Bubble } from "./Conversations";

/** Where the new theme opens out from: the control that was just used. */
function origin() {
  const el = document.activeElement as HTMLElement | null;
  if (!el || el === document.body) return undefined;
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
const mod = isMac ? "⌘" : "Ctrl";

export function Appearance() {
  const look = useAppearance();
  const scheme = schemeOf(look.resolved);
  const isDefault = (Object.keys(DEFAULTS) as (keyof Look)[]).every((k) => look[k] === DEFAULTS[k]);
  const showLight = look.mode !== "dark";
  const showDark = look.mode !== "light";

  return (
    <div className="@container mx-auto w-full max-w-[1080px] px-6 py-8 sm:px-8">
      <div className="grid grid-cols-1 gap-10 @[880px]:grid-cols-[minmax(0,1fr)_340px]">
        <aside className="@[880px]:order-last">
          <div className="@[880px]:sticky @[880px]:top-0">
            <LivePreview look={look} />
          </div>
        </aside>

        <div className="flex min-w-0 flex-col gap-10">
          <Block title="Theme" description="Follow your computer, or choose one. The light and dark looks are yours to pick.">
            <Segmented
              label="Theme mode"
              value={look.mode}
              onChange={(mode) => setAppearance({ mode }, origin())}
              options={[
                { value: "system", label: "Follow system" },
                { value: "light", label: "Light" },
                { value: "dark", label: "Dark" },
              ]}
            />
            {showLight && (
              <ThemeRow
                caption={look.mode === "system" ? "When your computer is light" : undefined}
                themes={THEMES.filter((t) => t.scheme === "light")}
                value={look.light}
                look={look}
                onPick={(id) => setAppearance({ light: id as Look["light"] }, origin())}
              />
            )}
            {showDark && (
              <ThemeRow
                caption={look.mode === "system" ? "When your computer is dark" : undefined}
                themes={THEMES.filter((t) => t.scheme === "dark")}
                value={look.dark}
                look={look}
                onPick={(id) => setAppearance({ dark: id as Look["dark"] }, origin())}
              />
            )}
            <p className="mt-4 text-[12.5px] text-muted-foreground">
              Switch between light and dark from anywhere with <Kbd>{mod}</Kbd> <Kbd>⇧</Kbd> <Kbd>L</Kbd>.
            </p>
          </Block>

          <Block
            title="Accent colour"
            description="Used for buttons, your current place and anything that needs your attention. Never as decoration."
            aside={<span className="text-[13px] font-medium">{ACCENTS.find((a) => a.id === look.accent)?.label}</span>}
          >
            <div role="radiogroup" aria-label="Accent colour" className="flex flex-wrap gap-3">
              {ACCENTS.map((a) => {
                const on = look.accent === a.id;
                const c = scheme === "dark" ? a.dark : a.light;
                return (
                  <button
                    key={a.id}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    aria-label={a.label}
                    title={a.label}
                    onClick={() => setAppearance({ accent: a.id })}
                    className={`press grid size-9 cursor-pointer place-items-center rounded-full outline-offset-2 ${on ? "outline-2 outline-[var(--swatch)]" : ""}`}
                    style={{ background: c, ["--swatch" as string]: c }}
                  >
                    {on && <Check size={16} strokeWidth={2.6} className={scheme === "dark" ? "text-black/80" : "text-white"} />}
                  </button>
                );
              })}
            </div>
            <p className="mt-3 text-[12.5px] text-muted-foreground">
              {look.accent === "graphite"
                ? "Graphite has no colour at all, so nothing on screen catches the eye from across a room."
                : look.accent === "violet"
                  ? "Violet is Sotto's own colour."
                  : "Each colour is tuned separately for light and dark so text stays easy to read."}
            </p>
          </Block>

          <Block title="Chat wallpaper" description="Drawn behind your conversations. Each one follows your theme and accent.">
            <div role="radiogroup" aria-label="Chat wallpaper" className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              {WALLPAPERS.map((w) => {
                const on = look.wallpaper === w.id;
                return (
                  <button key={w.id} type="button" role="radio" aria-checked={on} onClick={() => setAppearance({ wallpaper: w.id })} className="group flex cursor-pointer flex-col gap-2 text-left">
                    <span
                      {...themeAttrs({ ...look, wallpaper: w.id }, look.resolved)}
                      className={`press relative block aspect-[4/3] overflow-hidden rounded-xl bg-[var(--chat-bg)] outline-offset-2 ${on ? "outline-2 outline-[var(--primary)]" : "group-hover:opacity-90"}`}
                      style={{ boxShadow: "var(--shadow-pane)" }}
                    >
                      <Wallpaper kind={w.id} strength={look.wallStrength} scale={0.55} />
                      <span className="absolute inset-x-2.5 bottom-2.5 flex flex-col gap-1">
                        <span className="h-2.5 w-3/5 rounded-full bg-[var(--bubble-theirs)] shadow-[var(--bubble-theirs-shadow)]" />
                        <span className="h-2.5 w-1/2 self-end rounded-full bg-[var(--bubble-mine)]" />
                      </span>
                    </span>
                    <span className="flex items-center gap-1.5 text-[12.5px] font-medium">
                      {w.label}
                      {on && <Check size={13} className="text-primary" aria-hidden="true" />}
                    </span>
                  </button>
                );
              })}
            </div>
            {look.wallpaper !== "none" && (
              <div className="mt-5 flex flex-wrap items-center gap-3">
                <span className="text-[13.5px]">Strength</span>
                <Segmented
                  label="Wallpaper strength"
                  value={String(look.wallStrength) as "1" | "2" | "3"}
                  onChange={(v) => setAppearance({ wallStrength: Number(v) as 1 | 2 | 3 })}
                  options={[
                    { value: "1", label: "Subtle" },
                    { value: "2", label: "Medium" },
                    { value: "3", label: "Strong" },
                  ]}
                />
              </div>
            )}
          </Block>

          <Block title="Messages" description="How conversations read.">
            <div className="flex flex-col gap-7">
              <div>
                <Label>Your messages</Label>
                <div role="radiogroup" aria-label="Your messages" className="mt-2 grid grid-cols-3 gap-3">
                  {(
                    [
                      ["ink", "Ink", "Dark and quiet"],
                      ["accent", "Accent", "Your accent colour"],
                      ["tint", "Tint", "A light wash of it"],
                    ] as const
                  ).map(([id, label, hint]) => {
                    const on = look.bubbles === id;
                    return (
                      <button key={id} type="button" role="radio" aria-checked={on} onClick={() => setAppearance({ bubbles: id })} className="group flex cursor-pointer flex-col gap-2 text-left">
                        <span
                          {...themeAttrs({ ...look, bubbles: id, wallpaper: "none" }, look.resolved)}
                          className={`press flex h-16 flex-col justify-center gap-1.5 rounded-xl bg-[var(--pane)] px-3 outline-offset-2 ${on ? "outline-2 outline-[var(--primary)]" : ""}`}
                          style={{ boxShadow: "var(--shadow-pane)" }}
                        >
                          <span className="h-3 w-3/5 rounded-full bg-[var(--bubble-theirs)]" />
                          <span className="flex h-5 w-4/5 items-center justify-end self-end rounded-full bg-[var(--bubble-mine)] px-2">
                            <span className="h-1 w-1/2 rounded-full bg-[var(--bubble-mine-fg)] opacity-60" />
                          </span>
                        </span>
                        <span>
                          <span className="flex items-center gap-1.5 text-[12.5px] font-medium">
                            {label}
                            {on && <Check size={13} className="text-primary" aria-hidden="true" />}
                          </span>
                          <span className="block text-[12px] text-muted-foreground">{hint}</span>
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>

              <div>
                <div className="flex items-baseline justify-between">
                  <Label htmlFor="text-size">Message text size</Label>
                  <span className="text-[12.5px] text-muted-foreground tabular">{look.textSize === DEFAULTS.textSize ? "Default" : `${look.textSize} px`}</span>
                </div>
                <div className="mt-2 flex items-center gap-3">
                  <span aria-hidden="true" className="text-[12px] font-medium text-muted-foreground">
                    A
                  </span>
                  <input
                    id="text-size"
                    type="range"
                    min={TEXT_MIN}
                    max={TEXT_MAX}
                    step={0.5}
                    value={look.textSize}
                    onChange={(e) => setAppearance({ textSize: Number(e.target.value) })}
                    className="h-1.5 flex-1 cursor-pointer accent-[var(--primary)]"
                  />
                  <span aria-hidden="true" className="text-[18px] font-medium text-muted-foreground">
                    A
                  </span>
                </div>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <Label>Density</Label>
                  <p className="text-[12.5px] text-muted-foreground">Compact fits more messages and chats on screen.</p>
                </div>
                <Segmented
                  label="Density"
                  value={look.density}
                  onChange={(density) => setAppearance({ density })}
                  options={[
                    { value: "comfortable", label: "Comfortable" },
                    { value: "compact", label: "Compact" },
                  ]}
                />
              </div>
            </div>
          </Block>

          <Block title="Accessibility" description="Make Sotto easier to see and calmer to use.">
            <div className="flex flex-col gap-6">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <Label>Interface size</Label>
                  <p className="text-[12.5px] text-muted-foreground">
                    Or press <Kbd>{mod}</Kbd> <Kbd>+</Kbd> and <Kbd>{mod}</Kbd> <Kbd>−</Kbd> anywhere. <Kbd>{mod}</Kbd> <Kbd>0</Kbd> resets.
                  </p>
                </div>
                <Segmented
                  label="Interface size"
                  value={String(look.zoom)}
                  onChange={(v) => setAppearance({ zoom: Number(v) })}
                  options={ZOOMS.map((z) => ({ value: String(z), label: `${Math.round(z * 100)}%` }))}
                />
              </div>
              <ToggleRow title="Increase contrast" text="Stronger text, lines and surfaces in every theme." checked={look.contrast} onChange={(contrast) => setAppearance({ contrast })} />
              <ToggleRow
                title="Reduce motion"
                text="Sotto already follows your computer's setting. Turn this on to keep motion to a minimum here whatever that says."
                checked={look.reduceMotion}
                onChange={(reduceMotion) => setAppearance({ reduceMotion })}
              />
            </div>
          </Block>

          <div className="flex flex-col items-start gap-3 rounded-xl bg-muted px-4 py-3.5 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-[12.5px] leading-relaxed text-muted-foreground">
              Appearance is kept on this computer, outside your vault, so the lock screen matches. It holds nothing about your work, and erasing Sotto removes it too.
            </p>
            <Button size="sm" variant="ghost" icon={<RotateCcw size={14} />} disabled={isDefault} onClick={() => resetAppearance(origin())}>
              Reset appearance
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Block({ title, description, aside, children }: { title: string; description: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section>
      <div className="mb-4 flex items-baseline justify-between gap-4">
        <div>
          <h2 className="text-[15px] font-semibold tracking-tight">{title}</h2>
          <p className="mt-0.5 text-[13px] text-muted-foreground">{description}</p>
        </div>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Label({ children, htmlFor }: { children: ReactNode; htmlFor?: string }) {
  return (
    <label htmlFor={htmlFor} className="text-[13.5px] font-medium">
      {children}
    </label>
  );
}

function ToggleRow({ title, text, checked, onChange }: { title: string; text: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="flex items-center justify-between gap-6">
      <div>
        <div className="text-[13.5px] font-medium">{title}</div>
        <div className="text-[12.5px] text-muted-foreground">{text}</div>
      </div>
      <Switch checked={checked} onChange={onChange} label={title} />
    </div>
  );
}

function ThemeRow({ caption, themes, value, look, onPick }: { caption?: string; themes: typeof THEMES; value: ThemeName; look: Look; onPick: (id: ThemeName) => void }) {
  return (
    <div className="mt-5">
      {caption && <div className="mb-2 text-[12.5px] font-medium text-muted-foreground">{caption}</div>}
      <div role="radiogroup" aria-label={caption ?? "Theme"} className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {themes.map((t) => {
          const on = value === t.id;
          return (
            <button key={t.id} type="button" role="radio" aria-checked={on} onClick={() => onPick(t.id)} className="group flex cursor-pointer flex-col gap-2 text-left">
              <ThemeThumb look={look} theme={t.id} selected={on} />
              <span>
                <span className="flex items-center gap-1.5 text-[13px] font-medium">
                  {t.label}
                  {on && <Check size={13} className="text-primary" aria-hidden="true" />}
                </span>
                <span className="block text-[12px] text-muted-foreground">{t.hint}</span>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** A miniature Sotto window, drawn with the real tokens of the theme it stands for. */
function ThemeThumb({ look, theme, selected }: { look: Look; theme: ThemeName; selected: boolean }) {
  return (
    <span
      {...themeAttrs(look, theme)}
      className={`press block aspect-[16/10] overflow-hidden rounded-xl bg-[var(--background)] p-1.5 outline-offset-2 ${selected ? "outline-2 outline-[var(--primary)]" : "group-hover:opacity-90"}`}
      style={{ boxShadow: "var(--shadow-pane)" }}
    >
      <span className="flex h-full gap-1.5">
        <span className="flex w-[24%] flex-col gap-1 pt-1 pl-0.5">
          <span className="mb-1 size-2.5 rounded-[4px] bg-[var(--primary)]" />
          <span className="h-1.5 w-4/5 rounded-full bg-[var(--primary)] opacity-80" />
          <span className="h-1.5 w-3/5 rounded-full bg-[var(--muted-strong)]" />
          <span className="h-1.5 w-2/3 rounded-full bg-[var(--muted-strong)]" />
        </span>
        <span className="relative flex flex-1 flex-col justify-end gap-1 overflow-hidden rounded-lg bg-[var(--chat-bg)] p-1.5" style={{ boxShadow: "var(--shadow-pane)" }}>
          <Wallpaper kind={look.wallpaper} strength={look.wallStrength} scale={0.4} />
          <span className="relative h-2 w-3/5 rounded-full bg-[var(--bubble-theirs)]" />
          <span className="relative h-2 w-1/2 self-end rounded-full bg-[var(--bubble-mine)]" />
          <span className="relative mt-0.5 h-2.5 rounded-full bg-[var(--composer)]" />
        </span>
      </span>
    </span>
  );
}

const noop = () => {};
const now = Date.now();
const msg = (id: string, outgoing: boolean, text: string, minutesAgo: number, extra: Partial<Message> = {}): Message => ({
  id,
  contact_id: "preview",
  outgoing,
  kind: "text",
  text,
  file: null,
  sent_at: now - minutesAgo * 60_000,
  received_at: now - minutesAgo * 60_000,
  expires_at: null,
  delivery: outgoing ? "delivered" : "received",
  reply_to: null,
  reactions: [],
  edited: false,
  read: false,
  ...extra,
});

const SAMPLE: Message[][] = [
  [
    msg("p2", false, "Same shell company, *different* director. Can you check the filing dates?", 13, { reactions: [{ emoji: "👍", count: 1, mine: true }] }),
  ],
  [msg("p3", true, "On it. The registry shows `2023-04-11` for both.", 9, { reply_to: "p2", read: true })],
  [msg("p4", false, "That's before the contract was signed.", 4)],
];

/** The real conversation components, at your current settings. Nothing here is interactive. */
function LivePreview({ look }: { look: Look & { resolved: ThemeName } }) {
  const byId = new Map(SAMPLE.flat().map((m) => [m.id, m]));
  return (
    <figure className="m-0 overflow-hidden rounded-2xl bg-pane" style={{ boxShadow: "var(--shadow-float)" }}>
      <div className="flex h-12 items-center gap-2.5 px-3.5">
        <Avatar seed="preview:Ben" size={32} />
        <span className="min-w-0">
          <span className="block truncate text-[13.5px] font-semibold">Ben (platform team)</span>
          <span className="block truncate text-[11.5px] text-muted-foreground">End-to-end encrypted · verified</span>
        </span>
      </div>
      <div inert className="relative flex h-[300px] flex-col @[880px]:h-[380px] justify-end [mask-image:linear-gradient(to_bottom,transparent,black_28px)] gap-[var(--run-gap)] overflow-hidden border-t border-line bg-[var(--chat-bg)] px-3.5 pt-3 pb-3">
        <Wallpaper kind={look.wallpaper} strength={look.wallStrength} />
        <div className="relative flex justify-center">
          <span className="rounded-full bg-pane/90 px-3 py-1 text-[11.5px] font-medium text-muted-foreground shadow-pane">Today</span>
        </div>
        {SAMPLE.map((run) => (
          <div key={run[0].id} className={`relative flex flex-col gap-0.5 ${run[0].outgoing ? "items-end" : "items-start"}`}>
            {run.map((m, i) => (
              <Bubble
                key={m.id}
                m={m}
                first={i === 0}
                last={i === run.length - 1}
                reply={m.reply_to ? byId.get(m.reply_to) : undefined}
                replyName={(r) => (r.outgoing ? "You" : "Ben")}
                flash={false}
                reviewing={false}
                onReply={noop}
                onReact={noop}
                onEdit={noop}
                onCopy={noop}
                onMenu={noop}
                onJump={noop}
                onReview={noop}
                onOpenReview={noop}
                onRetry={noop}
              />
            ))}
          </div>
        ))}
        <div className="relative mt-1 flex h-10 shrink-0 items-center rounded-2xl bg-[var(--composer)] px-3 text-[length:var(--msg-size)] text-muted-foreground shadow-[var(--composer-shadow)]">
          Message Ben
        </div>
      </div>
      <figcaption className="flex items-center justify-between gap-2 px-3.5 py-2.5 text-[12px] text-muted-foreground">
        <span>Live preview</span>
        <span>
          {THEMES.find((t) => t.id === look.resolved)?.label} · {ACCENTS.find((a) => a.id === look.accent)?.label}
        </span>
      </figcaption>
    </figure>
  );
}
