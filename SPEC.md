# Sotto: a private, end-to-end encrypted workspace for teams
## Product concept, threat model and technical direction

**Status:** Built through Phase 4 tooling (2026-11-10); see `task.md` for what is done, what was verified, and what needs owner action
**Working name:** Sotto (from *sotto voce*, "under the voice"). Not final, see section 12.
**Platforms:** Desktop first (macOS, Windows, Linux), then iOS and Android, plus a no-install anonymous inbox page for people outside the organization
**Core:** Rust (protocol, crypto, storage, Tor), shared by every client
**Relays and anonymous inbox server:** Rust, Tor onion services, self-hostable

---

## 1. The idea, refined

The starting idea was "a completely anonymous messaging app journalists would love". Working it through showed the same needs in many more places: a security team handling a breach, a legal team on a sensitive matter, a company running a whistleblowing hotline, an executive whose phone number is a target, a developer receiving a vulnerability report. Two observations shaped the product:

1. **The person who most needs anonymity is often outside your team.** The employee reporting fraud, the researcher reporting a vulnerability, the source with a document: they are the ones who lose their job, their freedom or their safety if they are identified. So the anonymous inbox is designed around the sender first and the organization second.
2. **Messaging is only one step of sensitive work.** People get exposed at every step: the first contact, the files they send (printer dots, file metadata, unique wording), the way a team shares and stores material, and the moment something is shared outside. Signal protects the chat. Nothing protects the whole chain.

So Sotto is not "another private messenger". It is **a private workspace for teams**: one tool that covers sensitive work from the first contact to the moment it is shared, and makes the safe way to work the easy way.

### What it does, in one line each

- **Messenger:** conversations, groups, channels and calls with no phone number, no email, no username and no user ID on any server.
- **Anonymous inbox:** anyone outside your organization can reach you without installing anything, without an account, and without leaving a trace on their device. Tor Browser and a link is enough.
- **Safe review:** every incoming file is opened in a disposable sandbox and turned into a clean copy, so a malicious attachment cannot take over your laptop.
- **Projects:** a team works on sensitive material together (for example the lead, colleagues and legal) with its own retention rules, legal hold and insurance file, without ever seeing an anonymous sender's real identity unless the member holding the conversation chooses to share it.
- **Sharing check:** before anything leaves the vault, Sotto checks it for the things that have identified people in the past: hidden metadata, printer tracking dots, unique document fingerprints, fake redactions.

### Typical uses

- **Whistleblowing and ethics hotlines.** The EU Whistleblower Directive requires companies with 50 or more workers to run an internal reporting channel, acknowledge reports within seven days and give feedback within three months. The anonymous inbox lets someone report without being identified and come back with a codename to read replies.
- **Security vulnerability reports.** Researchers send findings and proof-of-concept files to an anonymous inbox; Safe review opens the attachments so nobody on the security team runs them by accident.
- **Incident response.** A project per incident, with only the people who need to know, its own retention, and calls that work when the usual chat tools may be compromised.
- **Legal matters and deals.** Privileged discussion and documents in a project with logged, time-limited access, legal hold and destroy dates.
- **Leadership and high-profile individuals.** Private conversations that are not tied to a phone number, little to find on a device taken at a border, and verified identities that make impersonation harder.
- **Engineering on sensitive work.** Unreleased products, security fixes under embargo, and anything a team would rather not leave in a third-party chat history.
- **Journalism.** Tips and documents from sources, long-running conversations with them, and a check before publication that catches what has identified sources in the past.

### Who it is for

| Person | What they need |
|---|---|
| **Team member** (engineering, product, operations or any other team) | Talk, share files and call without a phone number, email or account tying them to the conversation. Keep working safely if a laptop is lost, seized or searched at a border. |
| **Executive or high-profile individual** | Conversations that cannot be traced through a phone number or contact list, nothing useful on a device someone else gets hold of, and a way to tell real colleagues from impersonators. |
| **Developer or security team** | Receive vulnerability reports, run incident response in a project with its own retention, and open untrusted attachments without risk. |
| **Legal and compliance** | Run a whistleblowing hotline, review material inside a project with access that is logged and time-limited, and apply legal hold and destroy dates. |
| **Anonymous sender** (whistleblower, researcher, tipster, journalistic source) | Reach the organization safely, once or over months, with nothing on their phone or record that points back to them. Know they are talking to the real organization, not an impersonator. |
| **Journalist** (staff or freelance) | Receive tips and documents, talk to sources over long periods, protect them under subpoena, device seizure or border search. |
| **Organization admin or security lead** | Run the organization's intake, see what projects exist, enforce retention rules, without becoming a single point that can expose anonymous senders. |

