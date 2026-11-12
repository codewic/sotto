//! `sotto-release`: the release key and the signed manifest.
//!
//! ```text
//! sotto-release keygen                          # once, on the owner's machine
//! sotto-release manifest 0.2.0 dist/ > latest.json   # in CI, unsigned
//! sotto-release publish v0.2.0                  # on the owner's machine: check, sign, publish
//! sotto-release verify latest.json latest.json.sig
//! ```
//!
//! The private key never leaves the owner's machine and is never printed. CI builds the files and
//! a draft release in the private source repository; `publish` downloads that draft, checks every
//! file against the manifest, signs the manifest and publishes everything to the public releases
//! repository. The app installs nothing whose manifest doesn't verify against the pinned key.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Value, json};
use sotto_core::keys::{sign, signing_public};
use sotto_core::release::{pinned_key, signed_message, verify_manifest};
use sotto_core::{hex, kdf};

const SOURCE_REPO: &str = "codewic/sotto";
const RELEASES_REPO: &str = "codewic/sotto-releases";

fn key_path() -> PathBuf {
    if let Ok(p) = std::env::var("SOTTO_RELEASE_KEY") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/vorynza/sotto-release.key")
}

fn fail(msg: impl AsRef<str>) -> ExitCode {
    eprintln!("sotto-release: {}", msg.as_ref());
    ExitCode::FAILURE
}

fn read_seed() -> Result<[u8; 32], String> {
    let p = key_path();
    let text = std::fs::read_to_string(&p).map_err(|_| format!("no release key at {}. Run `sotto-release keygen` first.", p.display()))?;
    sotto_core::unhex::<32>(text.trim()).map_err(|_| format!("{} isn't a release key.", p.display()))
}

fn keygen() -> ExitCode {
    let p = key_path();
    if p.exists() {
        return fail(format!("{} already exists. Refusing to replace a release key: every installed copy trusts it.", p.display()));
    }
    if let Some(dir) = p.parent()
        && std::fs::create_dir_all(dir).is_err() {
            return fail(format!("can't create {}", dir.display()));
        }
    let seed: [u8; 32] = sotto_core::rand::bytes();
    // Write with owner-only permissions from the start.
    let written = {
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        o.mode(0o600);
        o.open(&p).and_then(|mut f| f.write_all(hex(&seed).as_bytes()))
    };
    if written.is_err() {
        return fail(format!("couldn't write {}", p.display()));
    }
    println!("Release key written to {} (only you can read it). Back it up somewhere offline.", p.display());
    println!("Public key, to pin in sotto-core/src/release.rs:");
    println!("{}", hex(&signing_public(&seed)));
    ExitCode::SUCCESS
}

fn sha256_file(p: &Path) -> Result<(String, u64), String> {
    let bytes = std::fs::read(p).map_err(|e| format!("can't read {}: {e}", p.display()))?;
    Ok((hex(&kdf::sha256(&bytes)), bytes.len() as u64))
}

/// What a file is, from its name.
fn kind_of(name: &str) -> Option<(&'static str, &'static str)> {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".appimage") {
        Some(("appimage", "linux"))
    } else if n.ends_with(".deb") {
        Some(("deb", "linux"))
    } else if n.ends_with(".exe") {
        Some(("exe", "windows"))
    } else if n.ends_with(".tar.gz") {
        Some(("tools", "linux"))
    } else {
        None
    }
}

fn manifest(version: &str, dir: &str, notes: Option<&str>) -> ExitCode {
    let mut files = Vec::new();
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(r) => r.flatten().collect(),
        Err(e) => return fail(format!("can't read {dir}: {e}")),
    };
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let Some((kind, os)) = kind_of(&name) else { continue };
        let (sha256, size) = match sha256_file(&e.path()) {
            Ok(x) => x,
            Err(m) => return fail(m),
        };
        files.push(json!({ "name": name, "kind": kind, "os": os, "arch": "x86_64", "size": size, "sha256": sha256 }));
    }
    if files.is_empty() {
        return fail(format!("nothing to release in {dir}"));
    }
    let notes = match notes {
        Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
        None => String::new(),
    };
    let m = json!({
        "app": "sotto",
        "version": version.trim_start_matches('v'),
        "published_at": sotto_core::time::now(),
        "notes": notes.trim(),
        "files": files,
    });
    println!("{}", serde_json::to_string_pretty(&m).expect("json"));
    ExitCode::SUCCESS
}

