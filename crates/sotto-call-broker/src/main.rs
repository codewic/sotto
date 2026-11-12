//! Runs the call broker. Settings come from the environment (see deploy/calls/.env.example).
//! It logs one line at start and nothing per request.

use std::net::SocketAddr;
use std::time::Duration;

use sotto_call_broker::{Config, app};

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match std::env::var(name) {
        Ok(v) => v.parse().map_err(|_| format!("{name} is not a valid number")),
        Err(_) => Ok(default),
    }
}

fn config() -> Result<(Config, SocketAddr), String> {
    let mut cfg = Config::new(env("SOTTO_CALLS_URL")?, env("LIVEKIT_API_KEY")?, env("LIVEKIT_API_SECRET")?)?;
    cfg.join_window = Duration::from_secs(env_or("BROKER_JOIN_WINDOW_SECS", 120u64)?.clamp(30, 600));
    cfg.max_huddle = env_or("BROKER_MAX_HUDDLE", 8u32)?.clamp(2, 50);
    cfg.global_per_min = env_or("BROKER_RATE_GLOBAL_PER_MIN", 120u32)?;
    cfg.ip_per_min = env_or("BROKER_RATE_IP_PER_MIN", 20u32)?;
    cfg.trust_private_proxy = matches!(std::env::var("BROKER_TRUST_PRIVATE_PROXY").as_deref(), Ok("1" | "true" | "yes"));
    let bind = env_or("BROKER_BIND", "127.0.0.1:7890".parse::<SocketAddr>().unwrap())?;
    Ok((cfg, bind))
}

#[tokio::main]
async fn main() {
    let (cfg, bind) = match config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sotto-call-broker: {e}");
            std::process::exit(2);
        }
    };
    eprintln!("sotto-call-broker: serving {} on {bind} (join window {}s, huddles up to {})", cfg.public_url, cfg.join_window.as_secs(), cfg.max_huddle);
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("sotto-call-broker: can't listen on {bind}: {e}");
            std::process::exit(1);
        }
    };
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
    };
    if let Err(e) = axum::serve(listener, app(cfg).into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown).await {
        eprintln!("sotto-call-broker: {e}");
        std::process::exit(1);
    }
}
