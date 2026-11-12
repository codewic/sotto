# Sotto calls

Status: built; Sotto ships pointing at `https://calls.cmigl.org` (media on `wss://rtc.cmigl.org`), and organizations can set their own in Settings. Calls run through a call server we host on our own VPS, with end-to-end encryption keyed from the conversation. Tor is no longer used for calls (see "Why not Tor" at the end). Supersedes the "no calls in v1" lines in SPEC.md (sections 4.2 and 11).

## The decision in one paragraph

We self-host **LiveKit** (Apache 2.0, a single Go server with its own TURN relay built in) on the VPS. Every call is a throwaway room on that server. Audio and video are encrypted end to end on each device with a key that comes from the Sotto conversation itself, so the server forwards media it cannot decrypt. The server sees network addresses and call timing; it never sees content, names or contacts. Each side only ever talks to the server, so the two people on a call never learn each other's network address.

## Why LiveKit

| Option | Verdict |
|---|---|
| **LiveKit** | Chosen. Apache 2.0 SFU in Go, one binary, built-in TURN, end-to-end encryption supported on self-hosted servers with keys we supply, official SDKs for Rust, JavaScript, Swift and Android, short-lived signed join tokens |
| Jitsi | A whole conferencing product (web app, XMPP, Java bridge). Heavier to run, its encryption is built around its own web client, no Rust path |
| mediasoup | A media library, not a server: we would write and maintain all signalling ourselves |
| Janus | GPLv3, plugin-based C gateway, slower to deploy; strongest for SIP bridging, which Sotto doesn't need |
| Bare coturn plus peer-to-peer | Fine for 1:1, but no group calls, and peers exchange addresses unless every call is forced through the relay |

## What each party can see

| | Content (voice, video, chat in call) | Who is talking to whom in Sotto | Network addresses | When a call happens, how long, how much data |
|---|---|---|---|---|
| The other person on the call | Yes | Yes | **No** (everything goes through the server) | Yes |
| Our call server | **No** (end-to-end encrypted) | **No** (random room, random participant IDs, no Sotto identities) | **Yes, both sides** | Yes |
| Sotto relays | No | No | No (still over Tor) | Only that a padded message was sent: the call offer looks like any other message |
| Someone watching your network | No | No | They see you connect to the call server | Yes |

That third column is the trade-off of leaving Tor. It's acceptable for most calls between colleagues and many calls with outside contacts. It is not acceptable for the highest-risk contacts, so risk modes decide (below).

## How a call works

```
 Caller device                 Call broker (VPS)           LiveKit (VPS)           Callee device
 ─────────────                 ─────────────────           ─────────────           ─────────────
 1. ask for a room  ── over Tor ──▶ mint 2 one-time join tokens
                    ◀── room URL + token A + token B
 2. CallOffer ──── Sotto relays, encrypted and padded like any message ─────────────▶ 3. rings
    { call id, server URL, token B, fresh random media key, expiry }
 4. join with token A ───────────────────────────────▶ room ◀───── 5. answer: join with token B
 6. media: end-to-end encrypted on each device (key from the conversation), forwarded by the server
 7. hang up: tokens expire within minutes; the room closes 30 s after it empties; keys are wiped
```

