//! Server-rendered pages. No JavaScript, no external requests, one stylesheet.

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// "12 Nov 2026" from days since the Unix epoch (civil-from-days, Howard Hinnant).
pub fn date(day: u32) -> String {
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

pub fn layout(newsroom: &str, title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>{title} · {newsroom}</title>
<link rel="stylesheet" href="/assets/drop.css">
</head>
<body>
<header class="top"><span class="mark" aria-hidden="true"></span><span>{newsroom}</span><span class="sep">·</span><span class="muted">Secure contact</span></header>
<main>
{body}
</main>
<footer class="foot"><p>This page runs on a Tor onion service. It stores no record of your address, browser or the times you visit.</p></footer>
</body>
</html>"#,
        title = esc(title),
        newsroom = esc(newsroom),
        body = body
    )
}

pub fn landing(newsroom: &str, intro: &str, verify_hint: &str) -> String {
    let intro = if intro.trim().is_empty() {
        "Share information with our reporters without revealing who you are. Nothing you send here is linked to your name, email address or phone number.".to_string()
    } else {
        esc(intro)
    };
    let verify = if verify_hint.trim().is_empty() {
        String::new()
    } else {
        format!(r#"<p class="hint">Only trust this page if you found its address on {}.</p>"#, esc(verify_hint))
    };
    let body = format!(
        r#"<section class="hero">
<h1>Contact {n} securely</h1>
<p class="lead">{intro}</p>
<div class="actions">
<a class="btn primary" href="/new">Start a new conversation</a>
<a class="btn" href="/login">Return with your codename</a>
</div>
{verify}
</section>
<section class="card">
<h2>Before you write</h2>
<ol class="steps">
<li><strong>Use a personal device on a network that isn't your employer's.</strong> Work computers and networks are often monitored.</li>
<li><strong>Keep this to yourself.</strong> Don't search for us from work, and don't tell anyone you're getting in touch.</li>
<li><strong>Stay in Tor Browser.</strong> For the strongest protection, set its security level to Safest. This page works with JavaScript turned off.</li>
<li><strong>Files can carry hidden details,</strong> such as author names or tiny printer marks. Send them anyway: we clean every file before anyone reads it. Tell us if only a few people could have had a copy.</li>
<li><strong>You'll get a codename, not an account.</strong> Write it on paper. It's the only way back to your conversation.</li>
</ol>
</section>
<details class="card limits">
<summary>What this can't protect you from</summary>
<p>Monitoring software on the device you use, people watching you, or details in the information itself that only a few people know. If you think your device is monitored, use a different one.</p>
</details>"#,
        n = esc(newsroom),
    );
    layout(newsroom, "Contact us securely", &body)
}

pub fn new_codename(newsroom: &str, codename: &str, csrf: &str) -> String {
    let words: String = codename
        .split(' ')
        .enumerate()
        .map(|(i, w)| format!(r#"<li><span class="n">{}</span><span class="w">{}</span></li>"#, i + 1, esc(w)))
        .collect();
    let body = format!(
        r#"<section class="card">
<h1>Your codename</h1>
<p class="lead">Write these eight words on paper, in order. Don't save them on this device or take a screenshot.</p>
<ol class="codename" aria-label="Codename">{words}</ol>
<p class="warn"><span class="dot" aria-hidden="true"></span>Without these words you can't read our replies, and we can't recover them for you.</p>
<form method="post" action="/new">
<input type="hidden" name="csrf" value="{csrf}">
<button class="btn primary" type="submit">I've written it down</button>
</form>
</section>"#,
        csrf = esc(csrf)
    );
    layout(newsroom, "Your codename", &body)
}

pub fn login(newsroom: &str, csrf: &str, error: Option<&str>) -> String {
    let err = error.map(|e| format!(r#"<p class="error" role="alert"><span class="dot" aria-hidden="true"></span>{}</p>"#, esc(e))).unwrap_or_default();
    let body = format!(
        r#"<section class="card narrow">
<h1>Return to your conversation</h1>
<form method="post" action="/login">
<input type="hidden" name="csrf" value="{csrf}">
<label for="codename">Codename</label>
<textarea id="codename" name="codename" rows="3" autocomplete="off" autocapitalize="off" spellcheck="false" required></textarea>
<p class="hint">The eight words you wrote down, in order.</p>
{err}
<button class="btn primary" type="submit">Continue</button>
</form>
<p class="hint"><a href="/new">I don't have a codename</a></p>
</section>"#,
        csrf = esc(csrf)
    );
    layout(newsroom, "Return with your codename", &body)
}

pub struct ReplyItem {
    pub name: String,
    pub day: u32,
    pub text: String,
}

pub fn inbox(newsroom: &str, csrf: &str, replies: &[ReplyItem], sent: bool, max_mb: u64, error: Option<&str>) -> String {
    let banner = if sent {
        r#"<p class="ok" role="status"><span class="dot" aria-hidden="true"></span>Sent. It was encrypted as it arrived, so only the newsroom can read it.</p>"#.to_string()
    } else {
        String::new()
    };
    let err = error.map(|e| format!(r#"<p class="error" role="alert"><span class="dot" aria-hidden="true"></span>{}</p>"#, esc(e))).unwrap_or_default();
    let list = if replies.is_empty() {
        r#"<div class="empty"><span class="spot" aria-hidden="true"></span><p><strong>No replies yet.</strong> Reporters usually answer within a few days. Come back with your codename to check.</p></div>"#.to_string()
    } else {
        replies
            .iter()
            .map(|r| {
                format!(
                    r#"<li class="reply"><div class="meta">Reply from the newsroom · {date}</div><div class="text">{text}</div>
<form method="post" action="/replies/delete"><input type="hidden" name="csrf" value="{csrf}"><input type="hidden" name="name" value="{name}"><button class="btn quiet" type="submit">Delete reply</button></form></li>"#,
                    date = date(r.day),
                    text = esc(&r.text),
                    csrf = esc(csrf),
                    name = esc(&r.name),
                )
            })
            .collect::<String>()
    };
    let body = format!(
        r#"{banner}{err}
<section class="card">
<h1>Your conversation</h1>
<ul class="replies">{list}</ul>
</section>
<section class="card">
<h2>Send a message or file</h2>
<form method="post" action="/submit" enctype="multipart/form-data">
<input type="hidden" name="csrf" value="{csrf}">
<label for="message">Message</label>
<textarea id="message" name="message" rows="6" spellcheck="true"></textarea>
<label for="file">Attach a file <span class="muted">(optional)</span></label>
<input id="file" name="file" type="file">
<p class="hint">Up to {max_mb} MB. Encrypted the moment it reaches us.</p>
<button class="btn primary" type="submit">Send securely</button>
</form>
</section>
<form class="leave" method="post" action="/logout"><input type="hidden" name="csrf" value="{csrf}"><button class="btn" type="submit">Leave and forget this session</button></form>"#,
        csrf = esc(csrf),
    );
    layout(newsroom, "Your conversation", &body)
}

pub fn message(newsroom: &str, title: &str, text: &str) -> String {
    let body = format!(r#"<section class="card narrow"><h1>{}</h1><p>{}</p><p><a class="btn" href="/">Back to the start</a></p></section>"#, esc(title), esc(text));
    layout(newsroom, title, &body)
}

pub const CSS: &str = include_str!("../assets/drop.css");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_dates() {
        assert_eq!(esc(r#"<a href="x">'&"#), "&lt;a href=&quot;x&quot;&gt;&#39;&amp;");
        assert_eq!(date(0), "1 Jan 1970");
        assert_eq!(date(20_404), "12 Nov 2025");
        let page = inbox("N", "c", &[ReplyItem { name: "r".into(), day: 0, text: "<script>".into() }], true, 10, None);
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("<script>"));
        assert!(!page.contains('\u{2014}'), "no em dashes in copy");
    }
}
