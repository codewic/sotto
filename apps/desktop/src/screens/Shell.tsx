import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Search } from "lucide-react";
import { createContext, type ReactNode, useContext, useEffect, useMemo, useRef, useState } from "react";
import { get, post } from "../api";
import { GlyphCases, GlyphConversations, GlyphDrop, GlyphLock, GlyphReview, GlyphSettings, GlyphShield } from "../brand/glyphs";
import type { ChannelView, Contact, DropView, GroupView, Status } from "../types";
import { setAppearance, toggleScheme } from "../theme/appearance";
import { Kbd } from "../ui/bits";
import { Cases } from "./Cases";
import { Conversations } from "./Conversations";
import { Drops } from "./Drops";
import { Review } from "./Review";
import { CallLayer } from "./Calls";
import { Settings } from "./Settings";
import { UpdateNotice } from "./Updates";

export type Screen = "conversations" | "drops" | "cases" | "review" | "settings";
type NavState = { screen: Screen; param?: string };

const NavCtx = createContext<{ go: (s: Screen, param?: string) => void; state: NavState; status: Status }>({
  go: () => {},
  state: { screen: "conversations" },
  status: { state: "unlocked", version: "" },
});
export const useNav = () => useContext(NavCtx);

const items: { screen: Screen; label: string; key: string; Glyph: (p: { className?: string }) => ReactNode }[] = [
  { screen: "conversations", label: "Conversations", key: "c", Glyph: GlyphConversations },
  { screen: "drops", label: "Anonymous inbox", key: "d", Glyph: GlyphDrop },
  { screen: "cases", label: "Projects", key: "a", Glyph: GlyphCases },
  { screen: "review", label: "Safe review", key: "r", Glyph: GlyphReview },
  { screen: "settings", label: "Settings", key: "s", Glyph: GlyphSettings },
];

function typingIn(el: EventTarget | null) {
  const t = el as HTMLElement | null;
  return !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
}

