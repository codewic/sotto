//! Outlet tooling for `/.well-known/sotto.json`.
//!
//! sotto-directory keygen                           new outlet signing key (keep the seed offline)
//! sotto-directory sign --seed HEX --in draft.json  print the signed directory
//! sotto-directory verify --domain D --in FILE      check a published directory

use sotto_directory::Directory;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn fail(msg: &str) -> ! {
    eprintln!("sotto-directory: {msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let (seed, public) = sotto_core::keys::signing_keypair();
            println!("seed (secret, keep offline): {}", sotto_core::hex(&seed));
            println!("outlet key (public):         {}", sotto_core::hex(&public));
        }
        Some("sign") => {
            let seed_hex = arg(&args, "--seed").unwrap_or_else(|| fail("--seed is required"));
            let seed: [u8; 32] = sotto_core::unhex(&seed_hex).unwrap_or_else(|_| fail("--seed must be 64 hex characters"));
            let path = arg(&args, "--in").unwrap_or_else(|| fail("--in is required"));
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("can't read {path}: {e}")));
            let mut draft: Directory = serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("invalid draft: {e}")));
            draft.issued_at = sotto_core::time::now();
            println!("{}", serde_json::to_string_pretty(&draft.sign(&seed)).expect("json"));
        }
        Some("verify") => {
            let domain = arg(&args, "--domain").unwrap_or_else(|| fail("--domain is required"));
            let path = arg(&args, "--in").unwrap_or_else(|| fail("--in is required"));
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("can't read {path}: {e}")));
            match Directory::verify(&text, &domain) {
                Ok(d) => println!("valid directory for {} ({}), {} reporter(s), {} drop(s)", d.outlet, d.domain, d.reporters.len(), d.drops.len()),
                Err(e) => fail(&e.to_string()),
            }
        }
        _ => {
            eprintln!("usage: sotto-directory keygen | sign --seed HEX --in draft.json | verify --domain DOMAIN --in sotto.json");
            std::process::exit(2);
        }
    }
}