---

## 2. What "anonymous" can honestly mean

"Completely anonymous" is a promise no software can keep, and the people who need privacy most know it. A product that claims it loses their trust on the first page. Sotto makes precise promises instead, and shows its limits plainly in the product.

### What Sotto guarantees

- **No identifiers.** No phone number, email, username or account ID exists anywhere. Identity is a set of keys on the device.
- **No social graph.** Relays cannot tell who talks to whom. Each contact uses its own one-way message queues, which can live on different relays, so no server sees both ends of a conversation.
- **No network location.** All traffic goes over Tor to onion services. Relays never see a real IP address.
- **Content only on the endpoints.** End-to-end encryption with forward secrecy and post-compromise security. Post-quantum key exchange from day one.
- **Little to seize.** Relays hold only encrypted, padded blobs, in memory, deleted on delivery or after a short TTL. No logs. Vorynza cannot hand over what it never had.
- **Survives device seizure better.** Local data is encrypted under a key held in the device's secure hardware plus a Sotto passcode. A duress passcode, panic wipe and travel mode exist for the moments that matter.

### What Sotto cannot protect against (shown in the app, in plain words)

- **A compromised device.** If spyware (Pegasus-class) is on the phone or laptop, it can read the screen. Sotto reduces the attack surface and pushes high-risk users to desktop, but it cannot fix an owned device.
- **The content itself.** If only five people saw a document, sharing it narrows the sender to five people. The sharing check helps; human judgment is still required.
- **The anonymous sender's own behaviour.** Using a work laptop, the office Wi-Fi, or telling a friend. The anonymous inbox page teaches this before first contact.
- **A global network adversary.** An attacker who can watch both ends of the Tor network at once may correlate timing. Cover traffic in high-risk mode makes this harder, not impossible.
- **Coercion of a member.** Sotto can make sure there is little to give up, and that the member need not know an anonymous sender's name. It cannot stop someone being pressured.

This section is a product feature, not legal small print. The first-run flow and the anonymous inbox page both show a short version.

---

## 3. Threat model

Adversaries, roughly in order of how often they expose people:

| Adversary | Capability | What Sotto does |
|---|---|---|
| **The anonymous sender's workplace** | Sees the work network, work devices, badge logs, who had access to a document | The anonymous inbox works only over Tor from a personal device. The page tells the sender not to use work equipment or networks, and not to look up the organization from work. The sharing check catches document fingerprints. |
| **Legal process against your organization or Vorynza** | Subpoenas, warrants, gag orders for records and metadata | No identifiers, no logs, no social graph, short-lived encrypted blobs. Transparency report and warrant canary. Self-hosted relays put the organization's own lawyers in control. |
| **Police or border agents with a member's device** | Physical access, compelled unlock | Separate Sotto passcode, duress passcode (opens a clean decoy), panic wipe, travel mode (projects are removed from the device and restored later from an encrypted backup only the owner can open). |
| **State spyware** | Zero-click exploits against phones, malicious documents | Text-only rendering (no link previews, no rich media decoding in the main process), attachments only opened in a disposable sandbox, high-risk mode pushes document work to desktop. |
| **Impersonators** | Fake accounts posing as your organization or a colleague, to trap anonymous senders or extract information | Verified member identities bound to the organization's own domain (section 5.5). Safety numbers and in-person QR verification. |
| **Network observers** | ISP, Wi-Fi owner, national firewall | Tor everywhere, pluggable transports (obfs4, Snowflake) where Tor is blocked, fixed-size packets, optional cover traffic. |
| **Malicious relay operator** | Runs or seizes a relay | Relays see only padded ciphertext with no sender, no recipient identity and no IP. Clients can spread queues across relays run by different operators. |
| **Abusers of the anonymous inbox** | Spam, malware, harassment of members | Proof-of-work and rate limits on the inbox, sandboxed review, triage view that never opens anything in the clear. |

Out of scope for v1: hardware implants, physical surveillance of the anonymous sender, and attacks on the Tor network itself beyond what cover traffic mitigates.

---

## 4. The three surfaces

### 4.1 Anonymous inbox (for people outside, no install)

Why no install: an app called Sotto on a whistleblower's phone is itself evidence. The first contact must leave nothing behind.

