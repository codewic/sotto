# Sotto interaction design

Telegram and Discord set the bar for how messaging should feel: instant, dense without clutter, every action one gesture away. Sotto matches that capability and quality in its own design language, then goes further where its users have needs those apps ignore. It never copies their look (DESIGN_PRINCIPLES section 2).

## What we take (capability, not appearance)

| From | Capability | Sotto version |
|---|---|---|
| Telegram | Chat list that tells you everything at a glance | Last-message preview, time, unread count, pinned chats on top, verified mark, disappearing-timer mark, filters (All, Unread, Pinned, Archived) |
| Telegram | Messages that read like conversation | Bubbles grouped by sender within five minutes, tail only on the last of a group, day separators that stay pinned while scrolling |
| Telegram | Reply, edit, react without leaving the keyboard | Hover bar and right-click menu on every message; Up arrow edits your last message; Escape cancels |
| Telegram | Never lose your place | "New messages" divider at the first unread, a jump-to-latest button with a count, drafts kept per conversation for the session |
| Telegram | Profile panel | Info panel beside the chat: safety number, verification, timer, files shared, delete |
| Discord | Quick switcher | Command palette (already built), now also jumps to any conversation by name |
| Discord | Spaces with purpose | Conversations, Anonymous inbox, Projects and Review stay as separate spaces with their own unread counts |
| Both | Drag and drop anything | Drop a file anywhere on a conversation; a preview chip appears in the composer before sending |
| Both | Formatting | `*bold*`, `_italic_`, `` `code` `` and quoted lines rendered safely; links shown, never fetched |

## Where Sotto goes further

1. **Security is visible, never noisy.** A protection strip under each chat header says, in words, what protects this conversation right now: verified or not, Tor or direct, messages disappear after N. Telegram hides its secret-chat state behind a lock icon; we say it.
2. **Privacy screen.** In High risk and Extreme modes, message text blurs when the window loses focus, so a glance over the shoulder or a screen-share accident shows nothing. One click or key brings it back.
3. **Metadata-honest features.** Typing indicators and online presence leak when you are active, so Sotto does not have them. Read receipts are off by default and opt-in per conversation, with the trade-off explained where you turn them on.
4. **Files arrive quarantined and say so.** A received file is a card with its review state, not a preview that auto-opens. Opening it means safe review.
5. **Disappearing messages are tangible.** Messages on a timer show it; the info panel shows the setting and what it means.
6. **No accidental forwarding.** There is no forward button; quoting someone into another chat is a deliberate copy, not one click.

## Layout

```
┌ sidebar ┐┌──── chat list ────┐┌──────── conversation ────────┐┌─ info ─┐
│ spaces  ││ search, filters   ││ header + protection strip     ││ panel  │
│ unread  ││ pinned            ││ messages (grouped, dated)     ││ toggled│
│ status  ││ chats with preview││ composer (reply/edit banner)  ││        │
└─────────┘└───────────────────┘└───────────────────────────────┘└────────┘
```

At narrow widths the chat list and the conversation become one column with a back control, and the info panel slides over the conversation.

## Keyboard

`Ctrl/Cmd K` switcher · `Up` edit last message · `Esc` cancel reply or edit, close panel · `Enter` send, `Shift Enter` new line · `Alt Up/Down` previous or next conversation · `Ctrl/Cmd I` info panel.

## Theming

Telegram and Discord both let people make the app their own. Sotto matches that depth, keeps every option inside the design system, and adds what people doing sensitive work need.

| Area | What you can choose |
|---|---|
| Theme | Follow system, Light or Dark, with a separate pick for each: Light or Soft, and Dim, Dark or Midnight |
| Accent | Violet (Sotto's own), Blue, Teal, Green, Amber, Rose, Graphite. Each is tuned separately for light and dark |
| Chat wallpaper | None, Notebook, Graph, Dots, Waves, Contours, Desk, Tint, at Subtle, Medium or Strong |
| Messages | Your messages as Ink, Accent or Tint; message text size from 13 to 18 px; Comfortable or Compact |
| Accessibility | Interface size 90 to 125% (Ctrl/Cmd plus, minus, 0), increase contrast, reduce motion |

How it works:

- **Tokens, not themes in components.** `src/styles.css` defines every value. `src/theme/appearance.ts` sets `data-theme`, `data-scheme`, `data-accent`, `data-bubbles`, `data-wallpaper`, `data-density` and `data-contrast` on `<html>`. The same attributes on any element scope a full theme to it, which is how the theme cards and wallpaper tiles show real tokens rather than pictures.
- **Measured contrast.** Every pairing (body and muted text on panes and bubbles, accent text, text on accent, text and time inside your own messages) is checked at 4.5:1 or better across all 5 themes, 7 accents, 3 bubble styles, with and without wallpaper and increased contrast.
- **Wallpapers are drawn, not images.** `src/theme/Wallpaper.tsx` builds each one in SVG from `--wall-bg`, `--wall-ink` and `--wall-accent`, so it follows theme, accent and contrast. With a wallpaper on, the chat moves onto its own surface: in light themes the other side's messages lift off as white, in dark themes the chat sinks below the pane and messages rise above it.
- **No flash.** `public/splash.js` applies the saved appearance before the first paint; the Electron window background and native dialogs follow the theme through the bridge.
- **Theme changes explain themselves.** The new theme opens out from the control you used (a view transition, off under reduced motion).

Where Sotto goes further:

1. **Midnight** is true black with softer text: the screen gives off as little light as possible, for working in a dark room or anywhere a glowing face draws attention.
2. **Graphite** removes colour altogether, so nothing on screen catches the eye from across a room.
3. **Notebook, Contours and Desk** are drawn from working life (a ruled notebook, terrain, the things on a desk) instead of generic doodles.
4. **Appearance never touches your work.** It is kept outside the vault so the lock screen matches, holds nothing about conversations, and is removed by Erase and Travel mode.
5. **No custom photo wallpaper for now.** A personal photo behind your messages identifies you to anyone who glances at the screen, and would have to be stored inside the vault. Per-conversation wallpapers are left out for the same reason: they would mark which conversations matter.

Keyboard: `Ctrl/Cmd Shift L` switches light and dark from anywhere, the lock screen included. The command palette has Appearance settings, Switch light and dark, Use light theme, Use dark theme and Follow system theme.
