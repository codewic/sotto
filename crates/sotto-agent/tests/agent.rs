//! The agent API end to end: two desks talking through a relay, lifecycle, duress, and a drop.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sotto_agent::Agent;
use sotto_client::Transport;
use tower::ServiceExt;

const TOKEN: &str = "test-token";

async fn relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(sotto_relay::serve(listener, sotto_relay::Config::default()));
    format!("ws://{addr}")
}

fn desk(dir: &std::path::Path, relay: &str) -> (Arc<Agent>, Router) {
    let agent = Agent::new(dir.join("vault"), sotto_vault::Params::testing(), relay.to_string(), Transport::Direct, TOKEN.into());
    let r = sotto_agent::router(agent.clone());
    (agent, r)
}

async fn call(r: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).header(header::HOST, "127.0.0.1");
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = r.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn until<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    // Generous: CI machines run every test at once and can be slow.
    for _ in 0..800 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_desks_lifecycle_and_duress() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_a, ra) = desk(da.path(), &url);
    let (_b, rb) = desk(db.path(), &url);

    // The token is required, and the Host must be local.
    let res = ra.clone().oneshot(Request::get("/api/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = ra
        .clone()
        .oneshot(Request::get("/api/status").header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).header(header::HOST, "evil.example").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    assert_eq!(call(&ra, "GET", "/api/status", None).await.1["state"], "new");
    let (s, v) = call(&ra, "POST", "/api/setup", Some(json!({ "passcode": "short" }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(v["error"].as_str().unwrap().contains("8 characters"));
    assert_eq!(call(&ra, "POST", "/api/setup", Some(json!({ "passcode": "alice-passcode", "duress": "alice-decoy-1" }))).await.0, StatusCode::OK);
    assert_eq!(call(&rb, "POST", "/api/setup", Some(json!({ "passcode": "bob-passcode" }))).await.0, StatusCode::OK);
    assert_eq!(call(&ra, "GET", "/api/status", None).await.1["state"], "unlocked");

    // Invite and accept over the API.
    let (_, inv) = call(&ra, "POST", "/api/invites", Some(json!({ "nickname": "Source K7", "label": "Ada" }))).await;
    assert!(inv["qr"].as_str().unwrap().contains("<svg"));
    let a_cid = inv["contact_id"].as_str().unwrap().to_string();
    let (s, v) = call(&rb, "POST", "/api/invites/accept", Some(json!({ "link": "sotto://invite/garbage" }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(v["error"].as_str().unwrap().contains("valid Sotto invite"), "{v}");
    let (_, acc) = call(&rb, "POST", "/api/invites/accept", Some(json!({ "link": inv["link"], "nickname": "Ada" }))).await;
    let b_cid = acc["contact_id"].as_str().unwrap().to_string();
    until("ready", || async { call(&ra, "GET", &format!("/api/contacts/{a_cid}"), None).await.1["state"] == "ready" }).await;

    call(&rb, "POST", &format!("/api/contacts/{b_cid}/messages"), Some(json!({ "text": "hello from the source" }))).await;
    until("delivered", || async {
        call(&ra, "GET", &format!("/api/contacts/{a_cid}/messages"), None).await.1.as_array().unwrap().iter().any(|m| m["text"] == "hello from the source")
    })
    .await;
    let (_, c) = call(&ra, "PATCH", &format!("/api/contacts/{a_cid}"), Some(json!({ "verified": true, "timer": 86400 }))).await;
    assert_eq!(c["verified"], true);
    assert_eq!(c["ttl_secs"], 86400);

    // Lock forgets keys; a wrong passcode is refused; the right one restores everything.
    call(&ra, "POST", "/api/lock", None).await;
    assert_eq!(call(&ra, "GET", "/api/contacts", None).await.0, StatusCode::LOCKED);
    assert_eq!(call(&ra, "POST", "/api/unlock", Some(json!({ "passcode": "wrong-passcode" }))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&ra, "POST", "/api/unlock", Some(json!({ "passcode": "alice-passcode" }))).await.0, StatusCode::OK);
    assert_eq!(call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().len(), 1);

    // The duress passcode opens an empty Sotto and destroys the real one.
    call(&ra, "POST", "/api/lock", None).await;
    assert_eq!(call(&ra, "POST", "/api/unlock", Some(json!({ "passcode": "alice-decoy-1" }))).await.0, StatusCode::OK);
    assert!(call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().is_empty());
    call(&ra, "POST", "/api/lock", None).await;
    assert_eq!(call(&ra, "POST", "/api/unlock", Some(json!({ "passcode": "alice-passcode" }))).await.0, StatusCode::UNAUTHORIZED);

    // Panic wipes the device.
    assert_eq!(call(&rb, "POST", "/api/panic", None).await.0, StatusCode::OK);
    assert_eq!(call(&rb, "GET", "/api/status", None).await.1["state"], "new");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_sync_and_reply() {
    let url = relay().await;
    let dir = tempfile::tempdir().unwrap();
    let (_agent, r) = desk(dir.path(), &url);
    call(&r, "POST", "/api/setup", Some(json!({ "passcode": "desk-passcode" }))).await;

    // Start a real drop server configured with this desk's keys.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let drop_url = format!("http://{}", listener.local_addr().unwrap());
    let (s, d) = call(&r, "POST", "/api/drops", Some(json!({ "name": "Tips", "url": drop_url }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let drop_id = d["id"].as_str().unwrap().to_string();
    let data = tempfile::tempdir().unwrap();
    let cfg = sotto_drop::DropConfig {
        name: "Outlet".into(),
        instance_id: sotto_core::hex(&[1u8; 16]),
        newsroom_key: d["newsroom_key"].as_str().unwrap().into(),
        journalists: vec![sotto_drop::JournalistKey { name: "desk".into(), sign_pk: d["journalist_key"].as_str().unwrap().into() }],
        data_dir: data.path().to_path_buf(),
        listen: String::new(),
        max_upload_mb: 5,
        argon_m_kib: 64,
        argon_t: 1,
        intro: String::new(),
        verify_hint: String::new(),
    };
    let app = sotto_drop::App::new(cfg).unwrap();
    tokio::spawn(async move { axum::serve(listener, sotto_drop::router(app)).await.unwrap() });

    // A source leaves a message (written the way the drop stores it).
    let newsroom = sotto_drop::proto::decode_key(d["newsroom_key"].as_str().unwrap()).unwrap();
    let source = sotto_drop::proto::source_secret("abandon ability able about above absent absorb abstract", &[1u8; 16], 64, 1).unwrap();
    let sid = sotto_drop::proto::source_id(&source.public());
    let sdir = data.path().join("sources").join(&sid);
    std::fs::create_dir_all(sdir.join("sub")).unwrap();
    std::fs::create_dir_all(sdir.join("rep")).unwrap();
    std::fs::write(sdir.join("key.bin"), postcard::to_stdvec(&source.public()).unwrap()).unwrap();
    let meta = sotto_drop::proto::SubmissionMeta { kind: sotto_drop::proto::SubmissionKind::Message, day: 20_000 };
    let mut plain = sotto_drop::proto::submission_prefix(&meta).unwrap();
    plain.extend_from_slice(b"They shredded the originals.");
    std::fs::write(sdir.join("sub").join("d20000-00112233445566ff.sealed"), sotto_core::seal::seal(&newsroom, &plain).unwrap()).unwrap();

    let (s, v) = call(&r, "POST", &format!("/api/drops/{drop_id}/sync"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["added"], 1);
    assert!(!sdir.join("sub").join("d20000-00112233445566ff.sealed").exists(), "deleted from the drop after download");
    let (_, sources) = call(&r, "GET", &format!("/api/drops/{drop_id}/sources"), None).await;
    assert_eq!(sources[0]["id"], sid.as_str());
    assert!(sources[0]["pseudonym"].as_str().unwrap().starts_with("Sender "));
    assert_eq!(sources[0]["unread"], 1);
    let (_, items) = call(&r, "GET", &format!("/api/drops/{drop_id}/sources/{sid}"), None).await;
    assert_eq!(items[0]["text"], "They shredded the originals.");

    // Reply; the source can read it with their codename-derived key.
    let (s, v) = call(&r, "POST", &format!("/api/drops/{drop_id}/sources/{sid}/reply"), Some(json!({ "text": "Thank you. Can you say when?" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let reply_file = std::fs::read_dir(sdir.join("rep")).unwrap().next().unwrap().unwrap().path();
    let opened = sotto_core::seal::open(&source, &std::fs::read(reply_file).unwrap()).unwrap();
    let reply: sotto_drop::proto::Reply = postcard::from_bytes(&opened).unwrap();
    assert_eq!(reply.text, "Thank you. Can you say when?");
    let (_, items) = call(&r, "GET", &format!("/api/drops/{drop_id}/sources/{sid}"), None).await;
    assert!(items.as_array().unwrap().iter().any(|i| i["kind"] == "reply"));
}

async fn relay_with_timelocks(dir: &std::path::Path) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(sotto_relay::serve(listener, sotto_relay::Config { data_dir: Some(dir.to_path_buf()), ..Default::default() }));
    format!("ws://{addr}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case_team_space_and_insurance() {
    let tl = tempfile::tempdir().unwrap();
    let url = relay_with_timelocks(tl.path()).await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_a, ra) = desk(da.path(), &url);
    let (_b, rb) = desk(db.path(), &url);
    call(&ra, "POST", "/api/setup", Some(json!({ "passcode": "owner-passcode" }))).await;
    call(&rb, "POST", "/api/setup", Some(json!({ "passcode": "editor-passcode" }))).await;
    call(&ra, "PATCH", "/api/settings", Some(json!({ "display_name": "Ada" }))).await;
    call(&rb, "PATCH", "/api/settings", Some(json!({ "display_name": "Ben" }))).await;

    // Connect the two desks.
    let (_, inv) = call(&ra, "POST", "/api/invites", Some(json!({ "nickname": "Ben (editor)", "label": "Ada" }))).await;
    let a_cid = inv["contact_id"].as_str().unwrap().to_string();
    call(&rb, "POST", "/api/invites/accept", Some(json!({ "link": inv["link"], "share_name": "Ben" }))).await;
    until("ready", || async { call(&ra, "GET", &format!("/api/contacts/{a_cid}"), None).await.1["state"] == "ready" }).await;

    // Owner opens a case and invites the editor.
    let (s, case) = call(&ra, "POST", "/api/cases", Some(json!({ "title": "Harbour contracts" }))).await;
    assert_eq!(s, StatusCode::OK, "{case}");
    let case_id = case["id"].as_str().unwrap().to_string();
    let (s, v) = call(&ra, "POST", &format!("/api/cases/{case_id}/members"), Some(json!({ "contact_id": a_cid, "role": "editor" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("editor joins", || async { call(&rb, "GET", "/api/cases", None).await.1.as_array().map(|a| !a.is_empty()).unwrap_or(false) }).await;
    until("member confirmed", || async {
        call(&ra, "GET", &format!("/api/cases/{case_id}"), None).await.1["members"].as_array().unwrap().iter().any(|m| m["name"] == "Ben" && m["pending"] == false)
    })
    .await;

    // Notes both ways, sources by pseudonym only.
    call(&ra, "POST", &format!("/api/cases/{case_id}/notes"), Some(json!({ "text": "Tender records requested from the port." }))).await;
    call(&ra, "POST", &format!("/api/cases/{case_id}/sources"), Some(json!({ "alias": "Source QV4", "note": "Procurement, two years" }))).await;
    until("editor sees notes", || async {
        let e = call(&rb, "GET", &format!("/api/cases/{case_id}"), None).await.1;
        let entries = e["entries"].as_array().cloned().unwrap_or_default();
        entries.iter().any(|x| x["text"] == "Tender records requested from the port." && x["author"] == "Ada") && entries.iter().any(|x| x["alias"] == "Source QV4")
    })
    .await;
    call(&rb, "POST", &format!("/api/cases/{case_id}/notes"), Some(json!({ "text": "Legal says we can name the company." }))).await;
    until("owner sees editor note", || async {
        call(&ra, "GET", &format!("/api/cases/{case_id}"), None).await.1["entries"].as_array().unwrap().iter().any(|x| x["text"] == "Legal says we can name the company." && x["author"] == "Ben")
    })
    .await;
    let (_, members) = call(&rb, "GET", &format!("/api/cases/{case_id}"), None).await;
    assert!(members["members"].as_array().unwrap().iter().any(|m| m["name"] == "Ada" && m["role"] == "owner"));

    // Only the owner sets retention; the editor's desk follows.
    let (s, _) = call(&rb, "PATCH", &format!("/api/cases/{case_id}"), Some(json!({ "legal_hold": true }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    call(&ra, "PATCH", &format!("/api/cases/{case_id}"), Some(json!({ "legal_hold": true }))).await;
    until("hold synced", || async { call(&rb, "GET", &format!("/api/cases/{case_id}"), None).await.1["legal_hold"] == true }).await;

    // Insurance file: the editor holds the key, the relay holds the locked package.
    let (s, v) = call(&ra, "POST", &format!("/api/cases/{case_id}/insurance"), Some(json!({ "interval_days": 7, "contact_ids": [a_cid] }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("grant arrives", || async { call(&rb, "GET", "/api/grants", None).await.1.as_array().map(|a| a.len() == 1).unwrap_or(false) }).await;
    let (_, grants) = call(&rb, "GET", "/api/grants", None).await;
    let rid = grants[0]["read_id"].as_str().unwrap().to_string();
    let (_, opened) = call(&rb, "POST", &format!("/api/grants/{rid}/open"), None).await;
    assert_eq!(opened["locked"], true, "locked before the deadline: {opened}");
    let (s, v) = call(&ra, "POST", &format!("/api/cases/{case_id}/insurance/checkin"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Destroying the case shreds it on both desks.
    call(&ra, "DELETE", &format!("/api/cases/{case_id}"), None).await;
    assert!(call(&ra, "GET", "/api/cases", None).await.1.as_array().unwrap().is_empty());
    until("editor copy destroyed", || async { call(&rb, "GET", "/api/cases", None).await.1.as_array().unwrap().is_empty() }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn travel_mode_seals_wipes_and_restores() {
    let url = relay().await;
    let dir = tempfile::tempdir().unwrap();
    let usb = tempfile::tempdir().unwrap();
    let (_a, r) = desk(dir.path(), &url);
    call(&r, "POST", "/api/setup", Some(json!({ "passcode": "travel-passcode" }))).await;
    call(&r, "POST", "/api/cases", Some(json!({ "title": "Border story" }))).await;
    let (_, p) = call(&r, "POST", "/api/travel/prepare", None).await;
    let words: Vec<String> = p["words"].as_str().unwrap().split(' ').map(String::from).collect();
    assert_eq!(words.len(), 24);
    let path = usb.path().join("sotto-travel.bin");
    let (s, _) = call(&r, "POST", "/api/travel/start", Some(json!({ "path": path, "check": [[0, "wrong"], [5, words[5]], [9, words[9]]] }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "wrong check words are refused");
    let (s, v) = call(&r, "POST", "/api/travel/start", Some(json!({ "path": path, "check": [[0, words[0]], [5, words[5]], [23, words[23]]] }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(call(&r, "GET", "/api/status", None).await.1["state"], "new", "the device is erased");
    let backup = std::fs::read(&path).unwrap();
    assert!(!backup.windows(11).any(|w| w == b"Border story"));

    let (s, _) = call(&r, "POST", "/api/travel/restore", Some(json!({ "path": path, "words": "abandon ".repeat(23) + "art" }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "another phrase does not open it");
    let (s, v) = call(&r, "POST", "/api/travel/restore", Some(json!({ "path": path, "words": words.join(" ") }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(call(&r, "GET", "/api/status", None).await.1["state"], "locked");
    assert_eq!(call(&r, "POST", "/api/unlock", Some(json!({ "passcode": "travel-passcode" }))).await.0, StatusCode::OK);
    let (_, cases) = call(&r, "GET", "/api/cases", None).await;
    assert_eq!(cases[0]["title"], "Border story");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn groups_names_messages_and_removal() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_a, ra) = desk(da.path(), &url);
    let (_b, rb) = desk(db.path(), &url);
    call(&ra, "POST", "/api/setup", Some(json!({ "passcode": "owner-passcode" }))).await;
    call(&rb, "POST", "/api/setup", Some(json!({ "passcode": "member-passcode" }))).await;
    call(&ra, "PATCH", "/api/settings", Some(json!({ "display_name": "Ada" }))).await;
    call(&rb, "PATCH", "/api/settings", Some(json!({ "display_name": "Ben" }))).await;

    // Ada knows Ben by a private nickname, and Ben shares no name with the conversation.
    let (_, inv) = call(&ra, "POST", "/api/invites", Some(json!({ "nickname": "Ben (editor)", "label": "Ada" }))).await;
    let a_cid = inv["contact_id"].as_str().unwrap().to_string();
    call(&rb, "POST", "/api/invites/accept", Some(json!({ "link": inv["link"] }))).await;
    until("ready", || async { call(&ra, "GET", &format!("/api/contacts/{a_cid}"), None).await.1["state"] == "ready" }).await;

    let (s, g) = call(&ra, "POST", "/api/groups", Some(json!({ "title": "Port contracts", "members": [a_cid] }))).await;
    assert_eq!(s, StatusCode::OK, "{g}");
    let gid = g["id"].as_str().unwrap().to_string();
    until("Ben joins", || async { call(&rb, "GET", "/api/groups", None).await.1.as_array().map(|a| !a.is_empty()).unwrap_or(false) }).await;
    until("member confirmed", || async {
        call(&ra, "GET", &format!("/api/groups/{gid}"), None).await.1["members"].as_array().unwrap().iter().any(|m| m["pending"] == false && !m["owner"].as_bool().unwrap())
    })
    .await;

    // The shared name is the one Ben signed into his own key package, never Ada's nickname.
    let (_, ga) = call(&ra, "GET", &format!("/api/groups/{gid}"), None).await;
    let names: Vec<&str> = ga["members"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Ada", "Ben"], "{ga}");
    assert!(ga["members"][0]["you"] == true && ga["members"][1]["you"] == false);
    until("member list synced", || async { call(&rb, "GET", &format!("/api/groups/{gid}"), None).await.1["member_count"] == 2 }).await;
    let (_, gb) = call(&rb, "GET", &format!("/api/groups/{gid}"), None).await;
    assert!(!gb.to_string().contains("Ben (editor)"), "nickname leaked: {gb}");
    let ada = &gb["members"][0];
    assert!(ada["owner"] == true && ada["contact_id"].is_string(), "owner linked to Ben's conversation with Ada: {gb}");
    assert!(gb["members"][1]["you"] == true);

    // Messages both ways, each with its author's chosen name.
    call(&ra, "POST", &format!("/api/groups/{gid}/messages"), Some(json!({ "text": "Manifests are in." }))).await;
    until("Ben reads it", || async {
        call(&rb, "GET", &format!("/api/groups/{gid}/messages"), None).await.1.as_array().unwrap().iter().any(|m| m["text"] == "Manifests are in." && m["author"] == "Ada")
    })
    .await;
    call(&rb, "POST", &format!("/api/groups/{gid}/messages"), Some(json!({ "text": "Pulling the registry." }))).await;
    until("Ada reads it", || async {
        call(&ra, "GET", &format!("/api/groups/{gid}/messages"), None).await.1.as_array().unwrap().iter().any(|m| m["text"] == "Pulling the registry." && m["author"] == "Ben")
    })
    .await;

    // Only the owner removes people, by conversation; Ben's desk learns it's out.
    let (s, _) = call(&rb, "DELETE", &format!("/api/groups/{gid}/members/{a_cid}"), None).await;
    assert_ne!(s, StatusCode::OK);
    let (s, v) = call(&ra, "DELETE", &format!("/api/groups/{gid}/members/{a_cid}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["member_count"], 1);
    until("Ben removed", || async { call(&rb, "GET", &format!("/api/groups/{gid}"), None).await.1["removed"] == true }).await;
}

/// Connect `owner` to a new desk: returns the owner-side contact id.
async fn connect(owner: &Router, other: &Router, nickname: &str) -> String {
    let (_, inv) = call(owner, "POST", "/api/invites", Some(json!({ "nickname": nickname, "label": "Desk" }))).await;
    let cid = inv["contact_id"].as_str().unwrap().to_string();
    call(other, "POST", "/api/invites/accept", Some(json!({ "link": inv["link"] }))).await;
    until("ready", || async { call(owner, "GET", &format!("/api/contacts/{cid}"), None).await.1["state"] == "ready" }).await;
    cid
}

async fn first_channel(r: &Router) -> Option<Value> {
    call(r, "GET", "/api/channels", None).await.1.as_array().and_then(|a| a.first().cloned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channels_follow_post_reply_and_stay_private() {
    let url = relay().await;
    let (da, db, dc) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_a, ra) = desk(da.path(), &url);
    let (_b, rb) = desk(db.path(), &url);
    let (_c, rc) = desk(dc.path(), &url);
    for (r, p) in [(&ra, "owner-passcode"), (&rb, "first-passcode"), (&rc, "second-passcode")] {
        call(r, "POST", "/api/setup", Some(json!({ "passcode": p }))).await;
    }
    let ben = connect(&ra, &rb, "Ben").await;
    let kemi = connect(&ra, &rc, "Kemi").await;

    // Ada makes a channel and invites Ben; she posts before he follows.
    let (s, ch) = call(&ra, "POST", "/api/channels", Some(json!({ "title": "Safety alerts", "about": "Checks before you travel", "invite": [ben] }))).await;
    assert_eq!(s, StatusCode::OK, "{ch}");
    let id = ch["id"].as_str().unwrap().to_string();
    call(&ra, "POST", &format!("/api/channels/{id}/posts"), Some(json!({ "text": "Border crossing at Seme is slow this week." }))).await;
    until("invite arrives", || async { first_channel(&rb).await.is_some_and(|c| c["invited"] == true) }).await;
    let inv = first_channel(&rb).await.unwrap();
    assert_eq!(inv["title"], "Safety alerts");
    // Before joining, nothing is posted to Ben, and he can't reply.
    assert!(call(&rb, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().is_empty());

    call(&rb, "POST", &format!("/api/channels/{id}/join"), None).await;
    until("owner sees the follower", || async { call(&ra, "GET", &format!("/api/channels/{id}"), None).await.1["subscriber_count"] == 1 }).await;
    until("history catches up", || async {
        call(&rb, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().iter().any(|p| p["body"] == "Border crossing at Seme is slow this week.")
    })
    .await;

    // Kemi follows too. Neither follower's desk has any trace of the other, or of the count.
    call(&ra, "POST", &format!("/api/channels/{id}/subscribers"), Some(json!({ "contact_ids": [kemi] }))).await;
    until("Kemi invited", || async { first_channel(&rc).await.is_some() }).await;
    call(&rc, "POST", &format!("/api/channels/{id}/join"), None).await;
    until("two followers", || async { call(&ra, "GET", &format!("/api/channels/{id}"), None).await.1["subscriber_count"] == 2 }).await;
    for r in [&rb, &rc] {
        let v = call(r, "GET", &format!("/api/channels/{id}"), None).await.1;
        assert!(v["subscribers"].is_null() && v["subscriber_count"].is_null(), "a follower sees the audience: {v}");
    }

    // A new post reaches both; an edit and a delete follow it.
    let (_, p) = call(&ra, "POST", &format!("/api/channels/{id}/posts"), Some(json!({ "text": "Avoid the Ikeja office today." }))).await;
    let pid = p["id"].as_str().unwrap().to_string();
    for r in [&rb, &rc] {
        until("post arrives", || async { call(r, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().len() == 2 }).await;
    }
    call(&ra, "PATCH", &format!("/api/channels/{id}/posts/{pid}"), Some(json!({ "text": "Avoid the Ikeja office until Friday." }))).await;
    until("edit arrives", || async {
        call(&rc, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().iter().any(|p| p["body"] == "Avoid the Ikeja office until Friday." && p["edited"] == true)
    })
    .await;
    // Followers can't post.
    let (s, _) = call(&rb, "POST", &format!("/api/channels/{id}/posts"), Some(json!({ "text": "hello all" }))).await;
    assert_ne!(s, StatusCode::OK);

    // Ben replies privately: Ada sees it with who sent it; Kemi never does.
    let (s, v) = call(&rb, "POST", &format!("/api/channels/{id}/posts/{pid}/replies"), Some(json!({ "text": "Understood, moving the meeting." }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("Ada gets the reply", || async {
        call(&ra, "GET", &format!("/api/channels/{id}/posts/{pid}/replies"), None).await.1.as_array().unwrap().iter().any(|r| r["body"] == "Understood, moving the meeting." && r["contact_id"] == ben.as_str())
    })
    .await;
    assert!(call(&rc, "GET", &format!("/api/channels/{id}/posts/{pid}/replies"), None).await.1.as_array().unwrap().is_empty());

    call(&ra, "DELETE", &format!("/api/channels/{id}/posts/{pid}"), None).await;
    until("delete arrives", || async { call(&rb, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().len() == 1 }).await;

    // Kemi leaves; Ada removes Ben; then Ada closes it.
    call(&rc, "DELETE", &format!("/api/channels/{id}"), None).await;
    until("Kemi left", || async { call(&ra, "GET", &format!("/api/channels/{id}"), None).await.1["subscriber_count"] == 1 }).await;
    assert!(first_channel(&rc).await.is_none());
    call(&ra, "DELETE", &format!("/api/channels/{id}/subscribers/{ben}"), None).await;
    until("Ben removed", || async { first_channel(&rb).await.is_some_and(|c| c["closed"] == true && c["removed"] == true) }).await;
    let (s, _) = call(&rb, "POST", &format!("/api/channels/{id}/posts/{pid}/replies"), Some(json!({ "text": "still there?" }))).await;
    assert_ne!(s, StatusCode::OK);
    call(&ra, "DELETE", &format!("/api/channels/{id}"), None).await;
    assert!(first_channel(&ra).await.is_none());
}

/// Upload raw bytes like the app does (file name and caption in headers).
async fn upload(r: &Router, path: &str, name: &str, caption: &str, bytes: &[u8]) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::HOST, "127.0.0.1")
        .header(header::CONTENT_TYPE, "text/plain")
        .header("x-file-name", name)
        .header("x-caption", caption)
        .body(Body::from(bytes.to_vec()))
        .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn named_desk(dir: &std::path::Path, url: &str, pass: &str, name: &str) -> Router {
    let (_a, r) = desk(dir, url);
    std::mem::forget(_a);
    call(&r, "POST", "/api/setup", Some(json!({ "passcode": pass }))).await;
    call(&r, "PATCH", "/api/settings", Some(json!({ "display_name": name }))).await;
    r
}

async fn group_view(r: &Router, id: &str) -> Value {
    call(r, "GET", &format!("/api/groups/{id}"), None).await.1
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn groups_admins_pins_files_and_safety() {
    let url = relay().await;
    let (da, db, dc) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ra = named_desk(da.path(), &url, "owner-passcode", "Ada").await;
    let rb = named_desk(db.path(), &url, "admin-passcode", "Ben").await;
    let rc = named_desk(dc.path(), &url, "third-passcode", "Kemi").await;
    // Ada knows Ben; only Ben knows Kemi.
    let ben = connect(&ra, &rb, "Ben").await;
    let kemi_for_ben = connect(&rb, &rc, "Kemi").await;

    let (_, g) = call(&ra, "POST", "/api/groups", Some(json!({ "title": "Port contracts", "members": [ben] }))).await;
    let id = g["id"].as_str().unwrap().to_string();
    until("Ben in", || async { group_view(&rb, &id).await["member_count"] == 2 }).await;

    // Ada makes Ben an admin; Ben's desk learns it.
    let (s, v) = call(&ra, "POST", &format!("/api/groups/{id}/admins"), Some(json!({ "name": "Ben", "on": true }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("Ben is admin", || async { group_view(&rb, &id).await["am_admin"] == true }).await;
    // Kemi can't be made admin by Ben (only the creator chooses admins).
    let (s, _) = call(&rb, "POST", &format!("/api/groups/{id}/admins"), Some(json!({ "name": "Ada", "on": true }))).await;
    assert_ne!(s, StatusCode::OK);

    // Ben adds Kemi, whom Ada has never talked to: the creator's desk commits, Ben delivers the Welcome.
    let (s, v) = call(&rb, "POST", &format!("/api/groups/{id}/members"), Some(json!({ "contact_id": kemi_for_ben }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("Kemi joins", || async { call(&rc, "GET", "/api/groups", None).await.1.as_array().is_some_and(|a| !a.is_empty()) }).await;
    for r in [&ra, &rb, &rc] {
        until("three members everywhere", || async { group_view(r, &id).await["member_count"] == 3 }).await;
    }

    // Everyone computes the same safety number.
    let mut numbers = Vec::new();
    for r in [&ra, &rb, &rc] {
        until("safety has three", || async { call(r, "GET", &format!("/api/groups/{id}/safety"), None).await.1["members"] == 3 }).await;
        numbers.push(call(r, "GET", &format!("/api/groups/{id}/safety"), None).await.1["number"].clone());
    }
    assert!(numbers[0].is_string() && numbers.iter().all(|n| n == &numbers[0]), "{numbers:?}");

    // Ada posts; Ben (admin) pins it; everyone sees the pin. Kemi (not admin) can't pin.
    let (_, m) = call(&ra, "POST", &format!("/api/groups/{id}/messages"), Some(json!({ "text": "Meeting moved to Thursday." }))).await;
    let mid = m["id"].as_str().unwrap().to_string();
    until("Ben has it", || async { call(&rb, "GET", &format!("/api/groups/{id}/messages"), None).await.1.as_array().unwrap().iter().any(|x| x["id"] == mid.as_str()) }).await;
    let (s, v) = call(&rb, "POST", &format!("/api/groups/{id}/messages/{mid}/pin"), Some(json!({ "on": true }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for r in [&ra, &rc] {
        until("pin everywhere", || async { group_view(r, &id).await["pins"].as_array().unwrap().iter().any(|p| p == mid.as_str()) }).await;
    }
    let (s, _) = call(&rc, "POST", &format!("/api/groups/{id}/messages/{mid}/pin"), Some(json!({ "on": false }))).await;
    assert_ne!(s, StatusCode::OK);

    // Kemi shares a file: it arrives on Ada's desk, fetched and quarantined.
    let (s, v) = upload(&rc, &format!("/api/groups/{id}/files"), "manifest.txt", "the list", b"container 4471, container 4471").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("file arrives", || async {
        call(&ra, "GET", &format!("/api/groups/{id}/messages"), None).await.1.as_array().unwrap().iter().any(|x| {
            x["kind"] == "file" && x["file"]["name"] == "manifest.txt" && x["file"]["status"] == "ready" && x["file"]["review"]["state"] == "quarantined" && x["author"] == "Kemi"
        })
    })
    .await;
    let files = call(&ra, "GET", "/api/files", None).await.1;
    assert!(files.as_array().unwrap().iter().any(|f| f["origin"] == "group" && f["name"] == "manifest.txt" && f["review_path"].is_string()), "{files}");

    // Ben (admin) asks to remove Kemi; Ada's desk carries it out.
    let (s, v) = call(&rb, "DELETE", &format!("/api/groups/{id}/members/Kemi"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("Kemi removed", || async { group_view(&rc, &id).await["removed"] == true }).await;
    until("two left", || async { group_view(&ra, &id).await["member_count"] == 2 }).await;

    // Mute is this desk's own setting.
    call(&rb, "PATCH", &format!("/api/groups/{id}"), Some(json!({ "muted": true }))).await;
    assert_eq!(group_view(&rb, &id).await["muted"], true);
    assert_eq!(group_view(&ra, &id).await["muted"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channels_files_receipts_and_mute() {
    let url = relay().await;
    let (da, db, dc) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ra = named_desk(da.path(), &url, "owner-passcode", "Ada").await;
    let rb = named_desk(db.path(), &url, "first-passcode", "Ben").await;
    let rc = named_desk(dc.path(), &url, "second-passcode", "Kemi").await;
    let ben = connect(&ra, &rb, "Ben").await;
    let kemi = connect(&ra, &rc, "Kemi").await;

    let (_, ch) = call(&ra, "POST", "/api/channels", Some(json!({ "title": "Safety alerts", "invite": [ben] }))).await;
    let id = ch["id"].as_str().unwrap().to_string();
    until("invite", || async { first_channel(&rb).await.is_some() }).await;
    call(&rb, "POST", &format!("/api/channels/{id}/join"), None).await;
    until("joined", || async { call(&ra, "GET", &format!("/api/channels/{id}"), None).await.1["subscriber_count"] == 1 }).await;

    // Receipts are the follower's choice; the owner can't turn them on.
    let (s, _) = call(&ra, "PATCH", &format!("/api/channels/{id}"), Some(json!({ "receipts": true }))).await;
    assert_ne!(s, StatusCode::OK);
    call(&rb, "PATCH", &format!("/api/channels/{id}"), Some(json!({ "receipts": true }))).await;

    // A text post and a file post.
    let (_, p) = call(&ra, "POST", &format!("/api/channels/{id}/posts"), Some(json!({ "text": "Read the attached checklist." }))).await;
    let pid = p["id"].as_str().unwrap().to_string();
    let (s, v) = upload(&ra, &format!("/api/channels/{id}/files"), "checklist.txt", "Before you travel", b"1. tell the desk 2. carry no notes").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("file reaches Ben", || async {
        call(&rb, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().iter().any(|p| p["file"]["name"] == "checklist.txt" && p["file"]["status"] == "ready" && p["file"]["caption"] == "Before you travel")
    })
    .await;

    // Ben reads: Ada sees it was read; without receipts nothing would be sent.
    call(&rb, "POST", &format!("/api/channels/{id}/read"), None).await;
    until("seen", || async {
        call(&ra, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().iter().any(|p| p["id"] == pid.as_str() && p["seen"] == 1)
    })
    .await;
    assert!(call(&rb, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().iter().all(|p| p["seen"].is_null()));

    // Kemi joins late and still gets the file, from a fresh copy made for her.
    call(&ra, "POST", &format!("/api/channels/{id}/subscribers"), Some(json!({ "contact_ids": [kemi] }))).await;
    until("Kemi invited", || async { first_channel(&rc).await.is_some() }).await;
    call(&rc, "POST", &format!("/api/channels/{id}/join"), None).await;
    until("late file", || async {
        call(&rc, "GET", &format!("/api/channels/{id}/posts"), None).await.1.as_array().unwrap().iter().any(|p| p["file"]["name"] == "checklist.txt" && p["file"]["status"] == "ready")
    })
    .await;

    // Mute is local.
    call(&rc, "PATCH", &format!("/api/channels/{id}"), Some(json!({ "muted": true }))).await;
    assert_eq!(first_channel(&rc).await.unwrap()["muted"], true);
}

async fn hook_post(r: &Router, path: &str, host: &str, token: &str, body: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integrations_post_and_secrets_open_once() {
    let url = relay().await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ra = named_desk(da.path(), &url, "owner-passcode", "Ada").await;
    let rb = named_desk(db.path(), &url, "member-passcode", "Ben").await;
    let ben = connect(&ra, &rb, "Ben").await;

    // An integration that posts into a group.
    let (_, g) = call(&ra, "POST", "/api/groups", Some(json!({ "title": "Deploys", "members": [ben] }))).await;
    let gid = g["id"].as_str().unwrap().to_string();
    until("Ben in", || async { group_view(&rb, &gid).await["member_count"] == 2 }).await;
    let (s, h) = call(&ra, "POST", "/api/hooks", Some(json!({ "name": "CI alerts", "kind": "group", "id": gid }))).await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let (hid, token) = (h["id"].as_str().unwrap().to_string(), h["token"].as_str().unwrap().to_string());
    assert!(call(&ra, "GET", "/api/hooks", None).await.1[0]["token"].is_null(), "the token is shown once only");

    // Wrong token, or a request that isn't from this computer: refused, nothing posted.
    assert_eq!(hook_post(&ra, &format!("/hooks/{hid}"), "127.0.0.1", "sotto_hook_wrong", "x").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(hook_post(&ra, &format!("/hooks/{hid}"), "evil.example", &token, "x").await.0, StatusCode::UNAUTHORIZED);
    // The app's own token doesn't open the hook either, and the hook token doesn't open the app.
    assert_eq!(hook_post(&ra, &format!("/hooks/{hid}"), "127.0.0.1", TOKEN, "x").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(hook_post(&ra, "/api/contacts", "127.0.0.1", &token, "").await.0, StatusCode::UNAUTHORIZED);

    let (s, v) = hook_post(&ra, &format!("/hooks/{hid}"), "127.0.0.1", &token, "Build 412 passed").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("Ben sees the alert, labelled", || async {
        call(&rb, "GET", &format!("/api/groups/{gid}/messages"), None).await.1.as_array().unwrap().iter().any(|m| m["text"] == "Build 412 passed" && m["author"] == "CI alerts (via Ada)")
    })
    .await;
    assert_eq!(call(&ra, "GET", "/api/hooks", None).await.1[0]["posts"], 1);
    call(&ra, "DELETE", &format!("/api/hooks/{hid}"), None).await;
    assert_eq!(hook_post(&ra, &format!("/hooks/{hid}"), "127.0.0.1", &token, "again").await.0, StatusCode::UNAUTHORIZED);

    // A one-time secret: Ben can read it once; Ada never keeps the text.
    let (s, v) = call(&ra, "POST", &format!("/api/contacts/{ben}/secrets"), Some(json!({ "label": "Staging DB password", "text": "hunter2-staging", "ttl_secs": 3600 }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let sid = v["id"].as_str().unwrap().to_string();
    let ada_side = call(&ra, "GET", &format!("/api/contacts/{ben}/secrets"), None).await.1;
    assert!(!ada_side.to_string().contains("hunter2"), "{ada_side}");
    let ada_on_ben = call(&rb, "GET", "/api/contacts", None).await.1[0]["id"].as_str().unwrap().to_string();
    until("secret arrives", || async { call(&rb, "GET", &format!("/api/contacts/{ada_on_ben}/secrets"), None).await.1.as_array().is_some_and(|a| a.len() == 1) }).await;
    let listed = call(&rb, "GET", &format!("/api/contacts/{ada_on_ben}/secrets"), None).await.1;
    assert!(listed[0]["state"] == "waiting" && !listed.to_string().contains("hunter2"), "listing never shows the text: {listed}");
    let (s, v) = call(&rb, "POST", &format!("/api/contacts/{ada_on_ben}/secrets/{sid}/open"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["text"], "hunter2-staging");
    let (s, _) = call(&rb, "POST", &format!("/api/contacts/{ada_on_ben}/secrets/{sid}/open"), None).await;
    assert_ne!(s, StatusCode::OK, "a second open must fail");
    until("Ada sees it was opened", || async { call(&ra, "GET", &format!("/api/contacts/{ben}/secrets"), None).await.1[0]["state"] == "opened" }).await;
    let (s, _) = call(&ra, "POST", &format!("/api/contacts/{ben}/secrets/{sid}/open"), None).await;
    assert_ne!(s, StatusCode::OK, "the sender can't open it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tags_let_people_connect_without_a_link() {
    let url = relay().await;
    let (da, db, dc) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ra = named_desk(da.path(), &url, "owner-passcode", "Ada").await;
    let rb = named_desk(db.path(), &url, "second-passcode", "Ben").await;
    let rc = named_desk(dc.path(), &url, "third-passcode", "Kemi").await;

    // Ada claims a tag; her stocked invites don't show up as conversations.
    let (s, v) = call(&ra, "PUT", "/api/tag", Some(json!({ "tag": "@Ada.M" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tag"], "ada.m");
    assert!(call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().is_empty());
    // Nobody else can take it.
    let (s, v) = call(&rb, "PUT", "/api/tag", Some(json!({ "tag": "ada.m" }))).await;
    assert_ne!(s, StatusCode::OK, "{v}");
    assert!(call(&rb, "PUT", "/api/tag", Some(json!({ "tag": "1x" }))).await.0 != StatusCode::OK, "too short and starts with a digit");

    // Ben reaches her by tag alone.
    let (s, v) = call(&rb, "POST", "/api/tag/connect", Some(json!({ "tag": "@ada.m" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ada_for_ben = v["contact_id"].as_str().unwrap().to_string();
    until("Ada sees a request", || async {
        call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().iter().any(|c| c["request_tag"] == "ada.m" && c["their_name"] == "Ben" && c["state"] == "ready")
    })
    .await;
    // They can talk straight away.
    call(&rb, "POST", &format!("/api/contacts/{ada_for_ben}/messages"), Some(json!({ "text": "Hi Ada, Ben from platform." }))).await;
    let ben = call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().iter().find(|c| c["their_name"] == "Ben").unwrap()["id"].as_str().unwrap().to_string();
    until("message arrives", || async {
        call(&ra, "GET", &format!("/api/contacts/{ben}/messages"), None).await.1.as_array().unwrap().iter().any(|m| m["text"] == "Hi Ada, Ben from platform.")
    })
    .await;
    // Accepting clears the request; the stock is topped back up.
    call(&ra, "POST", &format!("/api/tag/requests/{ben}/accept"), None).await;
    assert!(call(&ra, "GET", "/api/contacts", None).await.1[0]["request_tag"].is_null());
    until("restocked", || async { call(&ra, "GET", "/api/tag", None).await.1["ready"] == 10 }).await;

    // Kemi connects, and Ada removes her.
    call(&rc, "POST", "/api/tag/connect", Some(json!({ "tag": "ada.m" }))).await;
    until("second request", || async { call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().len() == 2 }).await;
    let kemi = call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().iter().find(|c| c["request_tag"] == "ada.m").unwrap()["id"].as_str().unwrap().to_string();
    let (s, _) = call(&ra, "DELETE", &format!("/api/tag/requests/{kemi}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(call(&ra, "GET", "/api/contacts", None).await.1.as_array().unwrap().len(), 1);

    // Releasing the tag: nobody can use it any more, and it's free for someone else.
    call(&ra, "DELETE", "/api/tag", None).await;
    let (s, _) = call(&rc, "POST", "/api/tag/connect", Some(json!({ "tag": "ada.m" }))).await;
    assert_ne!(s, StatusCode::OK);
    let (s, v) = call(&rb, "PUT", "/api/tag", Some(json!({ "tag": "ada.m" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chosen_avatars_reach_contacts() {
    let url = relay().await;
    let (da, db, dc) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ra = named_desk(da.path(), &url, "owner-passcode", "Ada").await;
    let rb = named_desk(db.path(), &url, "second-passcode", "Ben").await;
    let rc = named_desk(dc.path(), &url, "third-passcode", "Kemi").await;
    connect(&ra, &rb, "Ben").await;
    // Only ids from the collection are accepted.
    assert_ne!(call(&ra, "PUT", "/api/me/avatar", Some(json!({ "id": "v1:99:0:0" }))).await.0, StatusCode::OK);
    assert_ne!(call(&ra, "PUT", "/api/me/avatar", Some(json!({ "id": "https://example.com/x.png" }))).await.0, StatusCode::OK);
    let (s, v) = call(&ra, "PUT", "/api/me/avatar", Some(json!({ "id": "v1:7:3:1" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until("Ben sees Ada's avatar", || async { call(&rb, "GET", "/api/contacts", None).await.1[0]["avatar"] == "v1:7:3:1" }).await;
    // Someone who connects later hears it too.
    connect(&ra, &rc, "Kemi").await;
    until("Kemi sees it", || async { call(&rc, "GET", "/api/contacts", None).await.1[0]["avatar"] == "v1:7:3:1" }).await;
    // Back to the default.
    call(&ra, "PUT", "/api/me/avatar", Some(json!({ "id": null }))).await;
    until("Ben sees the default again", || async { call(&rb, "GET", "/api/contacts", None).await.1[0]["avatar"].is_null() }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tags_work_across_relays() {
    // Two organizations, each with its own relay.
    let (north, south) = (relay().await, relay().await);
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ra = named_desk(da.path(), &north, "owner-passcode", "Ada").await;
    let rb = named_desk(db.path(), &south, "second-passcode", "Ben").await;

    let (_, v) = call(&ra, "PUT", "/api/tag", Some(json!({ "tag": "ada" }))).await;
    let address = v["address"].as_str().unwrap().to_string();
    assert_eq!(address, format!("ada@{}", north.trim_start_matches("ws://")), "the full tag names Ada's relay");

    // A bare tag only looks on Ben's own relay, where nobody has it.
    let (s, v) = call(&rb, "POST", "/api/tag/connect", Some(json!({ "tag": "@ada" }))).await;
    assert_ne!(s, StatusCode::OK, "{v}");
    assert!(v["error"].as_str().unwrap().contains("your relay"), "{v}");

    // The full tag reaches her across relays.
    let (s, v) = call(&rb, "POST", "/api/tag/connect", Some(json!({ "tag": format!("@{address}") }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ada = v["contact_id"].as_str().unwrap().to_string();
    let contacts = call(&rb, "GET", "/api/contacts", None).await.1;
    assert_eq!(contacts[0]["nickname"], address.as_str(), "Ben's name for her is her full tag");
    call(&rb, "POST", &format!("/api/contacts/{ada}/messages"), Some(json!({ "text": "Hello from the south relay." }))).await;
    until("Ada gets the request and the message", || async {
        let c = call(&ra, "GET", "/api/contacts", None).await.1;
        let Some(ben) = c.as_array().unwrap().iter().find(|c| c["request_tag"] == "ada") else { return false };
        let id = ben["id"].as_str().unwrap().to_string();
        call(&ra, "GET", &format!("/api/contacts/{id}/messages"), None).await.1.as_array().unwrap().iter().any(|m| m["text"] == "Hello from the south relay.")
    })
    .await;

    // A tag on a relay that isn't a relay address at all is refused before anything is sent.
    let (s, _) = call(&rb, "POST", "/api/tag/connect", Some(json!({ "tag": "ada@" }))).await;
    assert_ne!(s, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn updates_install_only_signed_matching_packages() {
    use sotto_core::keys::{sign, signing_public};
    use sotto_core::release::signed_message;
    use std::sync::{Arc as A, Mutex as M};

    // A stand-in release server: manifest, signature and one package, all changeable.
    #[derive(Default)]
    struct Rel {
        manifest: Vec<u8>,
        sig: String,
        package: Vec<u8>,
    }
    let rel: A<M<Rel>> = A::new(M::new(Rel::default()));
    let seed = [42u8; 32];
    let publish = |rel: &A<M<Rel>>, version: &str, package: &[u8], served: &[u8], key: [u8; 32]| {
        let sha = sotto_core::hex(&sotto_core::kdf::sha256(package));
        let manifest = serde_json::to_vec(&json!({
            "app": "sotto", "version": version, "notes": "Faster group sync.",
            "files": [{ "name": "Sotto.AppImage", "kind": "appimage", "os": "linux", "arch": "x86_64", "size": package.len(), "sha256": sha }],
        }))
        .unwrap();
        let sig = sotto_core::hex(&sign(&key, &signed_message(&manifest)).0);
        *rel.lock().unwrap() = Rel { manifest, sig, package: served.to_vec() };
    };
    let r2 = rel.clone();
    let app = Router::new()
        .route("/latest/download/latest.json", axum::routing::get({ let r = r2.clone(); move || { let r = r.clone(); async move { r.lock().unwrap().manifest.clone() } } }))
        .route("/latest/download/latest.json.sig", axum::routing::get({ let r = r2.clone(); move || { let r = r.clone(); async move { r.lock().unwrap().sig.clone() } } }))
        .route("/download/{v}/{name}", axum::routing::get({ let r = r2.clone(); move || { let r = r.clone(); async move { r.lock().unwrap().package.clone() } } }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // SAFETY: only this test reads these variables.
    unsafe {
        std::env::set_var("SOTTO_UPDATE_BASE", format!("http://{addr}"));
        std::env::set_var("SOTTO_UPDATE_TEST_KEY", sotto_core::hex(&signing_public(&seed)));
        std::env::set_var("SOTTO_PACKAGE", "appimage");
    }

    let url = relay().await;
    let d = tempfile::tempdir().unwrap();
    let r = named_desk(d.path(), &url, "update-passcode", "Ada").await;
    let check = || async { call(&r, "POST", "/api/updates/check", None).await.1 };

    // Signed by someone else: ignored entirely.
    publish(&rel, "9.0.0", b"new build", b"new build", [7u8; 32]);
    let v = check().await;
    assert_eq!(v["state"], "error", "{v}");
    assert!(v["error"].as_str().unwrap().contains("signature"), "{v}");
    assert!(v["version"].is_null(), "nothing from an unsigned manifest is believed: {v}");

    // Signed, but the package served isn't the one the manifest names.
    publish(&rel, "9.0.0", b"new build", b"tampered build", seed);
    let v = check().await;
    assert_eq!(v["state"], "error", "{v}");
    assert!(v["error"].as_str().unwrap().contains("fingerprint"), "{v}");
    assert_eq!(call(&r, "GET", "/api/updates/staged", None).await.0, StatusCode::NOT_FOUND);

    // Not newer: nothing to do.
    publish(&rel, "0.0.1", b"old build", b"old build", seed);
    assert_eq!(check().await["state"], "current");

    // Signed and matching: downloaded, verified, staged for the shell to install.
    publish(&rel, "9.0.0", b"new build", b"new build", seed);
    let v = check().await;
    assert_eq!(v["state"], "ready", "{v}");
    assert_eq!(v["version"], "9.0.0");
    let (s, staged) = call(&r, "GET", "/api/updates/staged", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(std::fs::read(staged["path"].as_str().unwrap()).unwrap(), b"new build");
    assert!(sotto_core::release::verify_manifest(staged["manifest"].as_str().unwrap().as_bytes(), staged["sig"].as_str().unwrap(), &signing_public(&seed)));
}