- Each organization, team or individual gets an **anonymous inbox page** served only as a Tor onion service.
- Their public website carries a **"Contact us securely" page** (static, served on the normal web) that explains how to install Tor Browser, gives the onion address and teaches the basics. It accepts nothing itself.
- On the anonymous inbox, the sender writes a message and attaches files. No account, no email. They get a **random codename** (eight words) to come back and read replies.
- **Works with Tor Browser on "Safest" (JavaScript off).** In that mode files are encrypted to the organization's public key on the inbox server the moment they arrive, in memory, on a machine that stores nothing else. With JavaScript on, the browser encrypts before upload and the server never sees plaintext. The page says which mode is active.
- An anonymous sender who wants a long-running conversation can **move to the app** with a one-time pairing code. Staying on the inbox is fine too.
- Before the first message, the inbox shows a short checklist: personal device, not work Wi-Fi, don't tell anyone, don't look us up from work, strip your own files if you can (or let us do it).
- One organization can run several inboxes for different jobs: a whistleblowing hotline, a security vulnerability report page, an ethics or HR tip line, or a tip line for journalists.

### 4.2 Messenger (members, contacts and anonymous senders)

- Contacts are added by QR code in person, a one-time invite link, or from the anonymous inbox. There is no directory of users to search.
- 1:1 conversations, groups and channels: text, files, voice notes, replies, reactions, pins and search. Voice calls, including group calls, go through a self-hosted call server with end-to-end encryption and do not use Tor (real-time media over Tor is too slow): [docs/calls.md](docs/calls.md). Groups and channels: [docs/groups.md](docs/groups.md).
- Disappearing messages on by default (7 days for new conversations with contacts, adjustable per conversation).
- Notifications never contain the sender or content. In high-risk mode there are no push notifications at all, because push services (Apple, Google) learn when a message arrives. See section 7.3.
- Messages render as plain text. Links are shown, never fetched. Files are never previewed outside the sandbox.

### 4.3 Organization (desktop)

- **Projects.** Each piece of sensitive work (an incident, a legal matter, a deal, an investigation) is a project: its own encrypted group, members, files, notes and retention rules.
- **Pseudonymous contacts.** Inside a project an anonymous sender or protected person is a pseudonym such as "Client QV4" or "Witness A", with a colour and mark. Only the member who holds the conversation can see the link between K7 and the actual chat, unless they share it.
- **Roles.** Owner, editor, contributor and legal. Access is per project, can be time-limited, and every access is written to an encrypted audit log that records actions, never content.
- **Safe review** (section 5.3) for every file.
- **Sharing check** (section 5.4) before export.
- **Retention.** Per-project destroy dates, legal hold, and crypto-shredding: destroying a project's key destroys every copy of its data, including backups.
- **Intake triage.** Anonymous inbox submissions land in a triage queue. Triage shows sanitized previews only and lets an admin assign a message to a member or close it.

---

## 5. Features that make it better than a private messenger

### 5.1 No-identifier, no-graph messaging

Pairwise one-way queues per contact, on relays chosen by the receiver. A relay sees "someone wrote to queue A" and "someone read queue A" over Tor. It never sees users, contacts or IPs. Queue addresses rotate periodically so long conversations do not become long-lived identifiers.

### 5.2 Risk modes

One switch, three levels, explained in plain language:

| Mode | For | What changes |
|---|---|---|
| **Standard** | Day-to-day contact with known colleagues | Tor, E2E, disappearing messages, content-free notifications |
| **High risk** | Sensitive projects and anonymous senders | Plus: no push notifications, 1-minute auto-lock, cover traffic while the app is open, attachments only on desktop |
| **Extreme** | Life-or-liberty situations | Plus: desktop only, documents reviewed on an offline machine via encrypted USB export, no mobile copy of the project |

### 5.3 Safe review

Every incoming file opens in a disposable sandbox (a lightweight VM or gVisor container with no network) and is converted to flat pixels, then rebuilt as a clean PDF or image with optional OCR. The original stays quarantined and encrypted. You read the clean copy. Malicious PDFs, Office macros and image exploits never touch the real system.

### 5.4 Sharing check

Before a document, screenshot or excerpt leaves the vault, whether for release or for outside counsel, Sotto runs checks drawn from real incidents where leakers and sources were identified:

- **Hidden metadata:** author, device, software, GPS, edit history, embedded thumbnails. Stripped.
- **Printer tracking dots:** the yellow microdots colour laser printers add to every page. Detected, warned about, and removed by converting to clean greyscale or re-typesetting.
- **Unique fingerprints:** organisations sometimes send each recipient a slightly different version (wording, spacing, image noise) to catch leakers. Sotto compares versions when there are several and flags passages that should be paraphrased, not quoted.
- **Fake redactions:** black boxes drawn over text that is still selectable underneath. Detected; real redaction burns the pixels.
- **Timing advice:** a reminder when sharing would closely follow the anonymous sender's own access to the material.