fn gh(args: &[&str]) -> Result<(), String> {
    let ok = Command::new("gh").args(args).status().map_err(|e| format!("couldn't run gh: {e}"))?.success();
    if ok { Ok(()) } else { Err(format!("gh {} failed", args.join(" "))) }
}

fn publish(tag: &str) -> ExitCode {
    let seed = match read_seed() {
        Ok(s) => s,
        Err(m) => return fail(m),
    };
    let public = signing_public(&seed);
    match pinned_key() {
        Some(k) if k == public => {}
        _ => return fail("this release key isn't the one pinned in sotto-core/src/release.rs; the app would refuse the release."),
    }
    let work = std::env::temp_dir().join(format!("sotto-release-{}", tag.trim_start_matches('v')));
    let _ = std::fs::remove_dir_all(&work);
    if std::fs::create_dir_all(&work).is_err() {
        return fail("can't make a work folder");
    }
    let w = work.to_string_lossy().to_string();
    println!("Downloading the draft {tag} from {SOURCE_REPO}…");
    if let Err(m) = gh(&["release", "download", tag, "--repo", SOURCE_REPO, "--dir", &w]) {
        return fail(m);
    }
    let manifest_path = work.join("latest.json");
    let Ok(manifest_bytes) = std::fs::read(&manifest_path) else { return fail("the draft has no latest.json") };
    let Ok(m) = serde_json::from_slice::<Value>(&manifest_bytes) else { return fail("latest.json isn't valid JSON") };
    if m["version"].as_str() != Some(tag.trim_start_matches('v')) {
        return fail(format!("latest.json says version {}, not {tag}", m["version"]));
    }
    // Every file must match what CI recorded, byte for byte.
    let mut assets = vec![manifest_path.to_string_lossy().to_string()];
    for f in m["files"].as_array().cloned().unwrap_or_default() {
        let name = f["name"].as_str().unwrap_or_default();
        let p = work.join(name);
        match sha256_file(&p) {
            Ok((sha, size)) if Some(sha.as_str()) == f["sha256"].as_str() && Some(size) == f["size"].as_u64() => {
                println!("  ok  {name} ({size} bytes)");
                assets.push(p.to_string_lossy().to_string());
            }
            Ok(_) => return fail(format!("{name} doesn't match latest.json. Not publishing.")),
            Err(e) => return fail(e),
        }
    }
    let sig = hex(&sign(&seed, &signed_message(&manifest_bytes)).0);
    if !verify_manifest(&manifest_bytes, &sig, &public) {
        return fail("the signature didn't verify; not publishing");
    }
    let sig_path = work.join("latest.json.sig");
    if std::fs::write(&sig_path, &sig).is_err() {
        return fail("can't write latest.json.sig");
    }
    assets.push(sig_path.to_string_lossy().to_string());
    let notes = work.join("notes.md");
    let _ = std::fs::write(&notes, m["notes"].as_str().unwrap_or("").to_string() + "\n\nEvery file is listed with its SHA-256 in `latest.json`, which is signed with the Sotto release key (`latest.json.sig`). The app checks both before installing.\n");
    let notes_s = notes.to_string_lossy().to_string();
    let mut args = vec!["release", "create", tag, "--repo", RELEASES_REPO, "--title", tag, "--notes-file", notes_s.as_str()];
    for a in &assets {
        args.push(a);
    }
    println!("Signed. Publishing {tag} to {RELEASES_REPO}…");
    if let Err(e) = gh(&args) {
        return fail(e);
    }
    let _ = std::fs::remove_dir_all(&work);
    println!("Published: https://github.com/{RELEASES_REPO}/releases/tag/{tag}");
    ExitCode::SUCCESS
}

fn verify(manifest: &str, sig: &str) -> ExitCode {
    let (Ok(m), Ok(s)) = (std::fs::read(manifest), std::fs::read_to_string(sig)) else { return fail("can't read the files") };
    let Some(key) = pinned_key() else { return fail("no release key is pinned in this build") };
    if verify_manifest(&m, &s, &key) {
        println!("Good signature from the pinned Sotto release key.");
        ExitCode::SUCCESS
    } else {
        fail("BAD signature: don't install anything from this release.")
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["keygen"] => keygen(),
        ["manifest", version, dir] => manifest(version, dir, None),
        ["manifest", version, dir, "--notes", notes] => manifest(version, dir, Some(notes)),
        ["publish", tag] => publish(tag),
        ["verify", m, s] => verify(m, s),
        _ => {
            eprintln!("usage: sotto-release keygen | manifest <version> <dir> [--notes FILE] | publish <tag> | verify <latest.json> <latest.json.sig>");
            ExitCode::from(2)
        }
    }
}
