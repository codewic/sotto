# Sotto

Source protection for investigative journalism: a no-install drop for sources, a messenger with no identifiers, sandboxed review of every file, encrypted case spaces for newsroom teams, and checks that catch what identifies sources at publication.

Read first: [`SPEC.md`](SPEC.md) (what and why), [`docs/threat-model.md`](docs/threat-model.md) (what it defends against and what it cannot), [`docs/protocol.md`](docs/protocol.md) (how). Progress: [`task.md`](task.md).

## Layout

| Path | What it is |
|---|---|
| `crates/sotto-core` | Cryptography and wire formats: PQXDH-style handshake (X25519 + ML-KEM-768), Double Ratchet, hybrid sealed boxes, invites, frames, codenames |
| `crates/sotto-vault` | Encrypted on-device storage, duress passcode, panic wipe, crypto-shredding scopes |
| `crates/sotto-relay` | Store-and-forward relay: one-way queues, ordered case queues, file chunks, time-locked insurance packages |
| `crates/sotto-client` | Messaging engine: Tor transport with per-relay circuits, cover traffic, contacts, sessions, files, receipts, disappearing messages |
| `crates/sotto-drop` | No-JavaScript source drop served as an onion service |
| `crates/sotto-review` | Safe review in a bubblewrap sandbox and the publish check (metadata, printer dots, hidden text, version differences) |
| `crates/sotto-cases` | MLS (RFC 9420) groups for cases, via OpenMLS |
| `crates/sotto-agent` | Desktop agent: owns the vault and engine, local API, live events, drops, cases, insurance, travel mode |
| `crates/sotto-directory` | Outlet directory (`/.well-known/sotto.json`) format and signing tool |
| `crates/sotto-ffi` | UniFFI bindings for iOS and Android |
| `apps/desktop` | Electron shell and React interface |
| `apps/mobile` | Generated Swift and Kotlin bindings and integration notes |
| `deploy/` | torrc and systemd templates for relays and drops |

## Develop

```
cargo test --workspace          # everything (needs bubblewrap, poppler-utils, libreoffice for safe-review tests)
cargo build -p sotto-agent -p sotto-relay -p sotto-review
cd apps/desktop && bun install && node scripts/dev.mjs
```

`scripts/dev.mjs` starts a local relay, two desks (yours on the UI at http://localhost:5174, a test peer on 127.0.0.1:7802) and the Vite dev server. Local development connects directly; real installs connect through Tor.

On machines without AVX2, Vite 8's native bundler crashes; the desktop app pins Vite 7 for that reason.

## Release

`scripts/reproducible-build.sh` builds twice from clean trees and compares hashes. Publish the resulting `SHA256SUMS` with every release so anyone can confirm the binaries match the source.

## Licence

AGPL-3.0-or-later.
