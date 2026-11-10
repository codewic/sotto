//! The whole drop flow over HTTP: new source, submission, journalist fetch, reply, return with codename.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use sotto_core::seal::SealSecret;
use sotto_drop::proto::{self, SubmissionKind};
use sotto_drop::{App, DropConfig, JournalistKey};
use tower::ServiceExt;

struct Fixture {
    router: Router,
    newsroom: SealSecret,
    seed: [u8; 32],
    dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let newsroom = SealSecret::generate();
    let (seed, public) = sotto_core::keys::signing_keypair();
    let cfg = DropConfig {
        name: "Test Outlet".into(),
        instance_id: sotto_core::hex(&[7u8; 16]),
        newsroom_key: proto::encode_key(&newsroom.public()).unwrap(),
        journalists: vec![JournalistKey { name: "Ada".into(), sign_pk: sotto_core::hex(&public) }],
        data_dir: dir.path().to_path_buf(),
        listen: "127.0.0.1:0".into(),
        max_upload_mb: 5,
        argon_m_kib: 64,
        argon_t: 1,
        intro: String::new(),
        verify_hint: "outlet.example/contact".into(),
    };
    let app: Arc<App> = App::new(cfg).unwrap();
    Fixture { router: sotto_drop::router(app), newsroom, seed, dir }
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, body)
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> &'a str {
    let i = s.find(start).unwrap() + start.len();
    let j = s[i..].find(end).unwrap() + i;
    &s[i..j]
}

