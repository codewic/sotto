//! `sotto-hook`: post into a Sotto group or channel from a script.
//!
//! ```text
//! export SOTTO_HOOK_URL=unix:/home/you/.config/Sotto/agent.sock/hooks/<id>
//! export SOTTO_HOOK_TOKEN=sotto_hook_...
//! sotto-hook "Deploy 412 finished"
//! make test 2>&1 | tail -20 | sotto-hook
//! ```
//!
//! It talks only to the Sotto app on this computer, over its private socket (only your user can
//! open it) or, in development, plain HTTP to 127.0.0.1. The app encrypts the message and sends it
//! like any other. No dependencies, so it's easy to audit and copy around.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "Usage: sotto-hook [--url URL] [--token TOKEN] [MESSAGE...]

Posts MESSAGE (or, without one, everything on standard input) to the group or channel
an integration in Sotto points at. Make an integration in Sotto: Settings, Integrations.

  --url URL       The integration's address (or set SOTTO_HOOK_URL)
  --token TOKEN   The integration's token (or set SOTTO_HOOK_TOKEN; safer than the flag,
                  which other users on this computer can see in the process list)
  --code LANG     Send the message as a code block, for example --code sh
  -h, --help      Show this help
";

struct Args {
    url: Option<String>,
    token: Option<String>,
    code: Option<String>,
    message: Vec<String>,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args { url: None, token: None, code: None, message: vec![] };
    while let Some(x) = it.next() {
        match x.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--url" => a.url = Some(it.next().ok_or("--url needs a value")?),
            "--token" => a.token = Some(it.next().ok_or("--token needs a value")?),
            "--code" => a.code = Some(it.next().ok_or("--code needs a language, for example sh")?),
            "--" => a.message.extend(it.by_ref()),
            _ => a.message.push(x),
        }
    }
    Ok(a)
}

enum Endpoint {
    Tcp { host: String, port: u16, path: String },
    Unix { socket: String, path: String },
}

/// `unix:/path/agent.sock/hooks/<id>` or `http://127.0.0.1:PORT/hooks/<id>`.
fn endpoint(url: &str) -> Result<Endpoint, String> {
    if let Some(rest) = url.strip_prefix("unix:") {
        let i = rest.rfind("/hooks/").ok_or("The URL should end in /hooks/<id>. Copy it again from Sotto.")?;
        return Ok(Endpoint::Unix { socket: rest[..i].to_string(), path: rest[i..].to_string() });
    }
    let (host, port, path) = split_url(url)?;
    Ok(Endpoint::Tcp { host, port, path })
}

/// `http://host:port/path` → (host, port, path). Only plain HTTP to this computer.
fn split_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("http://").ok_or("The URL should start with unix: or http:// (the Sotto app on this computer).")?;
    let (hostport, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || hostport.starts_with('[') => (h.to_string(), p.parse::<u16>().map_err(|_| "The port in the URL isn't a number.")?),
        _ => (hostport.to_string(), 80),
    };
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
        return Err("sotto-hook only talks to the Sotto app on this computer (127.0.0.1).".into());
    }
    Ok((host, port, path))
}

/// The server's error message, if the body is `{"error": "..."}`.
fn error_of(body: &str) -> Option<String> {
    let i = body.find("\"error\"")?;
    let rest = &body[i + 7..];
    let start = rest.find('"')? + 1;
    let mut out = String::new();
    let mut chars = rest[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next().unwrap_or('\\')),
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn post(url: &str, token: &str, text: &str) -> Result<(), String> {
    let (mut s, host, path): (Box<dyn Stream>, String, String) = match endpoint(url)? {
        Endpoint::Tcp { host, port, path } => {
            let addr = format!("{}:{port}", host.trim_start_matches('[').trim_end_matches(']'));
            let s = TcpStream::connect(&addr).map_err(|_| format!("Couldn't reach Sotto at {addr}. Is the app running?"))?;
            s.set_read_timeout(Some(Duration::from_secs(30))).ok();
            (Box::new(s), format!("{host}:{port}"), path)
        }
        #[cfg(unix)]
        Endpoint::Unix { socket, path } => {
            let s = std::os::unix::net::UnixStream::connect(&socket).map_err(|_| format!("Couldn't reach Sotto at {socket}. Is the app running?"))?;
            s.set_read_timeout(Some(Duration::from_secs(30))).ok();
            (Box::new(s), "localhost".into(), path)
        }
        #[cfg(not(unix))]
        Endpoint::Unix { .. } => return Err("unix: addresses work only on Linux and macOS.".into()),
    };
    let body = text.as_bytes();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(body)).map_err(|e| format!("Sending failed: {e}"))?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).map_err(|e| format!("Reading the reply failed: {e}"))?;
    let status: u16 = resp.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    if status == 200 {
        Ok(())
    } else {
        Err(error_of(body).unwrap_or_else(|| format!("Sotto answered with status {status}.")))
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) if e.is_empty() => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("sotto-hook: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (Some(url), Some(token)) = (args.url.or_else(|| std::env::var("SOTTO_HOOK_URL").ok()), args.token.or_else(|| std::env::var("SOTTO_HOOK_TOKEN").ok())) else {
        eprintln!("sotto-hook: set SOTTO_HOOK_URL and SOTTO_HOOK_TOKEN (from Sotto: Settings, Integrations).");
        return ExitCode::from(2);
    };
    let mut text = args.message.join(" ");
    if text.is_empty() {
        if std::io::stdin().read_to_string(&mut text).is_err() {
            eprintln!("sotto-hook: couldn't read standard input.");
            return ExitCode::from(2);
        }
    }
    let text = text.trim_end();
    if text.is_empty() {
        eprintln!("sotto-hook: nothing to send.");
        return ExitCode::from(2);
    }
    let text = match &args.code {
        Some(lang) => format!("```{lang}\n{text}\n```"),
        None => text.to_string(),
    };
    match post(&url, &token, &text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sotto-hook: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(split_url("http://127.0.0.1:7801/hooks/ab").unwrap(), ("127.0.0.1".into(), 7801, "/hooks/ab".into()));
        assert_eq!(split_url("http://localhost/hooks/ab").unwrap(), ("localhost".into(), 80, "/hooks/ab".into()));
        assert!(split_url("https://127.0.0.1/hooks/ab").is_err());
        assert!(split_url("http://example.com:7801/hooks/ab").is_err());
        match endpoint("unix:/run/user/1000/sotto/agent.sock/hooks/ab").unwrap() {
            Endpoint::Unix { socket, path } => assert_eq!((socket.as_str(), path.as_str()), ("/run/user/1000/sotto/agent.sock", "/hooks/ab")),
            Endpoint::Tcp { .. } => panic!("expected a socket"),
        }
    }

    #[test]
    fn errors_and_args() {
        assert_eq!(error_of(r#"{"error":"Not \"allowed\"."}"#).as_deref(), Some("Not \"allowed\"."));
        let a = parse(["--code", "sh", "make", "test"].into_iter().map(String::from)).unwrap();
        assert_eq!((a.code.as_deref(), a.message.join(" ")), (Some("sh"), "make test".to_string()));
        assert!(parse(["--help"].into_iter().map(String::from)).is_err());
    }
}
