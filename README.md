# Sotto

Sotto is a private, end-to-end encrypted workspace for teams. It gives you messaging, groups, channels and calls with no phone numbers, emails or usernames; an anonymous inbox that people outside your organization can write to over Tor without installing anything; Safe review, which opens every incoming file in a disposable sandbox and hands you a clean copy; encrypted projects with roles, legal hold and insurance files; and a sharing check that catches identifying metadata, printer tracking dots and fake redactions before a document leaves your hands.

It suits any team whose conversations and files would do damage in the wrong hands: engineering and security teams, executives and other high-profile people, legal and compliance (including whistleblowing hotlines), incident response, and journalists working with sources.

Read first: [`SPEC.md`](SPEC.md) (what and why), [`docs/threat-model.md`](docs/threat-model.md) (what it defends against and what it cannot), [`docs/protocol.md`](docs/protocol.md) (how), [`docs/ux.md`](docs/ux.md) (how it should feel). Progress: [`task.md`](task.md).

## Layout

| Path | What it is |
|---|---|
| `crates/sotto-core` | Cryptography and wire formats: PQXDH-style handshake (X25519 + ML-KEM-768), Double Ratchet, hybrid sealed boxes, invites, frames, codenames |
| `crates/sotto-vault` | Encrypted on-device storage, duress passcode, panic wipe, crypto-shredding scopes |
| `crates/sotto-relay` | Store-and-forward relay: one-way queues, ordered project queues, file chunks, time-locked insurance packages |
| `crates/sotto-client` | Messaging engine: Tor transport with per-relay circuits, cover traffic, contacts, sessions, files, receipts, disappearing messages |
| `crates/sotto-drop` | No-JavaScript anonymous inbox served as an onion service |
| `crates/sotto-review` | Safe review in a bubblewrap sandbox and the sharing check (metadata, printer dots, hidden text, version differences) |
| `crates/sotto-cases` | MLS (RFC 9420) groups for projects and team groups, via OpenMLS |
| `crates/sotto-agent` | Desktop agent: owns the vault and engine, local API, live events, anonymous inbox, projects, groups, insurance, travel mode |
| `crates/sotto-directory` | Organization directory (`/.well-known/sotto.json`) format and signing tool |
| `crates/sotto-hook` | `sotto-hook`: post from scripts, CI and monitoring into a group or channel through the app on this computer |
| `crates/sotto-ffi` | UniFFI bindings for iOS and Android |
| `apps/desktop` | Electron shell and React interface |
| `apps/mobile` | Generated Swift and Kotlin bindings and integration notes |
| `deploy/` | torrc and systemd templates for relays and anonymous inboxes |

## Develop

```
cargo test --workspace          # everything (needs bubblewrap, poppler-utils, libreoffice for safe-review tests)
cargo build -p sotto-agent -p sotto-relay -p sotto-review
cd apps/desktop && bun install && node scripts/dev.mjs
```

`scripts/dev.mjs` starts a local relay, two devices (yours on the UI at http://localhost:5174, a test peer on 127.0.0.1:7802) and the Vite dev server. Local development connects directly; real installs connect through Tor.

On machines without AVX2, Vite 8's native bundler crashes; the desktop app pins Vite 7 for that reason.

## Release

`scripts/reproducible-build.sh` builds twice from clean trees and compares hashes. Publish the resulting `SHA256SUMS` with every release so anyone can confirm the binaries match the source.

## Licence

AGPL-3.0-or-later.
