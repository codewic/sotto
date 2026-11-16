# Sotto brand art

All hand-built SVG, themed through CSS variables (`--sotto-mark-body`, `--mark-deep`, `--mark-mid`, `--mark-light`, `currentColor`), so every asset works in light and dark. Motion is transform and opacity only and switches off under reduced motion.

| Asset | File | Used in |
|---|---|---|
| `Mark` | `Mark.tsx` | Title bar, setup, unlock, Settings > About |
| `MarkIndicator` | `Mark.tsx` | Every loading moment (buttons, panes, call window; the same outline is inlined as the splash in `index.html`) |
| `GlyphConversations`, `GlyphDrop`, `GlyphCases`, `GlyphReview`, `GlyphSettings` | `glyphs.tsx` | Sidebar navigation and command palette |
| `GlyphShield`, `GlyphLock` | `glyphs.tsx` | Protection level and lock control |
| `SpotConversations` | `spots.tsx` | No conversations yet |
| `SpotDrop` | `spots.tsx` | Anonymous inbox: none connected, nothing received |
| `SpotSandbox` | `spots.tsx` | Safe review empty and quarantine states |
| `SpotLocked` | `spots.tsx` | Unlock screen |
| `SpotCases` | `spots.tsx` | No projects yet |
| `SpotDone` | `spots.tsx` | Setup finished, invite accepted |

The mark is freestanding: a message bubble with a padlock shackle and three quiet dots, no tile, background square, ring or shadow. Its source of truth is `brand/marks/sotto-light.svg` and `sotto-dark.svg` in the Vorynza workspace (rules in `brand/marks/README.md`); `Mark.tsx` copies the shapes and colours exactly. The two files differ only in the bubble body (`#2B2170` on light, `#3B2E9C` on dark), which `Mark` takes from `--sotto-mark-body` so it follows `data-scheme`; the shackle (`#8F7FFF`) and dots (`#C9BFFF`) are fixed. The mark never follows the chosen accent. To change it, change it in `brand/marks` first and copy it back here. On hover the bubble lifts and tilts while the shackle clicks down 1 px (transform only, off under reduced motion). `MarkIndicator` and the splash draw the mark's outline (shackle and bubble) in the accent colour.

The app icon is the only place the mark sits on a plate, because the operating system imposes a shape: `build/icon.svg` is a white rounded square (radius 25%, `#FFFFFF` to `#F3F4F7`) with the light mark centred at 80%, rendered to `build/icon.png` (512 px) for the AppImage and .deb. To re-render it, run sharp on `build/icon.svg` at 512 px (any copy of sharp works; nothing in this repo depends on it).

Spot illustrations use the accent-tuned `--mark-*` tones on a 48 x 48 grid with a top-left light. Nav glyphs: 20 x 20, 1.6 stroke, one fill at 18% opacity so the active state tints with the accent.

Chat wallpapers live in `src/theme/Wallpaper.tsx`. They are built the same way, hand-drawn SVG from theme tokens (`--wall-bg`, `--wall-ink`, `--wall-accent`): Notebook (ruled lines and a margin), Graph, Dots, Waves, Contours (generated topographic rings with accent index lines) and Desk (twelve one-stroke objects from a work desk on a 300 px tile). They are decorative only and never take pointer events.
