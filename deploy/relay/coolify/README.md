# Sotto relay on Coolify

This sets up the relay that Sotto apps connect to by default (`wss://relay.cmigl.org`). It carries sealed messages between mailboxes: it can't read them, and it can't see who sends to whom. It runs next to the call server on the same Coolify server.

Everything Coolify needs is in one file, `docker-compose.yml` in this folder. It installs `sotto-relay` from a signed Sotto release and refuses to build if the download differs by a single byte. Use it as it is; don't edit it.

## What you need

- Coolify v4 on a server with a public IPv4 address (the call server's, `84.8.131.240`, is fine). Intel/AMD (x86_64) and ARM servers both work: the build picks the right one by itself.
- A DNS `A` record: `relay.cmigl.org` pointing at that server.
- Nothing else open: the relay goes through Coolify's proxy on ports 80 and 443, which you already have.

## Set it up

1. In Coolify, open your project, choose **New Resource**, then **Docker Compose Empty**, paste the whole `docker-compose.yml`, and save.
2. Set the domain on the **relay** service to `https://relay.cmigl.org:7400`. The `:7400` tells Coolify which container port to send traffic to; Coolify handles HTTPS, and WebSockets pass through it as they are.
3. **Deploy.** It downloads the release and starts in under a minute.

## Check it

- A WebSocket handshake should get `101`:

  ```
  curl -sS -m 5 --http1.1 -o /dev/null -w "%{http_code}\n" -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" https://relay.cmigl.org/
  ```

  (curl then waits on the open connection until `-m 5` ends it; the `101` is what matters. A plain request gets no answer, which the proxy shows as `502`.)
- In Sotto, the sidebar should read "Connected directly" within a few seconds of unlocking, and Settings, Connection should show the relay as Online.

## What it keeps

The `relay-data` volume holds mailbox and group ids with their public keys and time limits (so conversations survive a restart), claimed tags, and already-encrypted insurance packages. Message content stays in memory and is never written to disk. Logs are capped at 2 MB and the relay itself prints one line at start-up.

## Updating

When a release changes the relay, `docker-compose.yml` in this folder is updated with it. Paste the new file over the old one in Coolify and redeploy. The volume, and with it everyone's mailboxes, stays.
