# Tags, avatars and the default wallpaper

Status: built and tested (12 November 2026).

## Tags: invite people without a link

A tag is a short name like `@ada`. Anyone can type it in **Add someone** and connect to you, instead of you sending them a one-time link.

- **On your relay** (usually your organization's), `@ada` is enough.
- **From another relay**, people use your **full tag**: the tag, then your relay, like an email address: `ada@relay.example.org` (with a port if your relay uses one, `ada@relay.example.org:7400`). Settings and Invite someone show your full tag, and Copy copies it. A tag is unique per relay, so `ada@north.example` and `ada@south.example` are different people.

**How it works, and why it isn't an account.** Your device claims the tag on your relay and keeps about 10 ordinary one-time invites stocked under it, each made with a fresh identity and fresh mailbox, exactly like an invite link. Someone who types your tag takes one invite from the relay and connects through it. Your device tops the stock back up when one is used, when you unlock, and twice a day.

- **Requests.** People who connect by tag arrive marked "Wants to connect (found you by @ada)", with a banner to accept or remove them. Removing deletes the conversation and closes its mailbox; they aren't told why.
- **Ownership.** The first device to claim a tag on a relay owns it, by a signing key only that device holds. Nobody else can add invites under it or release it. Released tags are free to claim again.
- **Rules.** 3 to 24 characters: lowercase letters, numbers, dots and underscores, starting with a letter. `@Ada.M` and `ada.m` are the same tag.
- **What the relay learns.** That the tag exists and how often its invites are taken. It never learns who took one or who you are; everything after that is the same end-to-end encrypted handshake as a link.
- **Limits and choices.** Tags are optional. A tag can be released at any time (Settings, Identity). In Extreme mode you can't claim one, because anyone who knows it can ask to reach you; use one-time links there. For someone who shouldn't know your tag, **Invite someone** still makes a one-time link.
- **Other relays.** Looking up a full tag connects to that relay, under the same rules as any relay someone else names: it must be a real relay address, and on a device set to use only Tor it must be an onion service. That relay learns a lookup happened, just as Ada's own relay would. A relay that doesn't answer within 20 seconds gives a clear error rather than a long wait. After the lookup, the conversation works across relays exactly as one started from a link does.
- **Abuse.** Taking an invite counts heavily against a connection's request budget at the relay, so one connection can't drain a tag. If a tag is drained anyway, it stops handing out invites until your device restocks; your device never keeps more than 60 unused invites open.

Relay commands: `TagAdd` (signed by the tag's owner over the tag and a digest of the invites; an empty list reports how many are left), `TagTake`, `TagDel` (signed). Tags are saved in the relay's data folder so they survive a restart.

## Avatars

Everyone gets a unique avatar by default, made from a stable seed (their identity fingerprint): one of 20 hand-drawn shapes, in two tones of one of 12 hues, turned one of four ways. Lightness and colour strength come from theme tokens, so avatars sit right in light and dark themes. The same person always gets the same avatar on your device.

**Choosing your own.** Settings, Identity, Avatar, Change: pick a colour, a shape (Orbit, Peaks, Tide, Halves, Stack, Leaf, Kite, Arch, Bloom, Crescent, Comet, Cell, Bolt, Sprout, Rings, Dots, Sail, Prism, Spark, Steps), turn it, or press Surprise me. That's 960 avatars in the collection.

Only the choice travels, as a short id (`v1:shape:hue:turn`) in an end-to-end encrypted message to each contact, including people who connect later. It's never an image, so it can't carry a payload, a tracking pixel or anything that identifies a device. Anything that isn't a valid id from the collection is ignored. "Use my default" goes back to the generated one.

## Default wallpaper

New installs show the Contours wallpaper behind conversations: drawn from theme tokens, quiet enough to read over. Anyone who already chose a wallpaper (including None) keeps their choice. Change it in Settings, Appearance.

## Tests

- `tags_let_people_connect_without_a_link`: claim (normalised), stock hidden from the list, a taken tag refused, an invalid tag refused, connect by tag, the request with the other person's name, messages straight away, accept, restock to 10, a second request removed, release, and the tag free for someone else.
- `tags_work_across_relays`: two relays; the full tag names the owner's relay; a bare tag only looks on your own relay; the full tag connects across relays, the other side gets the request and the message; an empty relay part is refused.
- `chosen_avatars_reach_contacts`: only collection ids accepted, a choice reaches an existing contact and someone who connects later, and going back to the default.
- Live in the app on three desks: the avatar picker, the choice showing on another desk, claiming `@ada` from Invite someone, adding `@ada` from Add someone on another desk, the request banner, accepting, and avatars and wallpaper in dark and light themes.