### 5.5 Verified organization identity

The biggest fear of a first-time anonymous sender is that the inbox is a trap, and a common attack on executives is a convincing impersonator. An organization publishes its members' Sotto public keys in a signed file at `https://<organization-domain>/.well-known/sotto.json`. The anonymous inbox page and the app check it and show "Verified: member of Example Co, checked against example.com". Members can also hold unlisted identities for work they don't want tied to the organization.

### 5.6 Device seizure tools

- **Duress passcode:** opens a believable decoy with harmless chats; can also wipe in the background.
- **Panic wipe:** a gesture or a wrong-passcode count destroys local keys in under a second.
- **Travel mode:** before a border crossing, projects are removed from the device and kept as an encrypted backup that only the owner's recovery key can open. Restored after crossing.

### 5.7 Insurance file (dead man's switch)

Optional, per project. If the owner does not check in by a set time, a prepared project package is released to named, verified colleagues (never to the public, never shared automatically). Protects the work if the owner is detained or cut off, and removes the incentive to detain them to stop it.

### 5.8 Safety coach, built in

Every risky moment gets a short, in-context hint, not a manual: before the first anonymous inbox message, before exporting a file, before turning off disappearing messages, before adding someone to a project. Written for people under stress: short, specific, no jargon.

---

## 6. Trust and governance

A privacy tool is only as good as the reasons to believe it.

- **Open source** client, core, relay and anonymous inbox server. Reproducible builds so anyone can confirm the published app matches the source.
- **Independent security audit** of the protocol and core before 1.0, published in full.
- **No analytics, no telemetry, no crash reporting by default.** Opt-in, local crash reports the user can read and send by hand.
- **No accounts for messaging.** Paid organization admin (billing, seats) uses a separate login that is never linked to messaging keys. Signing in with Vorynza ID for billing must not reveal who any messaging identity belongs to.
- **Self-hostable** relays and anonymous inbox server, packaged for one-command deployment, so an organization's own lawyers control legal exposure.
- **Transparency report and warrant canary** for Vorynza-hosted relays.
- **Jurisdiction:** hosted relays in more than one legal jurisdiction, with clients spreading queues across them.

---

## 7. Technical direction

### 7.1 Principle: no home-made crypto

Everything security-critical uses published, reviewed designs and maintained libraries. Our job is the system around them.

| Need | Choice |
|---|---|
| 1:1 key agreement | PQXDH (X25519 + ML-KEM) then Double Ratchet |
| Groups (projects and team groups) | MLS, RFC 9420, via OpenMLS |
| Transport | Tor via Arti (Rust), embedded in every client; onion services for relays and the anonymous inbox |
| Censorship | Pluggable transports: obfs4, Snowflake |
| Local storage | SQLCipher, key wrapped by Secure Enclave / StrongBox / TPM plus Sotto passcode (Argon2id) |
| Files | Chunked, padded, encrypted, each chunk on its own address on a file relay |
| Padding | Fixed-size packets (for example 16 KB); messages and chunks padded to bucket sizes |

### 7.2 Components

```
sotto/
  core/          Rust: protocol, crypto, queues, storage, Tor (Arti). UniFFI bindings.
  relay/         Rust: message queue relay, in-memory store, onion service, no logs
  files/         Rust: encrypted file chunk relay
  drop/          Rust: anonymous inbox server, no-JS HTML, onion service only
  desktop/       Tauri shell over core; safe review sandbox lives here
  ios/           SwiftUI over core
  android/       Jetpack Compose over core
  directory/     .well-known verification spec and organization tooling
  docs/          threat model, protocol spec, audit reports
```

- **Why Rust core:** one implementation of the dangerous parts for every platform, memory safety, and Arti is Rust. Vorynza already runs Rust in `intelligence/` and `drift/`.
- **Why native mobile shells** rather than React Native: smaller attack surface, no JS runtime in the security app, direct access to secure hardware, screenshot blocking and background limits. This departs from the Expo default in Timklock on purpose; see section 12.
- **Why desktop first:** sensitive work happens on laptops, safe review needs a VM, and high-risk mode wants desktop anyway.

### 7.3 Realtime and notifications

Vorynza's rule is "live by default, no polling". Sotto keeps it while the app is open: one long-lived Tor circuit per relay pushes new messages instantly. In the background it is a deliberate exception: Apple and Google push services learn when a message arrives, so Standard mode uses content-free push through a relay-owned token that carries no identity, and High risk mode uses none, fetching only when the app is opened. The trade-off is explained to the user in the risk mode screen.

