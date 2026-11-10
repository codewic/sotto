# Sotto build tracker

Spec: `SPEC.md`. Protocol: `docs/protocol.md`. Threat model: `docs/threat-model.md`. Audit brief: `docs/audit-scope.md`.

Machine notes: 4 cores, about 2 GB free RAM, no AVX2 (Vite pinned to 7). `.cargo/config.toml` limits builds to 2 jobs. Run targeted tests (`cargo test -p <crate>`); the full suite takes about 10 minutes from cold.

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

## Defaults taken for open decisions
- Name: Sotto (working). Standalone brand, Vorynza credited in About.
- Accent: violet `#5b47c7` light, `#a896ff` dark.
- Desktop shell: Electron (no webkit2gtk here; Electron ships its own sandboxed Chromium).
- Hosting: hosted and self-hosted relays.
- Insurance file: kept.
