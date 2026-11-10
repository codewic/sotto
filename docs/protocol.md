# Sotto protocol, version 1

Status: implemented in `crates/sotto-core` (wire formats, handshake, ratchet, sealed boxes), `crates/sotto-relay` (relay) and `crates/sotto-client` (client engine). Not yet externally reviewed. Every design choice below follows a published construction; where Sotto adapts one, the adaptation is called out so a reviewer can find it quickly.

## 1. Primitives

| Purpose | Primitive | Crate |
|---|---|---|
| Diffie-Hellman | X25519 | `x25519-dalek` 3 |
| Signatures | Ed25519 | `ed25519-dalek` 3 |
| Post-quantum KEM | ML-KEM-768 (FIPS 203) | `ml-kem` 0.3 |
| AEAD | XChaCha20-Poly1305 | `chacha20poly1305` 0.11 |
| KDF | HKDF-SHA-256 | `hkdf` 0.13 |
| MAC | HMAC-SHA-256 | `hmac` 0.13 |
| Password hashing | Argon2id v1.3 | `argon2` 0.6 |
| Serialization | postcard (serde) | `postcard` 1 |
| Randomness | OS CSPRNG via `getrandom` | `getrandom` |

All secrets are zeroized on drop where the type allows it.

## 2. Identities

An identity is a pair of long-term keys held only on the device:

- `IK.sign`: Ed25519, signs invites and join transcripts.
- `IK.dh`: X25519, used in the handshake.

A user can hold many identities. Journalists normally have one *listed* identity (published through the outlet directory, see `docs/directory.md`) and any number of *unlisted* ones. Sources get a fresh identity per contact by default, so two journalists comparing notes cannot link the same source by key.

**Safety number.** `SHA-512("sotto-safety-v1" || min(A.sign, B.sign) || max(A.sign, B.sign))`, first 40 bytes read as eight 5-byte big-endian integers, each `mod 100000`, shown as eight groups of five digits. Both sides see the same number.

## 3. Invites (out-of-band first contact)

There is no user directory and no prekey server. First contact is a one-time invite passed out of band: QR code in person, a link through an existing channel, or a reply on a source drop.

```
Invite {
  version: 1,
  identity: { sign_pk[32], dh_pk[32] },
  ephemeral_pk[32],          // one-time X25519 key, plays the PQXDH signed-prekey role
  kem_pk[1184],              // one-time ML-KEM-768 encapsulation key
  queue: { relay: String, snd_id[16] },   // inviter's receive queue (send side)
  label: String,             // optional display hint, chosen by inviter
  expires_at: u64,           // unix seconds
  sig[64]                    // Ed25519 by identity.sign over "sotto-invite-v1" || postcard(all fields above)
}
```

Encoded as `sotto://invite/<base64url(postcard(Invite))>`. `invite_id = SHA-256(ephemeral_pk)[0..16]`.

## 4. Handshake (PQXDH adapted to one-time invites)

Follows Signal's PQXDH (revision 3), with the invite standing in for the prekey bundle. Because an invite is single use, the ephemeral key is both the signed prekey and the one-time prekey, so `DH4` is omitted.

Joiner B, holding invite from A:

```
EK_B = fresh X25519
DH1 = DH(IK_B.dh, EPK_A)
DH2 = DH(EK_B,    IK_A.dh)
DH3 = DH(EK_B,    EPK_A)
(CT, SS) = ML-KEM-768.Encaps(KEM_A)
SK = HKDF-SHA-256(salt = 0^32,
                  ikm  = 0xFF^32 || DH1 || DH2 || DH3 || SS,
                  info = "Sotto_PQXDH_X25519_SHA-256_ML-KEM-768_v1")[0..32]
AD = "sotto-ad-v1" || IK_A.sign || IK_A.dh || IK_B.sign || IK_B.dh
transcript_sig = Ed25519(IK_B.sign, "sotto-join-v1" || invite_id || IK_B.dh || EK_B || CT)
```

