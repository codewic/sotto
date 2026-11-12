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
    let sent = bob.send_text(&b_cid, "I have the contracts.", None).await.unwrap();
    until("alice receives", || async { alice.messages(&a_cid, 50).unwrap().iter().any(|m| m.text.as_deref() == Some("I have the contracts.")) }).await;
    until("bob sees delivered", || async {
        bob.messages(&b_cid, 50).unwrap().iter().any(|m| m.id == sent.id && m.delivery == sotto_client::Delivery::Delivered)
    })
    .await;
    alice.send_text(&a_cid, "Thank you. Use the file button.", None).await.unwrap();
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
    bob.send_text(&b_cid, "while you were away", None).await.unwrap();

    // Alice comes back with the same vault and gets the message.
    let alice = Engine::start(Arc::new(va.scope(key).unwrap()), EngineConfig { default_relay: url.clone(), transport: Transport::Direct })
        .await
        .unwrap();
    until("offline message", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("while you were away")) }).await;
    alice.send_text(&a_cid, "back", None).await.unwrap();
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
    bob.send_text(&b_cid, "burn after reading", None).await.unwrap();
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
        if bob.send_text(&b_cid, "still here", None).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    until("alice gets it", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("still here")) }).await;
    alice.send_text(&a_cid, "me too", None).await.unwrap();
    until("bob gets it", || async { bob.messages(&b_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("me too")) }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replies_reactions_edits_and_read_receipts() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (alice, _va) = engine(da.path(), &url).await;
    let (bob, _vb) = engine(db.path(), &url).await;
    let (a_cid, link) = alice.create_invite("B", "", false).await.unwrap();
    let b_cid = bob.accept_invite(&link, "A", "").await.unwrap();
    until("ready", || async { alice.contact(&a_cid).await.unwrap().state == "ready" }).await;

    let first = bob.send_text(&b_cid, "the meeting is tuesday", None).await.unwrap();
    until("arrives", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.id == first.id) }).await;
    assert_eq!(alice.contact(&a_cid).await.unwrap().preview, "the meeting is tuesday");

    // Reply and react.
    let fid = unhex::<16>(&first.id).unwrap();
    alice.send_text(&a_cid, "which room?", Some(fid)).await.unwrap();
    alice.react(&a_cid, &fid, "✅", true).await.unwrap();
    until("reply and reaction arrive", || async {
        let ms = bob.messages(&b_cid, 10).unwrap();
        ms.iter().any(|m| m.reply_to.as_deref() == Some(first.id.as_str())) && ms.iter().any(|m| m.id == first.id && m.reactions.iter().any(|r| r.emoji == "✅" && !r.mine))
    })
    .await;

    // Edit: only the author can, and the other side sees it.
    assert!(alice.edit(&a_cid, &fid, "not mine").await.is_err());
    bob.edit(&b_cid, &fid, "the meeting is wednesday").await.unwrap();
    until("edit arrives", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.text.as_deref() == Some("the meeting is wednesday") && m.edited) }).await;

    // Read receipts are off by default, and flow once turned on.
    alice.mark_read(&a_cid).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!bob.messages(&b_cid, 10).unwrap().iter().any(|m| m.id == first.id && m.read));
    let second = bob.send_text(&b_cid, "room 4", None).await.unwrap();
    until("second", || async { alice.messages(&a_cid, 10).unwrap().iter().any(|m| m.id == second.id) }).await;
    alice.set_read_receipts(&a_cid, true).await.unwrap();
    alice.mark_read(&a_cid).await.unwrap();
    until("read receipt", || async { bob.messages(&b_cid, 10).unwrap().iter().any(|m| m.id == second.id && m.read) }).await;
}

fn grant(tag: &str) -> sotto_client::RoomGrant {
    sotto_client::RoomGrant { url: "wss://rtc.example.test".into(), room: format!("room-{tag}"), my_token: format!("caller-{tag}"), their_token: format!("callee-{tag}") }
}

