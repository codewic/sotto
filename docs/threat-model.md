# Sotto threat model

Version 1, 2026-11-10. Companion to `SPEC.md` section 3 and `docs/protocol.md`. This document is the reference for what each component must defend against, and what it explicitly does not.

## 1. Assets

| Asset | Why it matters |
|---|---|
| **Anonymous sender and contact identity** | The primary asset. Exposure can cost a job, liberty or life. |
| **Who talks to whom** (social graph) | Reveals contacts and anonymous senders without reading a single message. |
| **Message and file content** | Sensitive material, often enough on its own to identify who sent it. |
| **Timing** | When an anonymous sender made contact, correlated with who had access to a document. |
| **Member device data** | Everything above, at rest, on a device that can be seized. |
| **Project structure** | Which members, colleagues and legal staff work on what. |

## 2. Adversaries and capabilities

**A1. The anonymous sender's workplace.** Controls the sender's work network and devices, keeps access logs, knows who could see a leaked document, may plant uniquely marked copies. For an internal whistleblowing hotline this can include people inside the organization that runs the inbox.

**A2. Legal compulsion.** Subpoenas, warrants and gag orders served on the organization, the relay operator or Vorynza. Can demand stored data and, in some places, ongoing interception.

**A3. Device seizure.** Police or border agents with physical access to a member's or anonymous sender's device, possibly compelling unlock.

**A4. Targeted spyware.** Zero-click or document-borne exploits against a member's phone or laptop.

**A5. Impersonation.** An adversary posing as your organization or one of its members, to identify anonymous senders who reach out or to extract information from colleagues.

**A6. Network observer.** ISP, Wi-Fi owner or national firewall watching an anonymous sender's or a member's traffic.

**A7. Malicious or seized relay.** Full control of one or more relays, including memory.

**A8. Anonymous inbox abuse.** Spam, malware, harassment and resource exhaustion against an organization's anonymous inbox.

**A9. Global passive adversary.** Watches large parts of the internet, including both ends of Tor circuits. Partially out of scope (section 5).

## 3. Security goals and how each is met

| ID | Goal | Mechanism | Against |
|---|---|---|---|
| G1 | No component outside the endpoints can read content | E2E: PQXDH handshake, Double Ratchet, MLS for projects and groups, hybrid sealed boxes for the anonymous inbox | A2, A6, A7 |
| G2 | No server can build the social graph | No accounts or identifiers; pairwise one-way queues with unrelated send and receive ids; receiver chooses relay; queues can sit on different relays | A2, A7 |
| G3 | No server learns network location | Clients reach relays and the anonymous inbox only as Tor onion services | A2, A6, A7 |
| G4 | Servers learn nothing from size | Fixed-size frames and blobs; files in fixed-size chunks | A6, A7 |
| G5 | Little exists to seize on a server | Messages in memory only, deleted on delivery or TTL; no request logs; the anonymous inbox stores only sealed blobs | A2, A7 |
| G6 | Past messages survive key compromise | Forward secrecy from the ratchet; disappearing messages on by default | A3, A4 |
| G7 | Device data resists seizure | Vault encrypted under Argon2id-derived key; separate Sotto passcode; duress passcode; panic wipe; travel mode | A3 |
| G8 | Hostile files cannot run on a member's machine | Safe review: render in a network-less sandbox to raw pixels, rebuild a clean copy outside | A4, A8 |
| G9 | Anonymous senders and contacts can tell real members from fakes | Directory keys at `/.well-known/sotto.json` on the organization's own domain; safety numbers | A5 |
| G10 | Sharing a document does not reveal the anonymous sender | Sharing check: metadata, printer dots, version fingerprints, fake redactions, timing reminder | A1 |
| G11 | First contact leaves no trace on the anonymous sender's device | The anonymous inbox works in Tor Browser with JavaScript off, no install, no account | A1, A3 |
| G12 | The organization cannot be forced to name an anonymous sender it never learned | Pseudonymous contacts inside projects; only the member holding the conversation links pseudonym to conversation | A2 |

## 4. Trust assumptions

- The endpoint device is not compromised at the time of use (A4 defeats every messenger; Sotto narrows the attack surface but cannot fix an owned device).
- The Tor network provides its usual guarantees against non-global adversaries.
- The organization's HTTPS website is controlled by the organization (directory verification anchors on it).
- Cryptographic primitives and the libraries listed in `docs/protocol.md` are sound.
- The OS sandbox (bubblewrap with namespaces on Linux; a VM on macOS and Windows) holds.

## 5. Out of scope (stated plainly in the product)

- Compromised endpoints, keyloggers, screen capture by malware.
- Identification from the content of the material itself (small circle of knowledge).
- The anonymous sender's own operational mistakes beyond what the anonymous inbox page teaches.
- End-to-end timing correlation by a global passive adversary. Cover traffic in High risk mode raises the cost; it does not remove the risk.
- Physical surveillance, coercion, rubber-hose cryptanalysis.
- Hardware implants and supply-chain attacks on devices.

## 6. Residual risks we accept in version 1, and the plan for each

| Risk | Why accepted now | Plan |
|---|---|---|
| The anonymous inbox's no-JS mode decrypts nothing but encrypts on the server, which briefly holds plaintext in memory | Required for JavaScript-off senders; same model as SecureDrop | Browser-side encryption when JavaScript is allowed |
| Ratchet is classical after the PQ handshake | No reviewed sparse PQ ratchet in Rust yet | Adopt once reviewed |
| Relay restart loses undelivered messages | Memory-only storage is a deliberate G5 choice | Receipts surface undelivered messages to the sender |
| Global rate limits on the anonymous inbox can be used to slow it down | Every onion client looks the same to the server | Tor onion service proof-of-work defence (`HiddenServicePoWDefensesEnabled`) in the deployment template |
| Push notifications reveal arrival time to Apple or Google | Needed for Standard mode usability on mobile | Off in High risk mode; content-free and identity-free in Standard |
| Filesystem timestamps on the anonymous inbox server reveal coarse arrival times | Hard to avoid entirely on a normal filesystem | The inbox rounds file times to the day; deployment on tmpfs-backed or encrypted volumes |
| A seized relay's `meta.bin` maps each queue's send id to its receive id and auth key | Without it, every conversation dies when a relay restarts | Encrypt the file under a key held outside the relay (operator-entered at start), and rotate queues every 7 days |
| Directory checks verify a pasted file against the key inside it | The app has no TLS client yet, so it cannot fetch `/.well-known/sotto.json` itself | Fetch over Tor with TLS in-app and pin the organization key on first use, warning if it changes |
| The review sandbox has namespaces and memory limits but no seccomp filter | bubblewrap's seccomp needs a compiled BPF filter that is not written yet | Add a seccomp allowlist and CPU and process limits |
| Vault writes do not fsync the directory; old blocks may survive on SSDs | Logical destruction (header slots replaced) is in place; physical erasure depends on the storage | fsync the directory after header writes; advise full-disk encryption |
| Project members are named by a self-chosen display name in notes and audit entries | Authorisation already uses the owner's pinned MLS signature key | Show each author's key fingerprint and warn on duplicate names |

## 7. Review checklist for every change

1. Does it add a field a server can see? Is it padded, random, or necessary?
2. Does it create a long-lived identifier (key, id, token) that links sessions?
3. Does it write plaintext or metadata to disk anywhere, including logs and temp files?
4. Does it parse untrusted input outside the sandbox? Is the parser memory-safe and bounded?
5. Does it make a network request outside Tor, or to a third party?
6. Is the failure mode safe (fail closed), and does the user see an honest message?
