# Sotto build tracker

Spec: `SPEC.md`. Protocol: `docs/protocol.md`. Threat model: `docs/threat-model.md`. Audit brief: `docs/audit-scope.md`.

Machine notes: 4 cores, about 2 GB free RAM, no AVX2 (Vite pinned to 7). `.cargo/config.toml` limits builds to 2 jobs. Run targeted tests (`cargo test -p <crate>`); the full suite takes about 10 minutes from cold.

## Positioning
- [x] Repositioned from journalists only to a private, end-to-end encrypted workspace for teams (12 November 2026): README, SPEC and docs rewritten; in the app Drop is now "Anonymous inbox", Cases are "Projects", project roles read Owner, Editor, Contributor and Legal, "Before you publish" is "Before you share". API paths, stored data and crate names are unchanged.
- [x] Developer features (`docs/developers.md`): code blocks with highlighting in messages, groups and channels; integrations that post into a group or channel with their own tokens, plus the `sotto-hook` command; one-time secrets in 1:1 conversations; search inside 1:1 conversations

- [x] Tags (`@ada`) for inviting without a link, contact requests, generated default avatars for everyone, a 960-avatar collection with a picker shared with contacts, and the Contours wallpaper by default (`docs/tags-and-avatars.md`)

## Releases and updates (`docs/releasing.md`)
- [x] `codewic/sotto` (private) and `codewic/sotto-releases` (public); release key made on the owner's machine, public half pinned
- [x] Signed automatic updates: agent checks and stages (`updates.rs`), shell re-verifies and installs, update notice and About panel; test `updates_install_only_signed_matching_packages`
- [x] Release workflow builds AppImage and .deb into an unsigned draft; `sotto-release publish` checks, signs and publishes
- [ ] Windows installer (the agent and review sandbox are Linux-only today) and macOS (needs an Apple developer account)
- [ ] Publish v0.2.0: CI draft built; the owner runs `sotto-release publish v0.2.0`
- [ ] Next from the plan: media, then Files (`docs/plan-updates-composer-media.md`)