- **The broker request goes over Tor**, so the broker doesn't learn who asked for a room. Only the media connection in step 4 reveals the caller's address, and only to our server.
- **The media key travels inside the conversation.** The caller makes a fresh random 32-byte key for each call and puts it in the offer, which the Double Ratchet encrypts end to end like any message. The server never has it. For project huddles the key will come from the project's MLS group and change when membership changes.
- **No identities on the server.** Room names and participant identities are random per call. Each token grants one room, and must be used within its join window (2 minutes, plus LiveKit's one minute of clock leeway). There are no accounts and no recordings.
- **Room limits travel inside the token.** The first person to join creates the room with its participant cap and close-when-empty timer, so the broker never talks to LiveKit and holds no state.
- **Signalling stays inside Sotto.** Offer, answer, decline and end are ordinary Sotto messages, so the relays can't tell a call from a text, and missed calls show in the conversation.
- **The person being called doesn't need a server setting.** The device that starts the call asks the broker. If an outside contact taps Call, their device asks the broker over Tor like anyone else.

## The pieces

### On the VPS (`deploy/calls/`)

- **LiveKit server** (Docker, host networking). Built-in TURN on, so calls work behind strict NAT and firewalls. Config: rooms close 30 seconds after emptying, at most 2 participants for a 1:1 room and 8 for a huddle, warning-level logs only, no webhooks, no recording or egress service installed.
- **Call broker**: a small Rust service using the `livekit-api` crate. It holds the LiveKit API secret, signs short-lived tokens for a fresh random room, and rate-limits requests. It keeps no logs of rooms, and can also be offered as an onion service so room requests never leave Tor.
- **Caddy** for TLS on `calls.<domain>` (signalling over WSS and the broker) and the TURN-over-TLS certificate.
- **Firewall:** 443/tcp (signalling, broker), 7881/tcp (media over TCP fallback), 3478/udp (TURN), 5349/tcp (TURN over TLS), and the media UDP range (or a single muxed UDP port).
- **Capacity:** voice is about 40 kbit/s per person each way; video 1:1 around 1 to 2.5 Mbit/s per person. A small VPS handles many voice calls; video is what costs bandwidth.

### In the desktop app

- **Agent (Rust):** requests rooms from the broker over Tor, derives media keys, sends and receives call messages, and tells the UI about call state over the existing SSE stream. New routes: `POST /api/contacts/{id}/call`, `POST /api/calls/{id}/answer`, `/decline`, `/end`.
- **A dedicated call window** runs the LiveKit JavaScript SDK with end-to-end encryption on (its worker ships with the app). It is a separate Electron window with its own session:
  - only this window is granted microphone and camera; the main window, which renders message text, keeps denying every permission as it does today;
  - its network allowlist is exactly the configured call server; everything else stays blocked;
  - it never shows message content, only the call.
- **Call UI:** incoming call sheet, in-call screen (mute, camera, speaker, end, connection quality in words), and a small in-call bar that stays visible across screens. Missed calls appear as messages.
- **Settings → Calls:** call server address (prefilled with ours; organizations can point at their own), a connection test, and what the server can see, in plain words.

### On phones

The LiveKit Swift and Android SDKs, both with end-to-end encryption, driven by the same agent logic over UniFFI. CallKit and ConnectionService for the system call screen. Waking a locked phone needs push (Apple or Google learn when a call arrives), so it's opt-in in Standard only.

## Risk modes

| | Standard | High risk | Extreme |
|---|---|---|---|
| Voice calls | On | On, after a one-time notice that the call server sees your network address | Off |
| Video | On | Off by default; can be turned on per call with a warning | Off |
| Wake a phone for calls | Opt-in | Off | Off |

In Extreme, the safe options are masked voice notes and text, which never leave Tor.

## Keeping the server's view small

- **Voice activity is hidden from the server.** The call window rewrites every session description it sets (`apps/desktop/src/call/sdp.ts`): the audio-level header extension (`ssrc-audio-level`, each packet's loudness in the clear) is never negotiated, and Opus runs at a constant bitrate with discontinuous transmission off (`cbr=1;usedtx=0`, set alike on every Opus line so the bundle stays valid). Who's speaking is worked out on the device from the decrypted audio. Mute silences the microphone locally without telling the server, so muting looks like nothing from outside. Measured on a live call with a tone pulsing two seconds on and two off (12 November 2026): before, packets were 151 bytes during sound and 33 during silence and the loudness header was negotiated; after, every packet was 150 bytes through sound and silence alike at a steady rate, the header was gone in both directions, mute kept the same size and rate while the other side heard silence, and the speaking ring still followed the tone.
- **Short-lived everything:** tokens expire within minutes, rooms close 30 seconds after emptying, nothing is recorded.
- **Minimal logs** on LiveKit, the broker and Caddy (no access logs), and a published statement of exactly what the server keeps, added to `docs/transparency/`.
- **Organization-hosted option:** an organization can run the same `deploy/calls/` on its own VPS and set it in Settings, so even call timing never reaches Vorynza.

## What stays from the earlier design

Metadata-honest ringing (the caller sees "Calling…", never "Ringing"; declining is silent), consensual recording only with a visible banner on both sides, masked voice notes, project huddles keyed from MLS, and no forwarding or call links anyone can join.

## Phases

1. **Server:** `deploy/calls/` (LiveKit, broker, Caddy, firewall notes) and `crates/sotto-call-broker` with tests. Built and tested locally against LiveKit 1.13.3 (12 November 2026); deploying to the VPS is the remaining step.
2. **1:1 voice on desktop:** call messages and media keys in `sotto-core`, agent routes, the dedicated call window with end-to-end encryption, call UI, missed calls, risk-mode rules. Built and tested on a live local call (12 November 2026), see below.
3. **Video** in the same window, Standard by default.
4. **Phones:** Android, then iOS.
5. **Project huddles** with MLS-derived keys, and consensual recording into the project vault.

## Phase 2 as built (12 November 2026)

- **Signalling:** `Payload::Call(CallSignal)` in `sotto-core` (offer, answer, decline, cancel, end), added last so older clients decode every earlier payload. Offers hide their token and key from debug output.
- **Engine** (`sotto-client`): one call at a time; 45 second ring; the caller joins the room right away and waits; the call goes active on the answer message or when the other person appears in the room, whichever comes first. Every ending leaves a line in the conversation that follows the disappearing timer: "Outgoing call, 4 min 12 s", "Incoming call, 35 s", "No answer", "Call cancelled", "They couldn't talk", "Missed call" (unread), "Declined call". Events never carry tokens or keys.
- **Agent:** call server setting (its own vault record), room requests to the broker through Tor when Tor is configured (HTTPS via rustls), `/api/calls/*` routes, Extreme mode refusing incoming calls before they ring ("Missed call (calls are off in Extreme mode)"), High risk mode asking for the notice once, and no auto-lock while a call is active.
- **Call window:** its own page (`call.html`) running the LiveKit client with end-to-end encryption, with the session-description rewrite that hides voice activity. In the app it's a separate window with its own session: the only place a microphone is allowed (audio only), network access limited to the one call server, and a bridge that reaches only that call's routes. Closing it hangs up. Mute (M, local only), speaking shown from the decrypted audio, microphone and speaker choice, connection quality in words, reconnecting, and clear errors for a blocked microphone or an unreachable server. Repeated decryption failures end the call rather than leaving a broken one open.
- **Main window:** Call button in the conversation header, an incoming call sheet with a soft ringtone (Answer, Decline, or decline and say you can't talk now), an in-call bar across the app, the High risk notice, and Settings, Calls with a connection test.

Tested on this machine with LiveKit 1.13.3 and the broker, between two development devices: the call rang, was answered, and audio flowed both ways encrypted and decoding cleanly (Opus, about 7.5 KB/s, full test-tone level, 436 concealed samples out of about a million); mute, hang-up from either side and the call records worked; Extreme mode refused calls in both directions. Unit tests cover the full call lifecycle between two engines over a real relay. Not yet tested: the Electron call window itself (the browser preview ran the same page as a popup), a real microphone, and a call over the internet through a deployed server.

## Group calls (12 November 2026)

Any call can grow to 8 people: every room is created with space for 8, the broker's `POST /v1/rooms/{room}/tokens` hands out more join tokens for a room to anyone who knows its unguessable name, and the call window's Add people sends ordinary encrypted invitations with the same media key. Names travel end to end encrypted inside the call. Details and test results: [groups.md](groups.md).

## Why not Tor (Phase 0, 10 November 2026)

The Tor-only design (each call a one-time onion service on the caller's device, constant-rate voice) was measured with `crates/sotto-call-spike`: two Tor 0.4.9.11 instances, a fresh client-authorized onion per call, one Tor cell every 40 ms each way.

| Run | Calls | Connected within 20 s of answering | Mouth-to-ear (median of calls) | Late frames (median) |
|---|---|---|---|---|
| One path, every frame queued | 12 | 58% (median 16.2 s) | 1,382 ms (best 921 ms) | 27% |
| Engine: three paths, constant rate, slow paths replaced | 3, stopped | 16 to 65 s | over 2 s | 50 to 89% |

The test machine's Wi-Fi (about 250 ms base latency, bursts of 35% loss) inflated these numbers, but even the best call was over the 700 ms target, connecting took 16 s, and redundancy made weak links worse. Live calls over Tor stay a possible future option for the highest-risk users; masked voice notes cover that need today. The spike crate is kept as the record of this measurement.

## Decisions for the owner

1. **High risk mode:** allow voice calls after a one-time notice (recommended), or limit calls to Standard only?
2. **Broker abuse control:** anonymous rate-limited rooms (built this way for now), or require organizations to hold a broker key?
3. **Video at launch:** include video in the first release on desktop, or ship voice first and add video in the next phase (recommended)?

## References

- LiveKit encryption overview: https://docs.livekit.io/transport/encryption/
- LiveKit Rust SDKs (`livekit` and `livekit-api` crates): https://github.com/livekit/rust-sdks
- `livekit` crate end-to-end encryption module: https://docs.rs/livekit/latest/livekit/e2ee/index.html
- Comparison of open-source media servers: https://webrtc.ventures/2026/06/open-source-webrtc-media-servers/
- Self-hosted SFU guide (Janus, mediasoup, LiveKit): https://www.pistack.xyz/posts/2026-04-25-janus-gateway-vs-mediasoup-vs-livekit-self-hosted-webrtc-sfu-guide-2026/
