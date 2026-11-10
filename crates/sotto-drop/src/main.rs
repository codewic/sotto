//! `sotto-drop serve --config /etc/sotto/drop.json`
//! `sotto-drop example-config` prints a starting configuration.
//!
//! Bind to localhost and publish the port as a Tor onion service only; see
//! `deploy/drop/torrc`. The drop prints one line at start-up and nothing else.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => {
            let path = match (args.get(1).map(String::as_str), args.get(2)) {
                (Some("--config"), Some(p)) => std::path::PathBuf::from(p),
                _ => usage(),
            };
            let cfg = sotto_drop::load_config(&path).unwrap_or_else(|e| fail(&e));
            let listen = cfg.listen.clone();
            let app = sotto_drop::App::new(cfg).unwrap_or_else(|e| fail(&e));
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|e| fail(&e.to_string()));
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind(&listen).await.unwrap_or_else(|e| fail(&format!("can't listen on {listen}: {e}")));
                println!("sotto-drop listening on {listen}");
                axum::serve(listener, sotto_drop::router(app)).await.unwrap_or_else(|e| fail(&e.to_string()));
            });
        }
        Some("example-config") => {
            let cfg = serde_json::json!({
                "name": "The Example Outlet",
                "instance_id": sotto_core::hex(&sotto_core::rand::bytes::<16>()),
                "newsroom_key": "paste the newsroom key from Sotto: Settings, Drops, Newsroom key",
                "journalists": [{ "name": "Reporter", "sign_pk": "paste the reporter key from Sotto: Settings, Drops" }],
                "data_dir": "/var/lib/sotto-drop",
                "listen": "127.0.0.1:7500",
                "max_upload_mb": 500,
                "argon_m_kib": 65536,
                "argon_t": 3,
                "intro": "",
                "verify_hint": "example.org/contact"
            });
            println!("{}", serde_json::to_string_pretty(&cfg).expect("json"));
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("usage: sotto-drop serve --config PATH | sotto-drop example-config");
    std::process::exit(2);
}

fn fail(msg: &str) -> ! {
    eprintln!("sotto-drop: {msg}");
    std::process::exit(1);
}
