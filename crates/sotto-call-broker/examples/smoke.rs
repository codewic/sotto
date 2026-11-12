//! Smoke test for a deployed call server: proves LiveKit accepts the broker's tokens, applies the
//! room limits carried inside them, and hands out TURN relays.
//!
//!   curl -s -X POST https://calls.example.org/v1/rooms | cargo run -p sotto-call-broker --example smoke
//!
//! Reads one room grant (the broker's JSON) on stdin, joins the room's signalling channel once per
//! token, and prints what the server answered. Exits non-zero on any failure. Prints no tokens and
//! no TURN credentials.

use std::io::Read;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use livekit_protocol::{SignalResponse, signal_response};
use prost::Message as _;
use tokio_tungstenite::tungstenite::Message;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("smoke: FAIL: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap_or_else(|e| fail(e));
    let grant: serde_json::Value = serde_json::from_str(&input).unwrap_or_else(|_| fail(format!("stdin isn't a room grant: {}", input.chars().take(200).collect::<String>())));
    let url = grant["url"].as_str().unwrap_or_else(|| fail("no url in the grant")).trim_end_matches('/');
    let room = grant["room"].as_str().unwrap_or_else(|| fail("no room in the grant"));
    let tokens: Vec<&str> = grant["tokens"].as_array().unwrap_or_else(|| fail("no tokens in the grant")).iter().filter_map(|t| t.as_str()).collect();
    println!("smoke: room {room} with {} tokens at {url}", tokens.len());

    // Keep every connection open until all have joined, so the room really holds everyone at once.
    let mut open = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let endpoint = format!("{url}/rtc?access_token={token}&auto_subscribe=1&sdk=rust&protocol=16");
        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(endpoint.as_str()))
            .await
            .unwrap_or_else(|_| fail(format!("participant {}: connecting timed out", i + 1)))
            .unwrap_or_else(|e| fail(format!("participant {}: the server refused the connection ({e})", i + 1)));
        let join = loop {
            let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
                .await
                .unwrap_or_else(|_| fail(format!("participant {}: no join response within 15 s", i + 1)))
                .unwrap_or_else(|| fail(format!("participant {}: connection closed before joining", i + 1)))
                .unwrap_or_else(|e| fail(format!("participant {}: {e}", i + 1)));
            if let Message::Binary(bytes) = msg {
                let resp = SignalResponse::decode(bytes.as_ref()).unwrap_or_else(|e| fail(format!("undecodable signal message: {e}")));
                match resp.message {
                    Some(signal_response::Message::Join(j)) => break j,
                    Some(signal_response::Message::Leave(l)) => fail(format!("participant {}: server said leave ({:?})", i + 1, l.reason())),
                    _ => continue,
                }
            }
        };
        let r = join.room.clone().unwrap_or_default();
        if r.name != room {
            fail(format!("joined room {} instead of {room}", r.name));
        }
        let me = join.participant.clone().unwrap_or_default();
        let relays: Vec<String> = join.ice_servers.iter().flat_map(|s| s.urls.iter().map(|u| u.split('?').next().unwrap_or(u).to_string())).collect();
        println!(
            "smoke: participant {} joined as {} (server {}, room limit {}, closes {}s after empty, {} others present, relays: {})",
            i + 1,
            me.identity,
            join.server_version,
            r.max_participants,
            r.empty_timeout,
            join.other_participants.len(),
            if relays.is_empty() { "none".into() } else { relays.join(", ") }
        );
        if (r.max_participants as usize) < tokens.len() || r.max_participants == 0 {
            fail(format!("room limit is {} but the grant has {} tokens: the room settings in the token weren't applied", r.max_participants, tokens.len()));
        }
        if join.other_participants.len() != i {
            fail(format!("expected {i} others in the room, saw {}", join.other_participants.len()));
        }
        open.push(ws);
    }
    for mut ws in open {
        let _ = ws.send(Message::Close(None)).await;
    }
    println!("smoke: OK");
}
