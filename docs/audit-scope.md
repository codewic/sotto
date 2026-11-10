# Independent audit: scope and brief

For the external firm reviewing Sotto before 1.0. The audit report will be published in full.

## In scope, by priority

1. **Protocol** (`docs/protocol.md`, `crates/sotto-core`): the invite-based PQXDH adaptation (no DH4, joiner transcript signature), Double Ratchet implementation, hybrid sealed box with STREAM chunking, file chunk construction, padding, relay authentication bound to a per-connection nonce.
2. **Vault** (`crates/sotto-vault`): key slots and their indistinguishability, duress behaviour (destroys real slots, removes unreadable records), crypto-shredding by scope, blob STREAM encryption, atomic writes.
3. **Relay** (`crates/sotto-relay`): what a seized relay learns (memory and `meta.bin`), authorisation of every command, resource exhaustion, time-lock semantics.
4. **Drop** (`crates/sotto-drop`): codename-derived keys (Argon2id parameters), session and CSRF handling, the no-JavaScript server-side sealing model, journalist API authentication and replay protection, file-system metadata.
5. **Safe review** (`crates/sotto-review`): sandbox escape surface (bubblewrap flags, what is bound in), strict parsing of sandbox output, completeness of the clean copy (no text layer, metadata or active content survives).
6. **Cases** (`crates/sotto-cases`, `crates/sotto-agent/src/cases.rs`): OpenMLS usage, owner-only commits over an ordered relay queue, member removal, insurance package handling.
7. **Desktop** (`crates/sotto-agent`, `apps/desktop/electron`): local API token and Host checks, Unix socket permissions, Electron sandboxing and navigation controls, travel-mode archive restore.

## Out of scope

Tor itself, the operating systems, OpenMLS internals and RustCrypto primitives (reviewed elsewhere), the visual design.

## Questions we want answered

- Can any party other than the endpoints read content, or link two conversations to one person?
- Can a relay operator, with full access, learn more than the threat model says?
- Can a hostile file reach the host outside the sandbox?
- Does duress or panic wipe leave anything recoverable that the documentation says it removes?
- Can a forged invite, join, MLS message or directory file be accepted?

## Known limits (documented, not findings)

See `docs/threat-model.md` section 6 and `docs/protocol.md` section 12.

## Access

Source at the tagged release, reproducible build script, a test relay and drop on onion services, and direct contact with the maintainers.
