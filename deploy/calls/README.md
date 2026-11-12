# Deploying the Sotto call server

**On a server with Coolify?** Use [coolify/](coolify/README.md) instead: one self-contained compose file to paste into Coolify, no repository access needed. `coolify/make.py` regenerates it from the broker source.

One VPS runs three things with Docker Compose:

| Service | What it does | Reachable at |
|---|---|---|
| LiveKit 1.13.3 | Forwards end-to-end encrypted audio and video; built-in TURN relay | UDP and TCP media ports below |
| Call broker | Hands out short-lived join tokens for throwaway rooms | `https://<domain>/v1/rooms` (and optionally an onion address) |
| Caddy | TLS certificates and the HTTPS front door | 80 and 443 |

Design and threat model: [docs/calls.md](../../docs/calls.md). What the server can see is in [docs/transparency/call-server.md](../../docs/transparency/call-server.md).

## 1. Before you start

- A VPS with a public IPv4 address, Docker and the Compose plugin (`docker compose version` must work).
- A DNS `A` record for the call domain (for example `calls.example.org`) pointing at the VPS.
- About 40 kbit/s per person for voice and 1 to 2.5 Mbit/s per person for video, each way.

## 2. Firewall

```
ufw default deny incoming
ufw allow 22/tcp
ufw allow 80/tcp            # certificate issuance
ufw allow 443/tcp           # broker and LiveKit signalling (through Caddy)
ufw allow 7881/tcp          # media over TCP when UDP is blocked
ufw allow 7882/udp          # media
ufw allow 3478/udp          # TURN
ufw allow 30000:30200/udp   # TURN relay ports
ufw enable
```

Ports 7880 (LiveKit signalling) and 7890 (broker) must stay closed: Caddy reaches them on 127.0.0.1. LiveKit runs with host networking, so these host rules apply to it.

## 3. Configure and start

```
cd deploy/calls
./setup.sh calls.example.org ops@example.org
docker compose up -d --build
```

`setup.sh` writes `.env` with a fresh random API key and secret (readable only by you) and renders `livekit.yaml` for your domain. Both files stay out of version control. The first start builds the broker image from this repository, which takes a few minutes.

## 4. Check it

From any machine with this repository:

```
curl -s https://calls.example.org/healthz
curl -s -X POST https://calls.example.org/v1/rooms | cargo run -p sotto-call-broker --example smoke
```

The smoke test joins the new room once per token and expects every participant to get in, the room limit to match the number of tokens, and a TURN relay to be offered. It prints `smoke: OK` and never prints tokens or relay credentials.

## 5. Point Sotto at it

The desktop app's call server setting (Settings, Calls) is `https://calls.example.org`. Outlets that run their own copy use their own domain.

## Optional: broker over Tor

To let Sotto ask for rooms without leaving Tor, append `torrc` to `/etc/tor/torrc` on the VPS, restart tor, and use the onion address from `/var/lib/tor/sotto-call-broker/hostname` as the broker address. Media still goes to the public call server.

## Optional: TURN over TLS

For networks that only allow TLS, uncomment the `tls_port`, `cert_file` and `key_file` lines in `livekit.yaml` once Caddy has obtained the certificate, open `5349/tcp`, and run `docker compose restart livekit`. LiveKit reads the certificate at start, so restart it after each renewal (a monthly `docker compose restart livekit` from cron covers it).

## Behaviour worth knowing

- **Join window.** Tokens are valid for `BROKER_JOIN_WINDOW_SECS` (default 120). LiveKit also allows about one minute of clock leeway, so a token is refused roughly a minute after it expires (measured locally: accepted 36 s after a 30 s window, refused at 76 s). Once someone has joined, the call continues past the window.
- **Room limits travel in the token.** The first person to join creates the room with the participant cap (2 for a call, up to 8 for a huddle) and a 30 second close-when-empty timer. The broker never talks to LiveKit.
- **Tokens are short-lived, not single-use.** Reusing a token within its window replaces that participant's earlier connection. Tokens travel only inside end-to-end encrypted Sotto messages.
- **Rate limits.** `BROKER_RATE_GLOBAL_PER_MIN` (default 120) for everyone together, `BROKER_RATE_IP_PER_MIN` (default 20) per address. Requests arriving over Tor share exit addresses, so the global limit is the real backstop.
- **Logs.** LiveKit logs warnings only, the broker logs one line at start, Caddy writes no access log, and Docker keeps at most 2 MB of logs per service.

## Updating

Change the LiveKit image tag in `docker-compose.yml`, then `docker compose pull && docker compose up -d`. Rebuild the broker after pulling new code with `docker compose up -d --build broker`.

## Tested so far

On a development machine: LiveKit 1.13.3 with this exact `livekit.yaml` (TURN enabled), the broker, and the smoke test, including a tampered token (refused with 401) and an expired one (refused after the leeway). Not yet tested: the Compose file and the broker image as a whole, and Caddy with a real certificate. Those run on the first VPS deploy.
