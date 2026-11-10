# Deploying a Sotto relay and a source drop

Both services bind to localhost and are reachable only as Tor onion services. Neither has a public IP endpoint.

## Relay

1. Install `tor` and build `sotto-relay` (`cargo build --release -p sotto-relay`), then copy the binary to `/usr/local/bin/`.
2. Create the service user and data directory:
   ```
   useradd --system --home /var/lib/sotto-relay --shell /usr/sbin/nologin sotto-relay
   install -d -o sotto-relay -m 0700 /var/lib/sotto-relay
   ```
3. Append `relay/torrc` to `/etc/tor/torrc` and restart tor. The onion address is in `/var/lib/tor/sotto-relay/hostname`; clients use `ws://<that address>`.
4. Install `relay/sotto-relay.service` to `/etc/systemd/system/` and run `systemctl enable --now sotto-relay`.

What the relay keeps on disk (`/var/lib/sotto-relay`): `meta.bin` with queue and group ids, their auth public keys and TTLs (needed so conversations survive a restart), and `timelocks/` with already-encrypted insurance packages. Message content never touches the disk. Put the directory on an encrypted volume anyway.

## Source drop

1. Build `sotto-drop` and copy it to `/usr/local/bin/`.
2. `sotto-drop example-config > /etc/sotto/drop.json`, then fill in `name`, `newsroom_key` and `journalists` from the Sotto desktop app (Drop, Drop keys). Set `verify_hint` to the page on your normal website where sources can check this onion address.
3. Append `drop/torrc` to `/etc/tor/torrc`, restart tor, and publish the onion address from `/var/lib/tor/sotto-drop/hostname` on that page of your website.
4. Install `drop/sotto-drop.service` and enable it.

The drop keeps only sealed submissions and sealed replies, with dates rounded to the day. Desks delete submissions after downloading them unless that setting is turned off.

## Denial of service

Every onion client looks the same to the service, so per-address limits do nothing. Both torrc templates turn on Tor's onion service proof-of-work defence (`HiddenServicePoWDefensesEnabled 1`), which makes floods expensive for attackers while normal visitors notice nothing. The drop also limits codename attempts to 30 per minute in total.

## Where Tor is blocked (pluggable transports)

Journalists and sources in censored networks connect through Tor bridges. Sotto uses whatever tor process runs on the device, so bridges are configured in tor itself:

- Tor Browser: Settings, Connection, Bridges (choose obfs4 or Snowflake). Then point Sotto at Tor Browser's SOCKS port `127.0.0.1:9150`.
- The tor service: add to `/etc/tor/torrc`
  ```
  UseBridges 1
  ClientTransportPlugin obfs4 exec /usr/bin/obfs4proxy
  Bridge obfs4 <bridge line from https://bridges.torproject.org>
  ```
  and keep Sotto on `127.0.0.1:9050`.

Each relay connection uses its own SOCKS credentials, so tor puts every relay on a separate circuit.

## Outlet directory

Publish your reporters' listed identity fingerprints at `https://<your domain>/.well-known/sotto.json` so sources can check them:

```
sotto-directory keygen                                  # once; keep the seed offline
sotto-directory sign --seed <seed> --in draft.json > sotto.json
sotto-directory verify --domain <your domain> --in sotto.json
```

The draft lists `outlet`, `domain`, `reporters` (name and fingerprint from each reporter's Settings, Identity) and `drops`.
