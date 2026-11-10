# Sotto brand art

All hand-built SVG, themed through CSS variables (`--mark-deep`, `--mark-mid`, `--mark-light`, `currentColor`), so every asset works in light and dark. Motion is transform and opacity only and switches off under reduced motion.

| Asset | File | Used in |
|---|---|---|
| `Mark` | `Mark.tsx` | Sidebar header, setup, unlock, about |
| `MarkIndicator` | `Mark.tsx` | Every loading moment (buttons, panes, splash in `index.html`) |
| `GlyphConversations`, `GlyphDrop`, `GlyphCases`, `GlyphReview`, `GlyphSettings` | `glyphs.tsx` | Sidebar navigation and command palette |
| `GlyphShield`, `GlyphLock` | `glyphs.tsx` | Protection level and lock control |
| `SpotConversations` | `spots.tsx` | No conversations yet |
| `SpotDrop` | `spots.tsx` | No drop connected, no submissions |
| `SpotSandbox` | `spots.tsx` | Safe review empty and quarantine states |
| `SpotLocked` | `spots.tsx` | Unlock screen |
| `SpotCases` | `spots.tsx` | No cases yet |
| `SpotDone` | `spots.tsx` | Setup finished, invite accepted |

Construction: 48 x 48 tile on a 4 px grid, 12 px corner radius, two tones of the Sotto violet, a soft top-left highlight. Nav glyphs: 20 x 20, 1.6 stroke, one fill at 18% opacity so the active state tints with the accent.
