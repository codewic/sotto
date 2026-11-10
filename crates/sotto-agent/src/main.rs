//! `sotto-agent --data DIR (--socket PATH | --listen 127.0.0.1:PORT) [--relay URL] [--tor 127.0.0.1:9050 | --direct] [--token T]`
//!
//! The desktop app starts this process and talks to it over a Unix socket.
//! `--listen` exists for development in a browser; it only binds to loopback.
//! On start-up the agent prints one line: `SOTTO_AGENT_READY <token>`.

use std::path::PathBuf;

use sotto_client::Transport;

fn fail(msg: &str) -> ! {
    eprintln!("sotto-agent: {msg}");
    std::process::exit(2);
}

fn main() {
    let mut data: Option<PathBuf> = None;
    let mut socket: Option<PathBuf> = None;
    let mut listen: Option<String> = None;
    let mut relay = "ws://127.0.0.1:7400".to_string();
    let mut transport = Transport::Tor { socks: "127.0.0.1:9050".into() };
    let mut token: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data = args.next().map(PathBuf::from),
            "--socket" => socket = args.next().map(PathBuf::from),
            "--listen" => listen = args.next(),
            "--relay" => relay = args.next().unwrap_or_else(|| fail("--relay needs a URL")),
            "--tor" => transport = Transport::Tor { socks: args.next().unwrap_or_else(|| fail("--tor needs host:port")) },
            "--direct" => transport = Transport::Direct,
            "--token" => token = args.next(),
            other => fail(&format!("unknown argument {other}")),
        }
    }
    let data = data.unwrap_or_else(|| fail("--data is required"));
    if let Some(l) = &listen {
        if !(l.starts_with("127.0.0.1:") || l.starts_with("[::1]:")) {
            fail("--listen only accepts loopback addresses");
        }
    }
    std::fs::create_dir_all(&data).unwrap_or_else(|e| fail(&format!("can't create {}: {e}", data.display())));
    let token = token.unwrap_or_else(|| sotto_core::hex(&sotto_core::rand::bytes::<32>()));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|e| fail(&e.to_string()));
    rt.block_on(async move {
        let agent = sotto_agent::Agent::new(data.join("vault"), sotto_vault::Params::default(), relay, transport, token.clone());
        sotto_agent::spawn_background(agent.clone());
        let app = sotto_agent::router(agent.clone());
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        if let Some(path) = socket {
            let _ = std::fs::remove_file(&path);
            let listener = tokio::net::UnixListener::bind(&path).unwrap_or_else(|e| fail(&format!("can't bind socket: {e}")));
            restrict(&path);
            println!("SOTTO_AGENT_READY {token}");
            axum::serve(listener, app).with_graceful_shutdown(shutdown).await.unwrap_or_else(|e| fail(&e.to_string()));
        } else {
            let addr = listen.unwrap_or_else(|| fail("--socket or --listen is required"));
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| fail(&format!("can't listen on {addr}: {e}")));
            println!("SOTTO_AGENT_READY {token}");
            axum::serve(listener, app).with_graceful_shutdown(shutdown).await.unwrap_or_else(|e| fail(&e.to_string()));
        }
        agent.lock().await;
    });
}

/// Only the current user may connect to the socket.
fn restrict(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}
