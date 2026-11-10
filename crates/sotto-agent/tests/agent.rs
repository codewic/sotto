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
    for _ in 0..200 {
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
    assert!(sources[0]["pseudonym"].as_str().unwrap().starts_with("Source "));
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
