# Sotto transparency report: [period]

Published by Vorynza for its hosted Sotto relays and anonymous inboxes. Self-hosted relays and anonymous inboxes are run by the organizations that host them and are not covered here.

## Requests received

| Type | Received | Complied | Data produced |
|---|---|---|---|
| Subpoenas or court orders for user data | 0 | 0 | none |
| Requests to preserve data | 0 | 0 | none |
| Requests to remove or block content | 0 | 0 | none |
| Gag orders | see the warrant canary | | |

## What we could have produced

Hosted relays hold, for each queue: a random id, a public key and a time-to-live. They hold message blobs in memory only, deleted on delivery or expiry, and already-encrypted insurance packages. They keep no request logs and never see IP addresses, because they are reachable only as Tor onion services. There are no accounts, phone numbers or email addresses to look up.

The hosted call server holds no records of past calls. While a call is active it sees the participants' IP addresses and the call's timing, never its content, which is end-to-end encrypted. Details: `call-server.md`.

## Changes this period

[Infrastructure, jurisdiction or policy changes that affect what we hold.]

## Contact

[Address for legal process and for questions about this report.]