B initialises the Double Ratchet as the first sender with `SK` and remote ratchet key `EPK_A`, creates its own receive queue, and sends a `Join` envelope to A's queue whose first ratchet message carries `Payload::Hello { reply_queue, profile }`.

A looks up the invite by `invite_id`, verifies `transcript_sig`, recomputes `SK`, initialises the ratchet as receiver with the `EPK_A` key pair, decrypts, and stores B's reply queue. A then deletes `EPK_A` and the KEM secret: the invite cannot be used twice.

Adaptations, for reviewers: binding B's signing key with `transcript_sig` is an addition (Signal uses XEdDSA with a single key). Post-quantum protection covers the initial secret; the ratchet itself is classical (see section 12).

## 5. Double Ratchet

Signal's Double Ratchet specification, without header encryption.

- `KDF_RK(rk, dh) = HKDF(salt = rk, ikm = dh, info = "SottoRatchetRoot")` gives 64 bytes: new root key, chain key.
- `KDF_CK(ck)`: `mk = HMAC(ck, 0x01)`, `ck' = HMAC(ck, 0x02)`.
- Message encryption: `HKDF(salt = 0^32, ikm = mk, info = "SottoMessageKeys")` gives a 32-byte key and 24-byte nonce for XChaCha20-Poly1305 with associated data `AD || header`.
- Header: `{ dh_pk[32], pn: u32, n: u32 }`.
- Skipped message keys: at most 1,000 per chain step and 2,000 stored in total; the oldest are evicted first.
- State changes are applied to a copy and committed only after successful decryption.

## 6. Envelopes and padding

```
Envelope = Join { invite_id[16], ik: {sign,dh}, ek[32], kem_ct[1088], sig[64], msg: RatchetMessage }
         | Msg(RatchetMessage)
RatchetMessage = { header, ciphertext }
```

Every envelope is padded to exactly `BLOB_SIZE` bytes before it reaches a relay (`u32 little-endian length || data || zeros`). The relay therefore sees blobs of one size only. The decrypted payload is one of `Hello`, `Text`, `File`, `Receipt`, `Delete`, `QueueRotate`, `Profile`, and the case messages `KeyPackageRequest`, `KeyPackage`, `CaseInvite` (section 10).

## 7. Relay protocol

A relay is a store-and-forward server reachable as a Tor onion service. Clients speak WebSocket; every binary message is exactly `FRAME_SIZE = 16,384` bytes, or `FILE_FRAME_SIZE = 278,528` bytes for file chunks. Frames are `u32 LE length || postcard(Frame) || zeros`.

On connect the relay sends `Hello { version, nonce[32] }`. Commands that prove ownership are signed:

`sig = Ed25519(key, "sotto-relay-v1" || nonce || tag || fields)`

so a captured signature cannot be replayed on another connection.

| Command | Purpose | Auth |
|---|---|---|
| `New { rcv_auth }` | create a one-way queue; returns `rcv_id`, `snd_id` (unrelated random 16-byte ids) | none |
| `Send { snd_id, blob }` | enqueue a blob | possession of `snd_id` |
| `Sub { rcv_id }` | stream messages for a queue on this connection (live push) | `rcv_auth` signature |
| `Ack { rcv_id, msg_id }` | delete a delivered message, push the next one | signature |
| `Del { rcv_id }` | delete a queue | signature |
| `GNew` / `GSend` / `GSub` | ordered group queue for cases: members hold `write_id`/`read_id` capabilities, messages kept until TTL, delivered in relay order with sequence numbers | capability |
| `FPut { read_auth, max_reads, data }` / `FGet { fid }` / `FDel` | encrypted file chunks | signature for get and delete |
| `TNew` / `TCheckIn` / `TGet` / `TDel` | time-locked blobs for the insurance file (section 11) | owner signature; `TGet` only after the deadline |
| `Ping` / `Cover` | keepalive and cover traffic; relay answers `Cover` with `Cover` | none |