export function Shell({ status }: { status: Status }) {
  const qc = useQueryClient();
  const [state, setState] = useState<NavState>({ screen: "conversations" });
  const [palette, setPalette] = useState(false);
  const go = (screen: Screen, param?: string) => setState({ screen, param });
  const contacts = useQuery({ queryKey: ["contacts"], queryFn: () => get<Contact[]>("/api/contacts") });
  const drops = useQuery({ queryKey: ["drops"], queryFn: () => get<DropView[]>("/api/drops") });
  const groups = useQuery({ queryKey: ["groups"], queryFn: () => get<GroupView[]>("/api/groups") });
  const channels = useQuery({ queryKey: ["channels"], queryFn: () => get<ChannelView[]>("/api/channels") });
  const unread: Partial<Record<Screen, number>> = {
    conversations: (contacts.data?.reduce((n, c) => n + c.unread, 0) ?? 0) +
      // Muted groups and channels keep their own counts out of the badge.
      (groups.data?.reduce((n, g) => n + (g.muted ? 0 : g.unread), 0) ?? 0) +
      (channels.data?.reduce((n, c) => n + (c.muted ? 0 : c.unread), 0) ?? 0),
    drops: drops.data?.reduce((n, d) => n + d.unread, 0) ?? 0,
  };

  async function lock() {
    await post("/api/lock");
    await qc.invalidateQueries();
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
        return;
      }
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "l") {
        e.preventDefault();
        void lock();
        return;
      }
      if (e.metaKey || e.ctrlKey || e.altKey || typingIn(e.target) || document.querySelector("dialog[open]")) return;
      const item = items.find((i) => i.key === e.key);
      if (item) go(item.screen);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Privacy screen: in High risk and Extreme modes message text blurs whenever Sotto loses focus.
  const guarded = (status.settings?.risk_mode ?? "standard") !== "standard";
  useEffect(() => {
    const root = document.documentElement;
    if (!guarded) {
      root.classList.remove("privacy-active");
      return;
    }
    const hide = () => root.classList.add("privacy-active");
    const show = () => root.classList.remove("privacy-active");
    const vis = () => (document.hidden ? hide() : show());
    window.addEventListener("blur", hide);
    window.addEventListener("focus", show);
    document.addEventListener("visibilitychange", vis);
    if (!document.hasFocus()) hide();
    return () => {
      window.removeEventListener("blur", hide);
      window.removeEventListener("focus", show);
      document.removeEventListener("visibilitychange", vis);
      show();
    };
  }, [guarded]);

  return (
    <NavCtx.Provider value={{ go, state, status }}>
      <CallLayer contacts={contacts.data ?? []} onSettings={() => go("settings", "calls")}>
      <div className="flex h-full bg-background">
        <aside className="flex w-[64px] shrink-0 flex-col pt-1 pb-3 pl-2 md:w-[236px] md:pl-3">
          <button
            type="button"
            onClick={() => setPalette(true)}
            aria-label="Search and commands"
            className="press mr-2 mb-3 flex h-9 cursor-pointer items-center justify-center gap-2 rounded-lg bg-muted px-3 text-[13px] whitespace-nowrap text-muted-foreground hover:bg-muted-strong md:mr-3 md:justify-start"
          >
            <Search size={14} />
            <span className="hidden md:inline">Search</span>
            <span className="ml-auto hidden md:inline">
              <Kbd>⌘K</Kbd>
            </span>
          </button>
          <nav aria-label="Main" className="flex flex-col gap-0.5">
            {items.map(({ screen, label, key, Glyph }) => {
              const active = state.screen === screen;
              return (
                <button
                  key={screen}
                  type="button"
                  onClick={() => go(screen)}
                  aria-current={active ? "page" : undefined}
                  aria-label={label}
                  className={`glyph-host press group relative flex h-10 cursor-pointer items-center gap-3 pr-3 pl-3 text-[14px] ${
                    active ? "rounded-l-xl bg-pane font-medium text-foreground" : "mr-3 rounded-xl text-muted-foreground hover:bg-muted hover:text-foreground"
                  }`}
                >
                  <span className={active ? "text-primary" : "text-muted-foreground group-hover:text-primary"}>
                    <Glyph />
                  </span>
                  <span className="hidden flex-1 text-left md:inline">{label}</span>
                  {unread[screen] ? (
                    <span className="absolute top-1 left-7 min-w-5 rounded-full md:static bg-primary px-1.5 text-center text-[11px] font-semibold text-primary-foreground tabular">{unread[screen]}</span>
                  ) : (
                    <span className="hidden opacity-0 transition-opacity group-hover:opacity-100 md:inline">
                      <Kbd>{key.toUpperCase()}</Kbd>
                    </span>
                  )}
                </button>
              );
            })}
          </nav>
          <div className="mt-auto mr-2 flex flex-col gap-2 md:mr-3">
            <UpdateNotice />
            <div className="hidden md:block">
              <ConnectionCard status={status} />
            </div>
            <button
              type="button"
              onClick={lock}
              aria-label="Lock Sotto"
              className="glyph-host press flex h-10 cursor-pointer items-center gap-3 rounded-xl px-3 text-[14px] text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <GlyphLock />
              <span className="hidden flex-1 text-left md:inline">Lock Sotto</span>
              <span className="hidden md:inline">
                <Kbd>⌘L</Kbd>
              </span>
            </button>
          </div>
        </aside>
        <main className="mr-2 mb-2 ml-0 flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-pane shadow-pane">
          {state.screen === "conversations" && <Conversations initial={state.param} />}
          {state.screen === "drops" && <Drops />}
          {state.screen === "cases" && <Cases />}
          {state.screen === "review" && <Review initial={state.param} />}
          {state.screen === "settings" && <Settings key={state.param ?? ""} initial={state.param} />}
        </main>
      </div>
      {palette && <Palette onClose={() => setPalette(false)} contacts={contacts.data ?? []} onLock={lock} />}
      </CallLayer>
    </NavCtx.Provider>
  );
}

function ConnectionCard({ status }: { status: Status }) {
  const relays = status.relays ?? [];
  const tor = status.profile?.transport.kind === "tor";
  const online = relays.filter((r) => r.status === "online").length;
  const state = relays.length === 0 ? "idle" : online === relays.length ? "online" : relays.some((r) => r.status === "connecting") ? "connecting" : online > 0 ? "partial" : "offline";
  const torDown = tor && status.tor_running === false;
  const text = torDown
    ? "Tor isn't running"
    : {
        idle: "Ready",
        online: tor ? "Connected over Tor" : "Connected directly",
        connecting: "Connecting…",
        partial: "Some relays unreachable",
        offline: "Relay unreachable",
      }[state];
  const dot = torDown || state === "offline" ? "bg-destructive" : state === "online" || state === "idle" ? "bg-primary" : "bg-muted-foreground";
  const { go } = useNav();
  const mode = status.settings?.risk_mode ?? "standard";
  return (
    <div className="rounded-xl bg-muted px-3 py-2.5">
      <div className="flex items-center gap-2 text-[13px] font-medium">
        <span aria-hidden="true" className={`size-2 rounded-full ${dot} ${state === "connecting" ? "live-dot" : ""}`} />
        {text}
      </div>
      <div className="mt-1 flex items-center gap-1.5 text-[12px] text-muted-foreground">
        <GlyphShield className="size-3.5" />
        {mode === "standard" ? "Standard protection" : mode === "high" ? "High risk mode" : "Extreme mode"}
      </div>
      {!tor && mode !== "standard" && <div className="mt-1 text-[12px] text-destructive">Not using Tor: relays see your network address</div>}
      {(torDown || state === "offline") && (
        <button type="button" onClick={() => go("settings", "connection")} className="mt-1.5 cursor-pointer text-[12px] font-medium text-primary hover:underline">
          {torDown ? "Start Tor, or connect directly" : "Check connection settings"}
        </button>
      )}
    </div>
  );
}