fn texts(e: &Engine, cid: &[u8; 16]) -> Vec<String> {
    e.messages(cid, 50).unwrap().into_iter().filter(|m| m.kind == "system").filter_map(|m| m.text).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calls_ring_answer_end_and_leave_records() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (alice, _va) = engine(da.path(), &url).await;
    let (bob, _vb) = engine(db.path(), &url).await;
    let (a_cid, link) = alice.create_invite("B", "", false).await.unwrap();
    let b_cid = bob.accept_invite(&link, "A", "").await.unwrap();
    until("ready", || async { alice.contact(&a_cid).await.unwrap().state == "ready" }).await;
    let mut bob_events = bob.events();

    // Alice calls: she can join straight away and waits in the room; Bob's side rings.
    let call = alice.start_call(&a_cid, grant("1")).await.unwrap();
    assert_eq!(call.phase, sotto_client::calls::Phase::Calling);
    let cid = unhex::<16>(&call.id).unwrap();
    let mine = alice.join_info(&cid).await.unwrap();
    assert_eq!((mine.token.as_str(), mine.outgoing), ("caller-1", true));
    assert!(alice.start_call(&a_cid, grant("x")).await.is_err(), "one call at a time");
    until("bob rings", || async { bob.calls().await.iter().any(|c| c.id == call.id && c.phase == sotto_client::calls::Phase::Ringing) }).await;
    assert!(bob.join_info(&cid).await.is_err(), "no join details before answering");

    // Events carry no secrets.
    let mut seen = String::new();
    while let Ok(e) = bob_events.try_recv() {
        seen.push_str(&serde_json::to_string(&e).unwrap());
    }
    assert!(seen.contains("ringing") && !seen.contains("callee-1") && !seen.contains("caller-1"), "{seen}");

    // Bob answers and gets the other token and the same media key.
    bob.answer_call(&cid).await.unwrap();
    let theirs = bob.join_info(&cid).await.unwrap();
    assert_eq!((theirs.token.as_str(), theirs.outgoing, theirs.url.as_str()), ("callee-1", false, "wss://rtc.example.test"));
    assert_eq!(theirs.key, mine.key);
    until("alice sees the answer", || async { alice.calls().await.iter().any(|c| c.phase == sotto_client::calls::Phase::Active) }).await;

    // Bob hangs up: both sides end and keep a record.
    bob.end_call(&cid).await.unwrap();
    until("alice ends too", || async { alice.calls().await.is_empty() }).await;
    assert!(alice.join_info(&cid).await.is_err(), "keys are gone after the call");
    until("records", || async { texts(&alice, &a_cid).iter().any(|t| t.starts_with("Outgoing call, ")) && texts(&bob, &b_cid).iter().any(|t| t.starts_with("Incoming call, ")) }).await;

    // An unanswered call rings out: "No answer" for the caller, an unread "Missed call" for the callee.
    alice.set_ring_time(Duration::from_millis(1500)).await;
    let unread_before = bob.contact(&b_cid).await.unwrap().unread;
    let second = alice.start_call(&a_cid, grant("2")).await.unwrap();
    until("bob rings again", || async { bob.calls().await.iter().any(|c| c.id == second.id) }).await;
    until("rings out on both sides", || async { alice.calls().await.is_empty() && bob.calls().await.is_empty() }).await;
    until("no answer and missed", || async { texts(&alice, &a_cid).contains(&"No answer".to_string()) && texts(&bob, &b_cid).contains(&"Missed call".to_string()) }).await;
    assert_eq!(bob.contact(&b_cid).await.unwrap().unread, unread_before + 1);

    // An explicit decline tells the caller; a cancelled call is a missed call for the callee.
    alice.set_ring_time(Duration::from_secs(30)).await;
    let third = alice.start_call(&a_cid, grant("3")).await.unwrap();
    let tid = unhex::<16>(&third.id).unwrap();
    until("rings", || async { bob.calls().await.iter().any(|c| c.id == third.id) }).await;
    bob.decline_call(&tid, true).await.unwrap();
    until("alice hears they couldn't talk", || async { texts(&alice, &a_cid).contains(&"They couldn't talk".to_string()) }).await;
    assert!(texts(&bob, &b_cid).contains(&"Declined call".to_string()));

    let fourth = alice.start_call(&a_cid, grant("4")).await.unwrap();
    until("rings", || async { bob.calls().await.iter().any(|c| c.id == fourth.id) }).await;
    alice.end_call(&unhex::<16>(&fourth.id).unwrap()).await.unwrap();
    until("bob stops ringing", || async { bob.calls().await.is_empty() }).await;
    assert!(texts(&alice, &a_cid).contains(&"Call cancelled".to_string()));
    assert_eq!(texts(&bob, &b_cid).iter().filter(|t| *t == "Missed call").count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn people_can_be_added_to_a_call() {
    use sotto_client::calls::Phase;
    let url = relay().await;
    let (da, db, dc) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (alice, _va) = engine(da.path(), &url).await;
    let (bob, _vb) = engine(db.path(), &url).await;
    let (cara, _vc) = engine(dc.path(), &url).await;
    let (a_b, link) = alice.create_invite("Bob", "", false).await.unwrap();
    let b_a = bob.accept_invite(&link, "Alice", "").await.unwrap();
    let (a_c, link) = alice.create_invite("Cara", "", false).await.unwrap();
    let c_a = cara.accept_invite(&link, "Alice", "").await.unwrap();
    until("ready", || async { alice.contact(&a_b).await.unwrap().state == "ready" && alice.contact(&a_c).await.unwrap().state == "ready" }).await;

    // Alice calls Bob; Bob answers.
    let call = alice.start_call(&a_b, grant("g")).await.unwrap();
    let id = unhex::<16>(&call.id).unwrap();
    assert_eq!(alice.call_room(&id).await.unwrap(), ("wss://rtc.example.test".to_string(), "room-g".to_string()));
    until("bob rings", || async { !bob.calls().await.is_empty() }).await;
    bob.answer_call(&id).await.unwrap();
    until("active", || async { alice.calls().await.iter().any(|c| c.phase == Phase::Active) }).await;

    // Alice adds Cara with a token for the same room: Cara rings with a group invitation and the same key.
    let v = alice.invite_to_call(&id, &a_c, "third-token".into()).await.unwrap();
    assert!(v.group && v.invited.len() == 1);
    assert!(alice.invite_to_call(&id, &a_c, "again".into()).await.is_err(), "no double invitations");
    assert!(alice.invite_to_call(&id, &a_b, "again".into()).await.is_err(), "Bob is already in");
    until("cara rings", || async { cara.calls().await.iter().any(|c| c.id == call.id && c.phase == Phase::Ringing && c.group) }).await;
    assert_eq!(bob.call_room(&id).await.unwrap().1, "room-g", "anyone in the call can add people");
    cara.answer_call(&id).await.unwrap();
    let j = cara.join_info(&id).await.unwrap();
    assert_eq!((j.token.as_str(), j.group), ("third-token", true));
    assert_eq!(j.key, alice.join_info(&id).await.unwrap().key);

    // Bob's call window sees two others and marks the call as a group call.
    bob.mark_group_call(&id).await.unwrap();

    // Cara leaves: Alice and Bob stay on.
    cara.end_call(&id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alice.calls().await.iter().any(|c| c.phase == Phase::Active), "one person leaving doesn't end a group call");
    until("cara can be added again", || async { alice.calls().await.iter().any(|c| c.invited.is_empty()) }).await;

    // Alice leaves: Bob stays on too (his window ends it when the room empties).
    alice.end_call(&id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(bob.calls().await.iter().any(|c| c.phase == Phase::Active));
    bob.end_call(&id).await.unwrap();

    until("group call records", || async {
        texts(&alice, &a_b).iter().any(|t| t.starts_with("Group call, ")) && texts(&bob, &b_a).iter().any(|t| t.starts_with("Group call, ")) && texts(&cara, &c_a).iter().any(|t| t.starts_with("Group call, "))
    })
    .await;
}
