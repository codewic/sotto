# Plan: signed updates, a richer composer, media, and Files

Status (12 November 2026): section 1 (signed updates) is built, see `docs/releasing.md`. Section 2 (composer) is built except voice notes and "send as photo" (both come with media), per-message disappearing time, and silent send (which waits for desktop notifications); see `task.md`. Sections 3 and 4 are next. The owner chose the recommended answer to every decision below.

The ask was auto-update with a GitHub releases repository, a feature-rich composer, media sharing, and a file manager with a viewer. This plan keeps all four and adds the parts a security product needs so that none of them becomes the weak point: updates that GitHub can't tamper with, update checks that don't reveal who uses Sotto, media that can't attack the device that opens it, and files that stay encrypted and quarantined however they're browsed.

## 1. Signed automatic updates

**Repositories.** Same pattern as Ona: source in a private `codewic/sotto`, releases in a public `codewic/sotto-releases` (the app downloads from `releases/latest/download/`, and the website can link the same files). The README and the About screen say Sotto is open source (AGPL), so the source should go public no later than the first public release; until then it stays private.

**GitHub delivers, your key decides.** For a source-protection app, a malicious update is the most dangerous attack there is: it reaches every user at once. So GitHub (and CI) are only the delivery channel:

- Every release has a manifest (`latest.json`: version, notes, minimum version, and the SHA-256 and size of every file) signed with a **release key** (Ed25519). The public key is built into the app. The app refuses anything whose manifest doesn't verify or whose files don't match the hashes, and never installs an older version than it runs (no downgrade attacks).
- The signing key is **not** in CI. CI builds and uploads a draft release; you sign the manifest on your own machine with a small `sotto-release sign` command and publish it. A compromised GitHub account or CI runner can then delay updates but can't push one. (Later: two keys, two signers.)
- Builds are made reproducible where the toolchain allows, and each release's hashes are listed in `docs/transparency`, so anyone can check that what's published came from the tagged source.

**Checking without telling anyone who you are.** An update check is a request that says "someone here runs Sotto".

- No identifiers, no telemetry, no version in the request: the app fetches the same public manifest everyone fetches.
- Checks happen at launch and about every 6 hours, at random offsets, so they don't line up with use.
- In High risk mode checks go through Tor. In Extreme mode the app checks only when you press **Check for updates**.
- Optional later: a mirror of the releases on your own server as an onion service, so Tor users never touch GitHub.

**In the app.**

- A quiet "Update ready" in the sidebar, with the release notes and **Restart to update**. Never restarts by itself, never during a call, and keeps your place.
- Settings, About gets the version, channel (Stable or Early) and **Check for updates**.
- Install follows the platform: AppImage replaces itself, the Windows installer runs silently, `.deb` asks for permission through the system. macOS needs an Apple developer account, so it comes later.

**Release flow.** Bump the version, push tag `v0.x.y`. The Actions workflow builds the agent, relay, review sandbox and `sotto-hook` for each platform, packages the app (AppImage, deb, Windows installer), writes the unsigned manifest and uploads a draft to `sotto-releases`. You run `sotto-release sign v0.x.y`, which checks the draft's files against what CI reported, signs and publishes.

## 2. A composer worth typing in

It already has reply, edit (and Up to edit last), one attachment, one-time secrets, drafts in 1:1 chats, and code blocks if you type the backticks. Add:

- **Formatting without remembering syntax.** A small toolbar on selection (bold, italic, code, code block, quote) and shortcuts (Ctrl+B, Ctrl+I, Ctrl+E for code, Ctrl+Shift+C for a code block). It still sends plain text with light markup, so nothing hidden travels.
- **Several attachments at once.** Paste images from the clipboard, drop several files, see them in a tray above the box with a thumbnail, size and remove button for each, and one caption for the set. Choose **Send as photo** (cleaned and compressed, see section 3) or **Send as file** (exact bytes).
- **Mentions in groups.** Type `@` for a member list; a mention highlights the message and adds an "@" marker to the group's unread badge for that person, even when the group is muted.
- **Emoji and shortcodes.** An emoji picker and `:shortcode:` completion, with the same recents as reactions.
- **Voice notes.** Hold or click to record, see a live waveform, listen before sending, send. Voice identifies a person, so High risk mode shows a one-time note about that, and Extreme mode offers it only after you confirm.
- **Slash commands.** `/secret`, `/code`, `/timer 1h`, `/call`, `/silent` from the keyboard, listed as you type `/`.
- **Per-message choices.** Send silently (no notification on their side), disappear after a set time for this message only, or schedule for later (held on this device and sent when it's due, so it only goes if Sotto is open).
- **Drafts everywhere.** Groups and channels keep unsent text too, and the list shows "Draft: ..." in the preview.
- **Kept out on purpose.** No link previews (fetching a link tells a server you saw it) and no typing indicators by default (they reveal when you're active). Typing indicators could be an opt-in per conversation, off in High risk.

## 3. Sharing media safely

Photos, videos, audio and voice notes with previews and a proper viewer, without giving an attacker a way in.

**Sending.** Photos are cleaned by default: the agent re-encodes them, which removes location, camera serial, timestamps and other metadata, and resizes very large ones. The composer says "Location and camera details removed". **Send as file** keeps the original byte for byte when you need it (for example, evidence). Videos and audio keep their bytes but their metadata containers are stripped where the format allows.

**Receiving.** Image and video decoders are among the most attacked code on any device. So a received photo is never decoded by the app directly: it's decoded and re-encoded inside the same sealed sandbox Safe review uses (no network, no access to your files), and the app shows only that clean copy. This happens automatically for photos from people you've accepted, in all modes except Extreme, where media waits for **Show** like any file. Videos get a sandbox-made preview frame; playing one transcodes it in the sandbox first. Voice notes recorded in Sotto play directly (the app made them, in one known format); other audio goes through the sandbox.

**Showing.** Photos appear inline with the right shape while loading (a blurred placeholder made from the sender's own small preview), albums appear as a grid, and a full-screen viewer offers zoom, pan, next and previous, keyboard control, save a clean copy, and forward. Each conversation's info panel gets Media, Files, Voice and Links tabs.

## 4. Files: one place for everything you've received or sent

A **Files** screen that grows out of today's Safe review list.

- **Everything, from everywhere.** Files from 1:1 chats, groups, channels, the anonymous inbox and projects, in a list or a grid.
- **Find things fast.** Filter by type (photos, videos, audio, documents, code, archives), by where it came from, by date, by review state (quarantined, clean, can't open) and by size, and search by name.
- **A viewer for each kind.**
  - Clean photos and video.
  - PDFs and office documents as the clean copy from Safe review, page by page.
  - Text and code shown as text, with highlighting.
  - CSV as a table.
  - Archives as a list of what's inside, read in the sandbox and never unpacked onto the disk.
- **Actions.**
  - Open the clean copy.
  - Review safely.
  - Save a copy, with a warning that it leaves Sotto's encrypted storage. Clean copies are saved by default.
  - Forward to a conversation, group or channel.
  - Star.
  - Delete from this device.
- **Storage.** Space used per conversation and in total, and **Free up space**: remove files older than a date or larger than a size, keeping starred ones.
- Everything stays encrypted at rest; nothing is written unencrypted to disk except a copy you choose to save.

## Order of work

1. **Signed updates and the release pipeline** first, because every later improvement then reaches people safely. This needs the two repositories and a release key.
2. **Composer**: formatting, multi-attach, mentions, emoji, drafts everywhere, slash commands, then voice notes and per-message options.
3. **Media**: photo cleaning on send, sandboxed decoding on receive, inline photos, albums, the viewer; then video and audio.
4. **Files**: the screen, filters and search, the viewers, actions, storage clean-up.

Steps 2 to 4 can run in parallel with step 1 once the release pipeline exists.

## Decisions for the owner

1. **Where the code lives.** `codewic/sotto` (private source) and `codewic/sotto-releases` (public releases), like Ona (recommended). Or the `vorynza` organization named in `Cargo.toml`. Also: when to make the source public. Recommended: at the first public release, since the app says it's open source.
2. **Who holds the release key.** You, on your own machine, signing each release locally (recommended). Or a key in GitHub Actions secrets, which is easier but means a CI compromise can ship an update.
3. **First platforms.** Linux AppImage and `.deb` plus a Windows installer (recommended). macOS needs an Apple developer account.
4. **Received photos.** Cleaned in the sandbox and shown automatically for accepted contacts (recommended), or always click to show.
5. **Voice notes in High risk and Extreme.** Allowed with a note in High risk and behind a confirm in Extreme (recommended), or off in Extreme.