function Palette({ onClose, contacts, onLock }: { onClose: () => void; contacts: Contact[]; onLock: () => void }) {
  const { go } = useNav();
  const [q, setQ] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const commands = useMemo(() => {
    const list: { label: string; hint: string; run: () => void }[] = [
      ...items.map((i) => ({ label: i.label, hint: "Go to", run: () => go(i.screen) })),
      { label: "Invite someone", hint: "Conversations", run: () => go("conversations", "invite") },
      { label: "Add someone by tag or link", hint: "Conversations", run: () => go("conversations", "accept") },
      { label: "Lock Sotto", hint: "Security", run: onLock },
      { label: "Appearance settings", hint: "Appearance", run: () => go("settings", "appearance") },
      { label: "Switch light and dark", hint: "Appearance", run: () => toggleScheme() },
      { label: "Use light theme", hint: "Appearance", run: () => setAppearance({ mode: "light" }) },
      { label: "Use dark theme", hint: "Appearance", run: () => setAppearance({ mode: "dark" }) },
      { label: "Follow system theme", hint: "Appearance", run: () => setAppearance({ mode: "system" }) },
      ...contacts.map((c) => ({ label: c.nickname || c.their_name || "Unnamed contact", hint: "Open conversation", run: () => go("conversations", c.id) })),
    ];
    const needle = q.trim().toLowerCase();
    return needle ? list.filter((c) => c.label.toLowerCase().includes(needle)) : list;
  }, [q, contacts, go, onLock]);

  useEffect(() => input.current?.focus(), []);
  // No open or close animation: the palette is used constantly, and motion would only slow it down.
  return (
    <div className="no-drag fixed inset-0 grid place-items-start justify-center bg-black/30 pt-[14vh]" style={{ zIndex: "var(--z-modal)" }} onMouseDown={onClose}>
      <div className="w-[560px] max-w-[calc(100vw-32px)] overflow-hidden rounded-2xl bg-pane shadow-float" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Search and commands">
        <div className="flex items-center gap-2 border-b border-line px-4">
          <Search size={16} className="text-muted-foreground" />
          <input
            ref={input}
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              setIndex(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) => Math.min(i + 1, commands.length - 1));
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(i - 1, 0));
              }
              if (e.key === "Enter" && commands[index]) {
                commands[index].run();
                onClose();
              }
            }}
            placeholder="Go to, search contacts, run a command…"
            className="h-12 flex-1 bg-transparent text-[15px] outline-none placeholder:text-muted-foreground"
            aria-label="Search"
          />
        </div>
        <ul className="scroll-pane max-h-[360px] p-1.5" role="listbox">
          {commands.length === 0 && <li className="px-3 py-6 text-center text-[13px] text-muted-foreground">Nothing matches "{q}".</li>}
          {commands.map((c, i) => (
            <li key={`${c.hint}-${c.label}-${i}`}>
              <button
                type="button"
                role="option"
                aria-selected={i === index}
                onMouseEnter={() => setIndex(i)}
                onClick={() => {
                  c.run();
                  onClose();
                }}
                className={`flex h-10 w-full cursor-pointer items-center justify-between rounded-lg px-3 text-left text-[14px] ${i === index ? "bg-muted" : ""}`}
              >
                <span>{c.label}</span>
                <span className="text-[12px] text-muted-foreground">{c.hint}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

/** Pane header with an optional loading line in its bottom border. */
export function PaneHeader({ title, subtitle, actions, loading }: { title: ReactNode; subtitle?: ReactNode; actions?: ReactNode; loading?: boolean }) {
  return (
    <header className="relative flex h-16 shrink-0 items-center gap-4 px-6">
      <div className="min-w-0 flex-1">
        <h1 className="truncate text-[17px] font-semibold tracking-tight">{title}</h1>
        {subtitle && <div className="truncate text-[12.5px] text-muted-foreground">{subtitle}</div>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
      <div className={`absolute right-0 bottom-0 left-0 h-px ${loading ? "loading-line" : "bg-line"}`} />
    </header>
  );
}
