//! End to end: two engines, each with its own vault, talking through a real relay.

use std::sync::Arc;
use std::time::Duration;

use sotto_client::{Engine, EngineConfig, Transport};
use sotto_core::unhex;
use sotto_vault::{Params, Vault};

async fn relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(sotto_relay::serve(listener, sotto_relay::Config::default()));
    format!("ws://{addr}")
}

async fn engine(dir: &std::path::Path, relay: &str) -> (Engine, Vault) {
    let vault = Vault::create(dir, "pass", None, Params::testing()).unwrap();
    let store = Arc::new(vault.scope(sotto_core::rand::bytes()).unwrap());
    let e = Engine::start(store, EngineConfig { default_relay: relay.to_string(), transport: Transport::Direct }).await.unwrap();
    (e, vault)
}

async fn until<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invite_chat_files_and_deletion() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (alice, _va) = engine(da.path(), &url).await;
    let (bob, _vb) = engine(db.path(), &url).await;

    // Alice invites, Bob accepts.
    let (a_cid, link) = alice.create_invite("Source K7", "Ada at Outlet", false).await.unwrap();
    assert!(alice.accept_invite(&link, "", "").await.is_err(), "own invite is refused");
    let b_cid = bob.accept_invite(&link, "Ada", "").await.unwrap();
    until("alice sees bob join", || async { alice.contact(&a_cid).await.unwrap().state == "ready" }).await;

    // Both sides compute the same safety number.
    let sa = alice.contact(&a_cid).await.unwrap().safety_number.unwrap();
    let sb = bob.contact(&b_cid).await.unwrap().safety_number.unwrap();
    assert_eq!(sa, sb);

    // Text both ways, with delivery receipts.
    let sent = bob.send_text(&b_cid, "I have the contracts.").await.unwrap();
    until("alice receives", || async { alice.messages(&a_cid, 50).unwrap().iter().any(|m| m.text.as_deref() == Some("I have the contracts.")) }).await;
    until("bob sees delivered", || async {
        bob.messages(&b_cid, 50).unwrap().iter().any(|m| m.id == sent.id && m.delivery == sotto_client::Delivery::Delivered)
    })
    .await;
    alice.send_text(&a_cid, "Thank you. Use the file button.").await.unwrap();
    until("bob receives reply", || async { bob.messages(&b_cid, 50).unwrap().iter().any(|m| !m.outgoing && m.kind == "text") }).await;
    assert_eq!(alice.contact(&a_cid).await.unwrap().unread, 1);
    alice.mark_read(&a_cid).await.unwrap();

    // A file larger than one chunk, with a partial last chunk.
    let doc: Vec<u8> = (0..(256 * 1024 * 2 + 1234)).map(|i| (i * 31 % 251) as u8).collect();
    bob.send_file(&b_cid, "../secret/contract.pdf", "application/pdf", "page 3", &mut &doc[..]).await.unwrap();
    until("alice downloads", || async {
        alice.messages(&a_cid, 50).unwrap().iter().any(|m| m.file.as_ref().map(|f| f.status == sotto_client::FileStatus::Ready).unwrap_or(false))
    })
    .await;
    let fm = alice.messages(&a_cid, 50).unwrap().into_iter().find(|m| m.kind == "file").unwrap();
    let f = fm.file.as_ref().unwrap();
    assert_eq!(f.name, "contract.pdf", "path components are stripped");
    assert_eq!(f.review.state, "quarantined", "received files start quarantined");
    let mid = unhex::<16>(&fm.id).unwrap();
    let mut back = Vec::new();
    alice.read_blob(&format!("file/{}", fm.id), &mut back).unwrap();
    assert_eq!(back, doc);

    // Timer changes reach the other side.
    alice.set_timer(&a_cid, Some(3600)).await.unwrap();
    until("bob gets timer", || async { bob.contact(&b_cid).await.unwrap().ttl_secs == Some(3600) }).await;

    // Delete for both.
    alice.delete_message(&a_cid, &mid, true).await.unwrap();
    assert!(alice.find_message(&a_cid, &mid).is_none());
    until("bob deletes too", || async { bob.find_message(&b_cid, &mid).is_none() }).await;

    // Deleting a contact removes its messages.
    alice.delete_contact(&a_cid).await.unwrap();
    assert!(alice.contacts().await.is_empty());
    assert!(alice.store().names("msg/").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_survives_restart_and_offline_messages_arrive() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let va = Vault::create(da.path(), "pass", None, Params::testing()).unwrap();
    let key: [u8; 32] = sotto_core::rand::bytes();
    let alice = Engine::start(Arc::new(va.scope(key).unwrap()), EngineConfig { default_relay: url.clone(), transport: Transport::Direct })
        .await
        .unwrap();
    let (bob, _vb) = engine(db.path(), &url).await;
    let (a_cid, link) = alice.create_invite("B", "", false).await.unwrap();
    let b_cid = bob.accept_invite(&link, "A", "Bob").await.unwrap();
    until("ready", || async { alice.contact(&a_cid).await.unwrap().state == "ready" }).await;
    assert_eq!(alice.contact(&a_cid).await.unwrap().their_name, "Bob");

    // Alice goes away; Bob writes while she is offline.
    alice.shutdown().await;
    drop(alice);
    bob.send_text(&b_cid, "while you were away").await.unwrap();

    // Alice comes back with the same vault and gets the message.
    let alice = Engine::start(Arc::new(va.scope(key).unwrap()), EngineConfig { default_relay: url.clone(), transport: Transport::Direct })
        .await
        .unwrap();
    until("offline message", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("while you were away")) }).await;
    alice.send_text(&a_cid, "back").await.unwrap();
    until("bob gets reply", || async { bob.messages(&b_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("back")) }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disappearing_messages_are_swept() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (alice, _va) = engine(da.path(), &url).await;
    let (bob, _vb) = engine(db.path(), &url).await;
    let (a_cid, link) = alice.create_invite("B", "", false).await.unwrap();
    let b_cid = bob.accept_invite(&link, "A", "").await.unwrap();
    until("ready", || async { alice.contact(&a_cid).await.unwrap().state == "ready" }).await;
    bob.set_timer(&b_cid, Some(1)).await.unwrap();
    until("timer", || async { alice.contact(&a_cid).await.unwrap().ttl_secs == Some(1) }).await;
    bob.send_text(&b_cid, "burn after reading").await.unwrap();
    until("received", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.kind == "text") }).await;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert!(alice.sweep_expired().await.unwrap() >= 1);
    assert!(!alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.kind == "text"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_time_locks_open_only_after_deadline() {
    use sotto_core::keys::{sign, signing_keypair};
    use sotto_core::wire::{Cmd, FILE_BLOB_SIZE, Resp, auth_message};
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(sotto_relay::serve(listener, sotto_relay::Config { data_dir: Some(dir.path().to_path_buf()), ..Default::default() }));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let conn = sotto_client::RelayConn::spawn(url, Transport::Direct, tx);
    let (seed, owner) = signing_keypair();
    let now = sotto_core::time::now();
    for (deadline, should_open) in [(now - 1, true), (now + 3600, false)] {
        let Resp::TIds { tl_id, read_id } = conn.request(move |_| Cmd::TNew { owner, deadline }).await.unwrap() else { panic!() };
        let blob = vec![7u8; FILE_BLOB_SIZE];
        let digest = sotto_core::kdf::sha256(&blob);
        conn.request(move |n| Cmd::TPut { tl_id, index: 0, sig: sign(&seed, &auth_message(n, "tput", &[&tl_id, &0u32.to_le_bytes(), &digest])), blob }).await.unwrap();
        let r = conn.request(move |_| Cmd::TGet { read_id, index: 0 }).await.unwrap();
        if should_open {
            assert!(matches!(r, Resp::TData { count: 1, .. }));
        } else {
            assert_eq!(r, Resp::Locked { deadline });
            // A forged check-in is refused.
            let (other, _) = signing_keypair();
            let bad = conn.request(move |n| Cmd::TCheckIn { tl_id, deadline: 0, sig: sign(&other, &auth_message(n, "tcheckin", &[&tl_id, &0u64.to_le_bytes()])) }).await;
            assert!(bad.is_err());
            // The owner can move the deadline (here: into the past, so it opens).
            let past = now - 10;
            conn.request(move |n| Cmd::TCheckIn { tl_id, deadline: past, sig: sign(&seed, &auth_message(n, "tcheckin", &[&tl_id, &past.to_le_bytes()])) }).await.unwrap();
            assert!(matches!(conn.request(move |_| Cmd::TGet { read_id, index: 0 }).await.unwrap(), Resp::TData { .. }));
        }
    }
    // Packages survive a relay restart because they are kept on disk.
    assert!(std::fs::read_dir(dir.path().join("timelocks")).unwrap().count() >= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conversations_survive_a_relay_restart() {
    let data = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = sotto_relay::Config { data_dir: Some(data.path().to_path_buf()), ..Default::default() };
    let first = tokio::spawn(sotto_relay::serve(listener, config.clone()));
    let url = format!("ws://{addr}");
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (alice, _va) = engine(da.path(), &url).await;
    let (bob, _vb) = engine(db.path(), &url).await;
    let (a_cid, link) = alice.create_invite("B", "", false).await.unwrap();
    let b_cid = bob.accept_invite(&link, "A", "").await.unwrap();
    until("ready", || async { alice.contact(&a_cid).await.unwrap().state == "ready" }).await;

    // The relay goes down and comes back on the same address with the same data directory.
    first.abort();
    let _ = first.await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tokio::spawn(sotto_relay::serve(listener, config));

    // Clients reconnect on their own and both directions still work.
    for _ in 0..100 {
        if bob.send_text(&b_cid, "still here").await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    until("alice gets it", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("still here")) }).await;
    alice.send_text(&a_cid, "me too").await.unwrap();
    until("bob gets it", || async { bob.messages(&b_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("me too")) }).await;
}
