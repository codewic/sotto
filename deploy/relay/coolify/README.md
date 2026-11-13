# Sotto relay on Coolify

This sets up the relay that Sotto apps connect to by default (`wss://relay.cmigl.org`). It carries sealed messages between mailboxes: it can't read them, and it can't see who sends to whom. It runs next to the call server on the same Coolify server.

`docker-compose.yml` installs `sotto-relay` from a signed Sotto release. `make.py` writes it: it checks that release's signed manifest against the Sotto release key and puts the tools archive's SHA-256 into the file, so the build stops if the download differs by a single byte.

## What you need

- Coolify v4 on a server with a public IPv4 address (the call server's, `84.8.131.240`, is fine).
- A DNS `A` record: `relay.cmigl.org` pointing at that server.
- Nothing else open: the relay goes through Coolify's proxy on ports 80 and 443, which you already have.

## Set it up

1. For a new release, regenerate the file first: `python3 deploy/relay/coolify/make.py vX.Y.Z` (it needs `cargo`, to check the signature).
2. In Coolify, open your project, choose **New Resource**, then **Docker Compose Empty**, paste the whole `docker-compose.yml`, and save.
3. Set the domain on the **relay** service to `https://relay.cmigl.org:7400`. The `:7400` tells Coolify which container port to send traffic to; Coolify handles HTTPS, and WebSockets pass through it as they are.
4. **Deploy.** It downloads the release and starts in under a minute.

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

Run `make.py` for the new release, paste the new file over the old one in Coolify, and redeploy. The volume, and with it everyone's mailboxes, stays.
