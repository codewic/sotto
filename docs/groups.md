# Groups, channels and group calls

Status: group calls, groups and channels are built and tested (12 November 2026).

Telegram and Discord set the bar for how groups feel: instant, lively, every action one gesture away. Sotto matches that in its own design language (DESIGN_PRINCIPLES section 2), with one rule Telegram and Discord don't have: **nobody learns who else is listening unless they need to.**

## Three things, each built for its job

| | Group | Channel | Group call |
|---|---|---|---|
| What it's for | A team talking together: an incident team, a deal team, legal and the lead | One-way updates from a team to many people: security notices to staff, status updates during an incident, alerts to outside contacts | Talking live, up to 8 people |
| Who can post | Everyone | The channel's owners | Everyone speaks |
| Who sees the members | Every member sees every member | Nobody but the owners; subscribers never see each other or how many there are | Everyone in the call sees everyone in the call |
| Encryption | MLS group (the same machinery as Projects) | Each post sent separately over each subscriber's own 1:1 conversation | A shared media key per call, carried in end-to-end encrypted invitations |
| Replies | In the group | Privately, into the 1:1 conversation with the channel's owner | Live |

**Why channels don't use MLS:** in an MLS group every member can see every other member. For a channel to outside contacts, that would hand each of them the list of everyone else the organization talks to. Fan-out over 1:1 conversations costs more sending, but subscribers stay invisible to each other. That's the right trade for this product.

## Group calls (built)

- **Add people to any call.** From the call window, "Add people" lists your conversations; each person you pick gets a normal call invitation ("Ada added you to a call") with the same media key. A 1:1 call becomes a group call without hanging up.
- **Rooms leave room to grow.** Every room is created with space for 8, so adding someone never needs a new room. The broker hands out extra join tokens for an existing room only to someone who already knows its unguessable name, which means someone already in the call.
- **Names stay off the server.** Participants appear on the server as random IDs. Each person's chosen name travels inside the call as end-to-end encrypted data, so the call window can show "Ada", "Ben (legal)" and "Kemi" while the server sees none of it.
- **A call ends when it empties,** not when the person who started it leaves.
- **Speaking** is shown per person from the decrypted audio, as in 1:1 calls; the server still can't tell who speaks.
- **Anyone in the call can add people:** the room's name travels inside the encrypted invitation, so the person who answered can add someone too. People who leave can be added back.

Tested live with three devices: Ada called Ben, added Kemi from the call window's Add people picker; all three windows showed "Group call · 3 people" with each person's chosen name, Kemi received both others' audio encrypted (about 775 packets each, under 1% concealed); Kemi left and Ada and Ben stayed on; Ada left and Ben's call ended by itself; every device kept a "Group call" record. Unit tests cover the same flow with three engines over a real relay.

## Groups (built)

- **One MLS group per Sotto group,** over its own relay group queue (the same machinery as Projects). The creator adds and removes people; everyone posts. Control messages (member list, title, timer) count only when they come from the creator's MLS key.
- **Joining:** the creator asks each person, inside their existing 1:1 conversation, for a key package; the reply is added to the MLS group and the Welcome goes back privately. Nobody can be added without an existing conversation with the creator.
- **Names:** a member's name in the group is the name signed into their own key package, the one they chose. Your private nickname for a contact never goes into the shared member list. On your own device, members you already talk to show under your own label, and you show as "You".
- **The conversation:** replies, reactions, edits and deletes (only by the author), disappearing messages set by the creator, pin and archive, unread counts and previews, all pushed live.
- **Group info:** members, rename, add and remove people, the disappearing timer, mute, the group's safety number, leave (or delete for everyone, for the creator).
- **Admins.** The creator can make any member an admin. Admins can rename, set the timer, pin messages, and add and remove people. Only the creator's device ever changes the MLS group, so an admin's add or removal is a request inside the group that the creator's device carries out: for an add, the admin fetches the newcomer's key package over their own conversation, the creator's device commits it, and the admin delivers the Welcome. So an admin can bring in someone the creator has never talked to, and changes happen when the creator's device is next online. Admins are known by their MLS signing key, not by name.
- **Pinned messages.** The creator or an admin pins a message for everyone; the newest pin shows in a bar under the header, and clicking it jumps there and steps back through older pins.
- **Files.** Attach or drop a file: it's encrypted on this computer, uploaded once (readable once per member), and arrives quarantined on everyone else's device until Safe review makes a clean copy. Group files also appear in Safe review's list. Up to 100 MB.
- **Safety number.** Built from every member's signing key, so it's the same on every device while membership is the same, and changes when anyone joins or leaves. Members compare it in person or on a call to make sure nobody else was slipped in.
- **Search** inside the group (the search button or Ctrl+F): matches step from newest to oldest and are highlighted in place.
- **Mute:** a muted group's unread count turns grey and stays out of the sidebar badge.
- **Call the group** from the header: everyone in it you also have a conversation with is invited to one group call.
- **Layout:** groups sit in the same list as conversations, newest first, with a Groups filter and their own rounded-square mark. Author names head each run of messages. The info panel floats over the thread when there isn't room beside it.
- Up to 100 members.