### 7.4 Relay data lifecycle

Accepted blob in memory, padded and encrypted. Delivered: deleted. Undelivered after TTL (default 14 days, set by the receiver): deleted. Relays never write message data to disk and have no request logs. Restarting a relay loses undelivered messages; clients re-send on failure.

---

## 8. Design direction

All Vorynza design principles apply (`DESIGN_PRINCIPLES.md`), with these product notes:

- **Calm under pressure.** The person using Sotto may be frightened. Fewer choices per screen, plain words, no alarming red unless something is actually wrong. Status always pairs colour with an icon and a word.
- **Security state is always visible and honest:** "Connected over Tor", "Verified", "Not verified yet", "Messages disappear after 7 days". Never a vague padlock.
- **The anonymous inbox page is the most important screen.** It must work with JavaScript off, read well in Tor Browser at its default letterboxed size, load nothing from third parties, and teach in under a minute.
- **No external requests from any client UI:** fonts, icons and art are bundled. Geist ships inside the app.
- Bespoke Sotto mark and spot illustrations per the identity art rules. Accent colour to be chosen (section 12); it must hold 4.5:1 on both themes.

---

## 9. Business model

- **Free forever for anonymous senders.** No exceptions.
- **Free for individuals** on Vorynza-hosted relays, with one anonymous inbox.
- **Organization plan:** seats, projects, triage, roles, retention policy and legal hold, audit, several anonymous inboxes, priority hosting in chosen jurisdictions, support for self-hosting.
- **Press freedom programme:** free organization plans for independent media and civil society groups in high-risk countries, through partner organisations.
- Revenue never depends on data. There is none to sell.

---

## 10. Phased plan

| Phase | Scope | Exit test |
|---|---|---|
| **0. Foundations** | Threat model and protocol spec in `docs/`, reviewed by an outside cryptographer. Repo, Rust workspace, CI, reproducible build plan. | Written spec with review notes resolved |
| **1. Anonymous inbox + desktop messenger (MVP)** | `core`, `relay`, `files`, `drop`, desktop app. Anonymous inbox with codenames (JS and no-JS), 1:1 messaging, safe review, disappearing messages, Sotto passcode. Self-host packaging for relay and inbox. | An anonymous sender using only Tor Browser can send a document to an organization, a member reads a clean copy and replies, and a relay operator with full server access learns nothing beyond "blobs came and went" |
| **2. Organization + mobile** | Projects on MLS, roles, triage, retention and crypto-shredding, audit log, verified identities (`.well-known`), iOS and Android apps, risk modes | A three-person team runs a project end to end with the anonymous sender never named inside the vault |
| **3. Protection tools** | Sharing check, duress passcode, panic wipe, travel mode, insurance file, cover traffic, pluggable transports | Each tool has a documented test plan and passes it |
| **4. 1.0** | Independent audit, published report, reproducible builds verified by a third party, transparency report, hosted relays in two jurisdictions | Audit findings fixed and published |

---

## 11. Not in scope for v1

- Video calls in v1. Voice calls (1:1 and group) are built: a self-hosted call server, media end-to-end encrypted, no Tor (it was measured and is too slow for live calls), and calls are off in Extreme mode because the call server sees network addresses. See [docs/calls.md](docs/calls.md).
- Public channels or a channel directory (private channels you invite people to are built; see [docs/groups.md](docs/groups.md))
- Phone number or email contact discovery (by design, never)
- Cloud sync of message history (backups are local and encrypted; travel mode is the one exception)
- AI features that send content anywhere

---

## 12. Open decisions for the owner

1. **Name.** "Sotto" is a working name. Plain, short, not alarming on a home screen.
2. **Brand link.** Ship as "Sotto" alone, or visibly part of Vorynza? Some anonymous senders and security-minded buyers trust an independent, open-source name more than a company brand.
3. **First customer.** Organization-first (sell to companies for private team messaging, whistleblowing hotlines and security reports) or individual-first (executives, freelancers and researchers: faster adoption, harder to fund)?
4. **Mobile stack.** Native SwiftUI and Compose over the Rust core (recommended), or Expo with the Rust core as a native module (faster for the team, larger attack surface).
5. **Hosting.** Vorynza-hosted relays, organization self-hosting, or both from Phase 1 (recommended: both, self-host packaging is cheap once the relay is stateless).
6. **Insurance file.** Keep it in Phase 3 or drop it to keep scope tight?
7. **Accent colour.** Needs a choice that reads as calm and trustworthy, distinct from VMail blue and Cloud teal.