## Composer (plan section 2)
- [x] One shared composer (`ui/Composer.tsx`) in conversations, groups and channels
- [x] Formatting toolbar on selection; Ctrl+B, Ctrl+I, Ctrl+E, Ctrl+Shift+C; bold and italic can hold other formatting
- [x] Several files at once (attach, paste, drop) in a tray with thumbnails; one caption for the set; a part-sent set keeps only what didn't go
- [x] @mentions in groups: member list as you type, highlighted in messages, an "@" badge that counts even when muted (`gmention/`); "you" in a group is found by signature key, so renaming yourself doesn't lose it
- [x] Emoji picker and `:shortcode:` completion
- [x] Slash commands: /code, /later, /emoji, plus /secret and /call where they apply
- [x] Send later (`scheduled.rs`): presets or a picked time, held encrypted on this device, sent while Sotto is open; scheduled chip with cancel; test `send_later_and_group_mentions`
- [x] Drafts everywhere, memory only, with "Draft: ..." in list previews
- [ ] Send as photo (cleaned) or as file: with media (plan section 3)
- [ ] Voice notes: with media
- [ ] Per-message disappearing time
- [ ] Silent send: needs desktop notifications first (Sotto doesn't show any yet)

## Phase 0: foundations
- [x] Threat model, protocol spec, Rust workspace, pinned toolchain, release profile
- [ ] Outside cryptographer review (owner action)

## Phase 1: drop and desktop messenger
- [x] `sotto-core`: X25519, Ed25519, ML-KEM-768, XChaCha20-Poly1305, HKDF, Argon2id, padding, PQXDH for invites, Double Ratchet, hybrid sealed box (stream and incremental), invites, frames, envelopes, codenames, recovery phrases, safety numbers
- [x] `sotto-vault`: encrypted records and salted blobs, duress passcode, panic wipe, crypto-shredding scopes, streaming writer
- [x] `sotto-relay`: queues, ordered group queues, file chunks, time locks, fixed frames, no logs, restart-safe metadata, quotas, idle expiry
- [x] `sotto-client`: Tor SOCKS with per-relay circuits, reconnect, cover traffic, invites, sessions, text, files, receipts, timers, deletion, disappearing messages, onion-only relays under Tor
- [x] `sotto-drop`: no-JS drop, codenames, streaming sealed uploads, journalist API with signed requests and replay cache, replies, KDF concurrency and session caps
- [x] `sotto-review`: bubblewrap sandbox (PDF, images, office documents through LibreOffice), clean pixel-only PDF, strict and capped parsing of sandbox output
- [x] `sotto-agent`: local API (token, exact Host check, unlock backoff), live events over SSE, drops, review, settings
- [x] Desktop app: Electron shell (sandboxed renderer, sender-checked IPC, content protection) and React UI. Verified in the browser pane: setup, unlock and error, conversations, live delivery, safe review with findings, cases, settings, light and dark
- [x] Deployment templates (`deploy/`): torrc with proof-of-work defence, hardened systemd units, bridges guidance
- [ ] Electron window itself not launched on this machine (verification uses the same UI in the browser pane)

## Phase 2: newsroom and mobile
- [x] Cases on MLS (OpenMLS): owner-only commits enforced by pinned signature key, members, notes, source pseudonyms, audit entries, retention, legal hold, crypto-shredding on every desk
- [x] Outlet directory format, signing CLI and verification endpoint (`sotto-directory`)
- [x] Mobile bindings (UniFFI), Swift and Kotlin generated (`apps/mobile/bindings`)
- [ ] Native iOS and Android app shells (no Xcode or Android SDK on this machine; integration notes in `apps/mobile/README.md`)
- [ ] In-app HTTPS fetch of `/.well-known/sotto.json` with key pinning

## Phase 3: protection tools
- [x] Publish check: metadata, printer dots (detect and remove), fake redactions, version comparison, timing reminder
- [x] Duress passcode and panic wipe
- [x] Travel mode: 24-word phrase, sealed archive verified before wipe, restore
- [x] Insurance file: time-locked package on the relay, key held by colleagues, check-ins on unlock and on demand
- [x] Cover traffic in High risk and Extreme modes
- [x] Pluggable transports through tor configuration (documented in `deploy/README.md`)

## Phase 4: 1.0
- [x] Reproducible build script and CI workflow (`scripts/reproducible-build.sh`, `.github/workflows/ci.yml`); not run on this machine (two full release builds)
- [x] Internal security review; high findings fixed (case authorisation, relay and drop resource limits), most medium findings fixed; the rest are listed in `docs/threat-model.md` section 6
- [x] Audit scope, transparency report and warrant canary templates
- [ ] Independent audit and its published report (owner action)
- [ ] Hosted relays in two jurisdictions (owner action)

## UI and UX overhaul (see `docs/ux.md`)
- [x] Replies, reactions, edits and opt-in read receipts in the protocol and engine (tested)
- [x] Chat list: previews, pinned on top, archive, filters (All, Unread, Pinned, Archived), search, right-click menu
- [x] Thread: grouped bubbles, sticky day labels, "New messages" divider, jump to latest with a count, hover actions and context menu
- [x] Composer: reply and edit banners, Up to edit your last message, drag and drop or paste a file, drafts kept in memory only
- [x] Safe formatting (bold, italic, code, quotes); links shown, never fetched
- [x] Protection strip under each header; info panel (Ctrl or Cmd I) with safety number, timer, read receipts, files
- [x] Privacy screen: messages and previews blur when Sotto loses focus in High risk and Extreme modes
- [x] Narrow layout: sidebar becomes a rail, chat list and thread become one column, info panel floats
- [ ] Message search inside a conversation
- [ ] Same treatment for Drop source threads and Case notes

## Theming (see `docs/ux.md`, Theming)
- [x] Token-driven themes: Light, Soft, Dim, Dark, Midnight; follow system with a separate light and dark pick
- [x] Seven accents tuned per scheme (Violet default, Graphite for no colour); every text pairing measured at 4.5:1 or better
- [x] Hand-drawn chat wallpapers (Notebook, Graph, Dots, Waves, Contours, Desk, Tint) with three strengths, drawn from tokens
- [x] Your messages as Ink, Accent or Tint; message text size; Comfortable or Compact density
- [x] Interface size (Ctrl/Cmd plus, minus, 0), increase contrast, reduce motion
- [x] Live preview built from the real message components; theme cards render real tokens
- [x] No flash on launch (applied before first paint), native dialogs and window follow the theme, new theme opens out from the click
- [x] Appearance kept outside the vault and removed by Erase and Travel mode
- [ ] Per-conversation wallpaper and a custom image wallpaper (would need storage inside the vault)

## Defaults taken for open decisions
- Name: Sotto (working). Standalone brand, Vorynza credited in About.
- Accent: violet `#5b47c7` light, `#a896ff` dark (the default; users can pick from seven tuned accents in Appearance).
- Desktop shell: Electron (no webkit2gtk here; Electron ships its own sandboxed Chromium).
- Hosting: hosted and self-hosted relays.
- Insurance file: kept.

## Calls (design in `docs/calls.md`, not started)
- [x] Phase 0: Tor latency spike (`crates/sotto-call-spike`). Result: too slow for live calls (best call 921 ms, median 1.4 s, 16 s to connect); calls move to a self-hosted server
- [ ] Phase 1: call server
  - [x] `crates/sotto-call-broker`: random rooms, short-lived tokens carrying room limits, rate limits, no logs (9 tests) and a smoke test that joins a real server
  - [x] `deploy/calls/`: Compose (LiveKit 1.13.3, broker, Caddy), `livekit.yaml` with TURN, `setup.sh`, firewall and operator guide, optional broker onion service
  - [x] Tested locally against LiveKit 1.13.3: pair and huddle rooms join with the right limits and a TURN relay; tampered and expired tokens refused
  - [x] `docs/transparency/call-server.md`: what the server sees and keeps
  - [x] Coolify package `deploy/calls/coolify/`: one pasteable compose file with the broker source embedded (generated by `make.py`, lock file pinned), guide for the server operator; tested locally (embedded files unpack byte-for-byte, LiveKit config from the environment, broker behind a container proxy)
  - [ ] Colleague deploys on Coolify and sends back the broker address; then run the smoke test against it
- [x] Phase 2: 1:1 voice on desktop (call messages and media keys, agent routes, dedicated call window with end-to-end encryption, call UI, missed calls, risk-mode rules); tested on a live local call
  - [x] Voice activity hidden from the server: no `ssrc-audio-level` header, constant-bitrate Opus, speaking detected on the device, mute local only (measured: packets 151/33 bytes before, a steady 150 after)
  - [ ] Try the Electron call window with a real microphone, then a call through the deployed server
- [x] Group calls: add people to any call (up to 8), names shared encrypted inside the call, ends when the room empties; tested live with three desks
- [ ] Phase 3: video in the call window
- [ ] Phase 4: phones (Android, then iOS)
- [ ] Phase 5: case huddles with MLS-derived keys, consensual recording into the case vault
- [ ] Owner decisions: calls in High risk mode, broker abuse control, video at launch

## Groups and channels (design in `docs/groups.md`)
- [x] Design: groups on Cases MLS, channels by 1:1 fan-out so subscribers never see each other, group calls
- [x] Groups: create, add and remove members, replies, reactions, edits, deletes, disappearing timer, pin and archive, group info, call the group from the header, live updates; agent test `groups_names_messages_and_removal`, tested live with three desks
- [x] Groups: files (quarantined, Safe review), search, pinned messages, admins besides the creator (adds and removals carried out by the creator's desk), group safety number, mute; agent test `groups_admins_pins_files_and_safety`
- [x] Channels: create, invite, follow by choice, posts as cards with edits and deletes, history for new followers, private replies to the owner, follower list on the owner's desk only, remove, close; agent test `channels_follow_post_reply_and_stay_private`, tested live with three desks
- [x] Channels: files in posts (copies per 250 followers, fresh copy for late followers), opt-in read receipts with "Seen by", search, mute; agent test `channels_files_receipts_and_mute`
- [ ] Owner decisions: Spaces (Discord-style servers), group size limit, channel size limit