Tested: an agent test runs two devices through a real relay (the nickname stays private, names come from key packages, messages both ways with authors, only the creator removes, the removed device learns it's out). Live with three devices in the app: a group made from the New group dialog reached both others with 3 members; messages, a reply, a reaction and an edit synced; the member's device can call the creator from the header.

Tested: a second agent test runs three devices: the creator makes Ben an admin, Ben adds Kemi (whom the creator has never talked to), all three devices show the same safety number, Ben pins a message and everyone sees it while Kemi can't pin, Kemi's file reaches the creator fetched and quarantined (and listed in Safe review), Ben asks to remove Kemi and the creator's device does it, mute stays local. Live in the app: the pinned bar, admin badge and member menu, the file card and a full Safe review of a group file, search, and identical safety numbers on three devices.

Groups made before members were named from their key packages keep the names they were given, and their members can't be made admins; a new group fixes both.

## Channels (built)

- **Fan-out over 1:1 conversations.** A channel has no shared group or queue. Every post, edit and delete is sent separately over the owner's existing end-to-end encrypted conversation with each follower. The relay sees ordinary 1:1 traffic; followers never learn who else follows, or how many.
- **Following is the follower's choice.** The owner invites people from their conversations; each gets an invitation card in Sotto that explains who sees what, and chooses Follow channel or Not now. There are no join links and no directory: the owner's device ignores a Join from anyone it didn't invite.
- **Catching up.** A new follower receives the last 20 posts when they join.
- **Private replies.** A follower can reply to any post; the reply goes to the owner alone, who sees it under the post with the follower's name (their own label for that person) and a link to answer in their 1:1 conversation. Other followers never see replies.
- **The owner's view:** follower count and the list of followers and pending invitations, on the owner's device only. The API returns no audience data to a follower's device at all. Invite more people, withdraw an invitation, remove a follower, rename, change the description, close the channel.
- **The follower's view:** posts as cards with the channel's mark, the private reply panel, stop following, and a clear notice if the owner removes them or closes the channel (they keep what they have).
- **Signals only count from the right place:** posts, edits, deletes, renames and closes are accepted only from the conversation the channel came from, and posts only once you follow.
- Up to 500 followers to start, since each post is sent once per follower.

Tested: an agent test runs three devices through a real relay (invite, join with history catch-up, posts, edits and deletes reaching both followers, followers can't post, a private reply reaching only the owner, neither follower's device holding any audience data, leave, removal, close). Live in the app on three devices: a channel made from the New menu, the invitation card, following, a private reply showing on the owner's device under her label for the follower, follower counts updating live.

- **Files in posts.** The owner attaches or drops a file; it's uploaded once per 250 followers (each copy readable that many times) and each follower gets the copy meant for them. A follower who joins later gets a fresh single-read copy with the history. Files arrive quarantined and go through Safe review like any other.
- **Read receipts, only if the follower chooses.** Off by default. A follower can turn on "Read receipts" in the channel info; from then on, reading posts tells the owner (and only the owner) which posts they read. The owner sees "Seen by N" on a post, counting only followers who opted in.
- **Search** inside the channel and **mute**, as in groups.

Also tested: a third agent test covers a file post reaching a follower, receipts (the owner can't turn them on for a follower; a post read after opting in shows "Seen by 1"), a late follower getting the file through a fresh copy, and mute.

Note: the relay can count how many times a channel file's chunks are fetched, which hints at audience size. It already sees each post leave as one message per follower, so this adds little, but it's why channel files are split into copies rather than one shared upload for everyone.

## Layout

Conversations, groups and channels share one list, newest first, with Groups and Channels filters. A group has a rounded-square mark; a channel has the same square with a hand-built broadcast badge. One "new" button in the list header offers New group and New channel. A group's thread shows the sender's name on the first message of each run; a channel's thread shows posts as cards.

## Decisions for the owner

1. **Spaces (Discord-style servers with several channels inside):** add later on top of groups, or not at all? Recommended: later, once groups are in use.
2. **Group size limit:** 100 members to start (recommended), or larger?
3. **Channel size limit:** 500 followers to start (recommended). Larger channels would need batching, because every post is sent once per follower.
