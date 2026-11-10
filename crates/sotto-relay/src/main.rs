//! `sotto-relay [--listen 127.0.0.1:7400] [--data-dir PATH] [--max-mb 1024]`
//!
//! Bind to localhost and publish the port as a Tor onion service; see
//! `deploy/relay/torrc`. The relay prints one line at start-up and nothing else.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut listen = "127.0.0.1:7400".to_string();
    let mut config = sotto_relay::Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().expect("--listen needs an address"),
            "--data-dir" | "--timelock-dir" => config.data_dir = Some(PathBuf::from(args.next().expect("--data-dir needs a path"))),
            "--max-mb" => config.max_bytes = args.next().and_then(|v| v.parse::<usize>().ok()).expect("--max-mb needs a number") << 20,
            "--help" | "-h" => {
                println!("sotto-relay [--listen ADDR] [--data-dir PATH] [--max-mb N]");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&listen).await?;
        println!("sotto-relay listening on {}", listener.local_addr()?);
        sotto_relay::serve(listener, config).await
    })
}
