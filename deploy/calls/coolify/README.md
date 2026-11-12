# Sotto call server on Coolify

This sets up the server that Sotto's voice and video calls run through. It's one Docker Compose file (`docker-compose.yml`) with two services:

- **livekit**: the call server (LiveKit 1.13.3, open source). It forwards encrypted audio and video between callers and has a built-in TURN relay for difficult networks.
- **broker**: a small service from the Sotto team that hands out short-lived passes to join a call. It builds from source the first time you deploy; the source is embedded in the compose file, so nothing else needs downloading from us.

Calls are end-to-end encrypted on the callers' devices. This server can't hear or see them, and it keeps no record of past calls.

## What you need

- Coolify v4 on a server with a public IPv4 address.
- Two DNS `A` records pointing at that server, for example:
  - `calls.example.org`: the broker (this is the address the Sotto app uses)
  - `rtc.example.org`: LiveKit
- These ports open to the internet, in the cloud provider's firewall (for example a Hetzner Cloud Firewall) as well as on the server:

  | Port | Protocol | Used for |
  |---|---|---|
  | 7881 | TCP | Call media when UDP is blocked |
  | 7882 | UDP | Call media |
  | 3478 | UDP | TURN relay |
  | 30000 to 30200 | UDP | TURN relay ports |

  Ports 80 and 443 are Coolify's proxy, which you already have. Note that Docker-published ports bypass `ufw` on the server itself, so the cloud firewall is what actually controls them.

## Set it up

1. In Coolify, open your project, choose **New Resource**, then **Docker Compose Empty**, paste the whole `docker-compose.yml`, and save.
2. Set the domains on the two services. The `:port` part tells Coolify which container port to send traffic to; Coolify handles HTTPS:
   - **livekit**: `https://rtc.example.org:7880`
   - **broker**: `https://calls.example.org:7890`
3. Open **Environment Variables** and set:
   - `LIVEKIT_PUBLIC_URL` = `https://rtc.example.org`

   Coolify generates `SERVICE_USER_LIVEKIT` and `SERVICE_PASSWORD_64_LIVEKIT` by itself. If either is empty, set `SERVICE_USER_LIVEKIT` to any random 16 letters and `SERVICE_PASSWORD_64_LIVEKIT` to 64 random letters and digits.
4. **Deploy.** The first deploy compiles the broker, which takes about 5 to 10 minutes. Later deploys reuse it.

## Check it

```
curl -s https://calls.example.org/healthz
```

should print `ok`, and

```
curl -s -X POST https://calls.example.org/v1/rooms
```

should print JSON with `"url":"wss://rtc.example.org"` and two long `tokens`. Each one expires within about three minutes and only opens an empty throwaway room, but there's no need to share the output.

## Send back

Only the broker address, for example `https://calls.example.org`. Please don't send the generated keys or secrets: the Sotto app never needs them. The Sotto team then runs a full test against the server from outside.

## Please keep it private

- Don't add log drains, monitoring agents or access logging for these two services. The broker writes one line when it starts and nothing per request; LiveKit logs warnings only. Coolify's proxy doesn't write access logs unless someone turns them on.
- Keep the generated `SERVICE_PASSWORD_64_LIVEKIT` secret. Anyone holding it can create rooms on this server.

## If something goes wrong

| What you see | What to do |
|---|---|
| broker keeps restarting, log says the secret must be at least 32 characters | `SERVICE_PASSWORD_64_LIVEKIT` is empty: set it as in step 3 and redeploy |
| broker log says `the LiveKit address has no host name` | `LIVEKIT_PUBLIC_URL` is empty: set it (step 3) and redeploy |
| The deploy fails on `dockerfile_inline` | The server's Docker Compose is too old. Update Docker, or build the broker by hand on the server with `docker build -t sotto-call-broker:1 -f broker.Dockerfile .` (the file is next to this README), then replace the broker's `build:` block in Coolify with `image: sotto-call-broker:1` |
| Calls connect but there's no sound or picture | UDP 7882 or 3478 is blocked: check the cloud firewall |
| `/v1/rooms` answers 429 | Rate limit: wait a minute. The limits are `BROKER_RATE_GLOBAL_PER_MIN` and `BROKER_RATE_IP_PER_MIN` in the compose file |

## Updating

When the Sotto team sends a new `docker-compose.yml`, paste it over the old one in Coolify (your domains and environment variables stay) and redeploy.
