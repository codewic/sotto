# What the Sotto call server can see

Applies to the call server Vorynza hosts. Organizations that run their own (`deploy/calls/`) hold the same data on their own machine.

Calls leave Tor: unlike relays and anonymous inboxes, the call server has a public address and sees the network addresses that connect to it. Calls are off in Extreme mode for this reason.

| | The call server sees | It never sees |
|---|---|---|
| Content | Encrypted audio, video and in-call data packets | Voice, video or text: keys come from the Sotto conversation and never reach the server |
| People | A random room name and a random participant id per call | Names, Sotto identities, contacts, phone numbers or accounts (there are none) |
| Network | The IP address and port of each device in a call, and of anyone asking the broker for a room without Tor | Which Sotto conversation a call belongs to |
| Timing | When each participant joins and leaves, and how much data flows | When anyone speaks: voice is sent at a constant rate |

## What it keeps

- **LiveKit:** room state in memory while a call is active; the room is removed 30 seconds after the last person leaves. Logs are warning-level only (no per-call records).
- **Call broker:** nothing. It signs tokens and logs one line when it starts.
- **Caddy:** TLS certificates. No access logs.
- **Docker:** at most 2 MB of service logs per service, overwritten as they fill.
- **Recordings:** none. No recording or egress service is installed.

## What we could produce if compelled

Nothing about past calls beyond what may be in the last 2 MB of warning logs. For a call in progress, a compelled operator could see the participants' IP addresses and the call's timing, and could not decrypt it.
