//! A relay behind TLS, as the hosted one runs: the client connects over `wss://`, checks the
//! certificate, and comes online.

use std::sync::Arc;
use std::time::Duration;

use sotto_client::conn::{RelayConn, Status, Transport};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connects_to_a_relay_over_wss() {
    let dir = tempfile::tempdir().unwrap();
    let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    let made = std::process::Command::new("openssl")
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost", "-addext", "basicConstraints=critical,CA:FALSE", "-addext", "extendedKeyUsage=serverAuth"])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !made {
        eprintln!("skipped: needs openssl to make a test certificate");
        return;
    }

    // The relay, plain, on loopback.
    let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = relay.local_addr().unwrap();
    tokio::spawn(sotto_relay::serve(relay, sotto_relay::Config::default()));

    // A TLS proxy in front of it, like Coolify's.
    use rustls::pki_types::pem::PemObject;
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(&cert).unwrap().map(Result::unwrap).collect();
    let key = rustls::pki_types::PrivateKeyDer::from_pem_file(&key).unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let front = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = front.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = front.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else { return };
                let mut back = tokio::net::TcpStream::connect(relay_addr).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut back).await;
            });
        }
    });

    // SAFETY: only this test reads it.
    unsafe { std::env::set_var("SOTTO_TEST_RELAY_CA", &cert) };
    let (pushes, _rx) = tokio::sync::mpsc::unbounded_channel();
    let conn = RelayConn::spawn(format!("wss://localhost:{port}"), Transport::Direct, pushes);
    let mut status = conn.status_watch();
    let online = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if *status.borrow() == Status::Online {
                return;
            }
            status.changed().await.unwrap();
        }
    })
    .await;
    assert!(online.is_ok(), "never came online over wss://");
}