Relay rules: message data lives in memory only; undelivered messages expire after the receiver's TTL (default 14 days, max 30); no request logging; per-queue and global memory caps. Two things are written to the relay's data directory so service survives a restart: `meta.bin` (for each queue its receive id, send id, auth public key and TTL; for each group its ids, TTL and a sequence floor) and already-encrypted time-locked packages. Group sequence numbers are reserved in blocks of 1,024, so after a restart they keep growing without a disk write per message.

A relay learns: that some Tor client created queue Q, that some Tor client sent a blob to Q's send id, and that some Tor client holding the right key read it. It does not learn identities, contacts, IP addresses, message sizes or content.

## 8. Files

1. Random 32-byte file key `FK`.
2. Plaintext split into 256 KiB chunks; chunk *i* sealed with XChaCha20-Poly1305 under `HKDF(FK, info = "SottoFileChunk" || i)` with a zero nonce (each chunk key is used once). Every chunk is padded to full size and carries its true length.
3. Each chunk uploaded with a fresh Ed25519 read key; `max_reads` is 1 for a contact and the member count for a case.
4. The `File` payload carries the relay, chunk ids, read keys, `FK`, size and SHA-256. If the chunk list does not fit in one envelope it is itself uploaded as a manifest file (one level of indirection).

Received files are stored encrypted in the vault and marked *quarantined* until safe review produces a clean copy.

## 9. Hybrid sealed box (drop and backups)

Used where the sender has no session: drop submissions, replies to sources, travel-mode backups, insurance files.

```
EK = fresh X25519
(CT, SS) = ML-KEM-768.Encaps(R.kem)
K = HKDF(salt = 0^32, ikm = DH(EK, R.dh) || SS, info = "SottoSeal_v1" || EK_pk || CT || R.dh)
```

The payload is sealed in 64 KiB chunks with the STREAM construction (nonce = 19-byte prefix from HKDF || u32 counter || last-chunk flag), so large files never sit in memory whole and truncation is detected.

## 10. Cases (groups)

Each case is an MLS group (RFC 9420, ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`) via OpenMLS. The relay group queue is the delivery service: it gives every member the same order, which MLS needs for commits. Key packages and Welcomes travel over the existing 1:1 ratchet channel. Only the case owner's commits are accepted by clients. Application messages carry notes, file references, source pseudonyms, role changes and audit entries.

## 11. Insurance file

The owner encrypts a case package (notes and source pseudonyms) under a random key `K` in file-chunk format, creates a time lock with `TNew { owner, deadline }`, uploads the chunks with signed `TPut`, and sends each chosen colleague `InsuranceGrant { read_id, K, deadline }` over the 1:1 ratchet channel. The relay holds only ciphertext and refuses `TGet` until the deadline; colleagues hold only the key. Neither can read the package early. `TCheckIn` (owner signature) moves the deadline forward; every unlock of the owner's Sotto checks in automatically, and colleagues receive `InsuranceUpdate` with the new deadline.

## 11a. Travel mode

The desk generates 32 random bytes, shown once as a 24-word BIP-39 phrase (the user confirms three words). The vault directory (`vault.hdr`, `records/*`, `blobs/*`, all already encrypted) is written as `u16 path length || path || u64 size || bytes` records into a hybrid sealed stream to `SealSecret::from_seed(entropy)`, saved to a path the user chooses, and the device is then panic-wiped. Restore parses the whole archive, accepts only paths a vault contains, writes records and blobs first and the header last, then the user unlocks with their original passcode.

## 12. Known limits and planned work

- The ratchet is classical after the post-quantum handshake. Planned: a sparse post-quantum ratchet once a reviewed specification and implementation exist.
- No header encryption; queue pairing already hides the social graph from the relay, but header encryption is planned for defence in depth.
- Queue rotation (`QueueRotate`) is defined and handled; automatic rotation every 7 days is planned.
- Relay restarts drop undelivered messages; clients mark messages delivered only on a `Receipt`.
- The drop's no-JavaScript mode encrypts on the server at arrival (the SecureDrop model). Browser-side encryption when JavaScript is allowed is planned.