fn csrf_of(html: &str) -> String {
    between(html, r#"name="csrf" value=""#, "\"").to_string()
}

fn form(path: &str, cookie: &str, body: String) -> Request<Body> {
    Request::post(path)
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

fn multipart(cookie: &str, csrf: &str, message: &str, file: Option<(&str, &[u8])>) -> Request<Body> {
    let b = "XsottoBoundaryX";
    let mut body = Vec::new();
    body.extend(format!("--{b}\r\nContent-Disposition: form-data; name=\"csrf\"\r\n\r\n{csrf}\r\n").as_bytes());
    body.extend(format!("--{b}\r\nContent-Disposition: form-data; name=\"message\"\r\n\r\n{message}\r\n").as_bytes());
    if let Some((name, data)) = file {
        body.extend(
            format!("--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/pdf\r\n\r\n").as_bytes(),
        );
        body.extend(data);
        body.extend(b"\r\n");
    }
    body.extend(format!("--{b}--\r\n").as_bytes());
    Request::post("/submit")
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={b}"))
        .body(Body::from(body))
        .unwrap()
}

fn signed(seed: &[u8; 32], method: &str, path: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, proto::auth_header(seed, method, path, &body))
        .body(Body::from(body))
        .unwrap()
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[tokio::test]
async fn source_to_newsroom_and_back() {
    let f = fixture();

    // Landing page: secure headers, no scripts, no third-party requests.
    let (status, headers, body) = send(&f.router, Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers.get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap();
    assert!(csp.contains("default-src 'none'"));
    let html = String::from_utf8(body).unwrap();
    assert!(!html.contains("<script"));
    assert!(!html.contains("http://") && !html.contains("https://"));
    assert!(html.contains("outlet.example/contact"));

    // New source gets a codename.
    let (_, headers, body) = send(&f.router, Request::get("/new").body(Body::empty()).unwrap()).await;
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    let html = String::from_utf8(body).unwrap();
    let csrf = csrf_of(&html);
    let words: Vec<String> = html.split(r#"<span class="w">"#).skip(1).map(|p| p.split('<').next().unwrap().to_string()).collect();
    assert_eq!(words.len(), 8);
    let codename = words.join(" ");

    // Without the right CSRF token nothing happens.
    let (status, _, _) = send(&f.router, form("/new", &cookie, "csrf=wrong".into())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, headers, _) = send(&f.router, form("/new", &cookie, format!("csrf={csrf}"))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/inbox");

    // Empty inbox teaches what happens next.
    let (_, _, body) = send(&f.router, Request::get("/inbox").header(header::COOKIE, &cookie).body(Body::empty()).unwrap()).await;
    assert!(String::from_utf8(body).unwrap().contains("No replies yet"));

    // Submit a message and a file.
    let pdf = b"%PDF-1.7 secret contract".repeat(5000);
    let (status, headers, _) = send(&f.router, multipart(&cookie, &csrf, "The audit was changed.", Some(("../../contract.pdf", &pdf)))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/inbox?sent=1");
    let (status, headers, _) = send(&f.router, multipart(&cookie, &csrf, "  ", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/inbox?error=empty");

    // Nothing readable on disk.
    for entry in walk(f.dir.path()) {
        let data = std::fs::read(&entry).unwrap();
        assert!(!data.windows(15).any(|w| w == b"secret contract"), "plaintext leaked to {}", entry.display());
        assert!(!data.windows(9).any(|w| w == b"The audit"));
    }

    // Journalist API refuses unsigned and replayed requests.
    let (status, _, _) = send(&f.router, Request::get("/j/v1/sources").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let req = signed(&f.seed, "GET", "/j/v1/sources", vec![]);
    let replay = Request::get("/j/v1/sources").header(header::AUTHORIZATION, req.headers()[header::AUTHORIZATION].clone()).body(Body::empty()).unwrap();
    let (status, _, body) = send(&f.router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(send(&f.router, replay).await.0, StatusCode::UNAUTHORIZED);
    let sources: Vec<sotto_drop::SourceSummary> = serde_json::from_slice(&body).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].submissions, 2);
    let sid = sources[0].id.clone();

    // Fetch and open both submissions with the newsroom key.
    let (_, _, body) = send(&f.router, signed(&f.seed, "GET", &format!("/j/v1/sources/{sid}/submissions"), vec![])).await;
    let subs: Vec<sotto_drop::SubmissionSummary> = serde_json::from_slice(&body).unwrap();
    let mut saw_text = false;
    let mut saw_file = false;
    for s in subs {
        let (status, _, sealed) = send(&f.router, signed(&f.seed, "GET", &format!("/j/v1/sources/{sid}/submissions/{}", s.name), vec![])).await;
        assert_eq!(status, StatusCode::OK);
        let plain = sotto_core::seal::open(&f.newsroom, &sealed).unwrap();
        let (meta, content) = proto::parse_submission(&plain).unwrap();
        match meta.kind {
            SubmissionKind::Message => {
                assert_eq!(content, b"The audit was changed.");
                saw_text = true;
            }
            SubmissionKind::File { name, mime } => {
                assert_eq!(name, "contract.pdf");
                assert_eq!(mime, "application/pdf");
                assert_eq!(content, &pdf[..]);
                saw_file = true;
            }
        }
    }
    assert!(saw_text && saw_file);

    // Reply, sealed to the source's codename-derived key.
    let (_, _, key) = send(&f.router, signed(&f.seed, "GET", &format!("/j/v1/sources/{sid}/key"), vec![])).await;
    let source_pub: sotto_core::seal::SealPublic = postcard::from_bytes(&key).unwrap();
    let reply = proto::Reply { day: proto::today(), text: "Thank you. Install Sotto and use this invite: sotto://invite/abc".into() };
    let sealed = sotto_core::seal::seal(&source_pub, &postcard::to_stdvec(&reply).unwrap()).unwrap();
    let (status, _, _) = send(&f.router, signed(&f.seed, "POST", &format!("/j/v1/sources/{sid}/replies"), sealed)).await;
    assert_eq!(status, StatusCode::CREATED);

    // The source returns later in a fresh session with the codename and reads it.
    let (_, headers, body) = send(&f.router, Request::get("/login").body(Body::empty()).unwrap()).await;
    let cookie2 = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    let csrf2 = csrf_of(&String::from_utf8(body).unwrap());
    let (_, _, body) = send(&f.router, form("/login", &cookie2, format!("csrf={csrf2}&codename=abandon+ability+able+about+above+absent+absorb+abstract"))).await;
    assert!(String::from_utf8(body).unwrap().contains("doesn&#39;t match a conversation"));
    let messy = codename.to_uppercase().replace(' ', "%0A");
    let (status, headers, _) = send(&f.router, form("/login", &cookie2, format!("csrf={csrf2}&codename={messy}"))).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "codename login works with case and line breaks");
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/inbox");
    let (_, _, body) = send(&f.router, Request::get("/inbox").header(header::COOKIE, &cookie2).body(Body::empty()).unwrap()).await;
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("Install Sotto and use this invite"));

    // Source deletes the reply; logout forgets the session.
    let name = between(&html, r#"name="name" value=""#, "\"").to_string();
    let csrf2 = csrf_of(&html);
    send(&f.router, form("/replies/delete", &cookie2, format!("csrf={csrf2}&name={name}"))).await;
    let (_, _, body) = send(&f.router, Request::get("/inbox").header(header::COOKIE, &cookie2).body(Body::empty()).unwrap()).await;
    assert!(String::from_utf8(body).unwrap().contains("No replies yet"));
    send(&f.router, form("/logout", &cookie2, format!("csrf={csrf2}"))).await;
    let (status, headers, _) = send(&f.router, Request::get("/inbox").header(header::COOKIE, &cookie2).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/login");

    // Path traversal through the API is refused.
    let (status, _, _) = send(&f.router, signed(&f.seed, "GET", &format!("/j/v1/sources/{sid}/submissions/..%2Fkey.bin"), vec![])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
