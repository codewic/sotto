# Releasing Sotto

Releases are built by CI and signed by the owner. GitHub delivers them; the release key decides what the app will install.

## Where things are

- Source: `codewic/sotto` (private until the first public release).
- Releases: `codewic/sotto-releases` (public). The app reads `releases/latest/download/latest.json` from here.
- Release key: `~/.config/vorynza/sotto-release.key` on the owner's machine (owner-only permissions). Back it up offline. Its public half is pinned in `crates/sotto-core/src/release.rs` and in `apps/desktop/electron/main.cjs`. Losing it means installed copies can't be updated automatically; leaking it means someone else could. Never put it in CI.

## Cutting a release

1. Bump the version in `Cargo.toml` (`[workspace.package]` and the internal `version = "x.y.z", path = ...` dependencies) and in `apps/desktop/package.json`, and build once so `Cargo.lock` follows.
2. Write the notes in `docs/releases/vX.Y.Z.md`. They're shown in the app's update dialog and on the releases page.
3. Commit, tag `vX.Y.Z` and push the tag (over SSH; the GitHub CLI's token can't push workflow files).
4. CI (`.github/workflows/release.yml`) checks the tag matches both versions, builds the agent, sandbox renderer and `sotto-hook`, packages the AppImage and `.deb`, writes the unsigned `latest.json` and leaves a **draft** release in `codewic/sotto`.
5. On the owner's machine: `cargo run -p sotto-release -- publish vX.Y.Z`. It downloads the draft, checks every file against `latest.json`, refuses to go on if the key isn't the pinned one, signs, uploads everything with `latest.json.sig` to a draft in `codewic/sotto-releases` (retrying each file), checks GitHub has every file at its full size, and only then publishes. If a big upload fails it stays a draft; run the same command again.
6. Check: `sotto-release verify latest.json latest.json.sig` on the published files.
7. Update the Vorynza site (`vorynza-site/src/content/products.ts`, the `sotto` entry): the download links name the version (`Sotto-X.Y.Z-x86_64.AppImage`), so point them at the new release, and add the release to `src/app/changelog/page.tsx`.

## What the app does

See `crates/sotto-agent/src/updates.rs`. In short: it fetches `latest.json` and its signature (through Tor when Sotto uses Tor; only on request in Extreme mode), verifies the signature before reading anything in it, ignores versions that aren't newer, downloads the package for how it was installed (AppImage, `.deb`, Windows installer), checks its SHA-256 and size, and stages it. The desktop shell verifies the signature and fingerprint again before installing, and only when you press **Restart to update** (never during a call).
