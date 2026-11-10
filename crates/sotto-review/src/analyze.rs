//! Publish check: metadata, hidden text under fake redactions, and version fingerprints.

use serde::Serialize;

/// pdfinfo fields worth reporting. Page geometry and similar are skipped.
const META_KEYS: &[&str] = &[
    "Title", "Subject", "Keywords", "Author", "Creator", "Producer", "CreationDate", "ModDate", "Custom Metadata", "Metadata Stream",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetaField {
    pub key: String,
    pub value: String,
}

/// Parse `Key: value` lines from the sandbox, keeping only fields that can identify someone.
pub fn metadata(info: &str) -> Vec<MetaField> {
    let mut out = Vec::new();
    for line in info.lines().take(200) {
        let Some((k, v)) = line.split_once(':') else { continue };
        let k = k.trim();
        let v: String = v.trim().chars().filter(|c| !c.is_control()).take(200).collect();
        let interesting = META_KEYS.contains(&k) || k == "EXIF" || k == "XMP" || k == "IPTC" || k == "Embedded text";
        if interesting && !v.is_empty() && v != "no" {
            out.push(MetaField { key: k.to_string(), value: v });
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub page: usize,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub text: String,
}

fn attr(tag: &str, name: &str) -> Option<f32> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')? + i;
    tag[i..j].parse().ok()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

/// Parse poppler's `-bbox-layout` output. Coordinates are PDF points, origin top-left.
/// Returns words and page sizes.
pub fn words(html: &str) -> (Vec<Word>, Vec<(f32, f32)>) {
    let mut out = Vec::new();
    let mut pages = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..=end];
        if tag.starts_with("<page ") {
            pages.push((attr(tag, "width").unwrap_or(612.0), attr(tag, "height").unwrap_or(792.0)));
        } else if tag.starts_with("<word ") && !pages.is_empty() {
            let after = &rest[end + 1..];
            let text = after.split('<').next().unwrap_or("");
            if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (attr(tag, "xMin"), attr(tag, "yMin"), attr(tag, "xMax"), attr(tag, "yMax")) {
                out.push(Word { page: pages.len() - 1, x0, y0, x1, y1, text: unescape(text).chars().take(100).collect() });
            }
            if out.len() > 2_000_000 {
                break;
            }
        }
        rest = &rest[end + 1..];
    }
    (out, pages)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HiddenText {
    pub page: usize,
    pub count: usize,
    /// A few of the hidden words, so the journalist sees what would leak.
    pub sample: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Passage {
    /// `removed` (only in the first version), `added` (only in the second).
    pub kind: &'static str,
    pub text: String,
}

/// Word-level differences between two versions of a document's text. Differences are where a
/// uniquely marked copy would give its recipient away: paraphrase these passages, don't quote them.
pub fn compare_texts(a: &str, b: &str) -> Vec<Passage> {
    let diff = similar::TextDiff::configure().algorithm(similar::Algorithm::Patience).diff_words(a, b);
    let mut out: Vec<Passage> = Vec::new();
    // Changes only merge into one passage when nothing but whitespace separates them.
    let mut joinable = false;
    for change in diff.iter_all_changes() {
        let v = change.value();
        let kind = match change.tag() {
            similar::ChangeTag::Delete => "removed",
            similar::ChangeTag::Insert => "added",
            similar::ChangeTag::Equal => {
                if !v.trim().is_empty() {
                    joinable = false;
                }
                continue;
            }
        };
        match out.last_mut() {
            Some(last) if joinable && last.kind == kind => {
                last.text.push(' ');
                last.text.push_str(v);
            }
            _ => out.push(Passage { kind, text: v.to_string() }),
        }
        joinable = true;
    }
    for p in &mut out {
        p.text = p.text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    out.retain(|p| !p.text.is_empty());
    out.truncate(500);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_keeps_identifying_fields() {
        let info = "Title:          Q3 audit\nAuthor:         J. Smith\nPages:          3\nPage size:      612 x 792 pts\nCreator:        Word\nTagged:         no\n";
        let m = metadata(info);
        assert_eq!(m.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), vec!["Title", "Author", "Creator"]);
        assert_eq!(m[1].value, "J. Smith");
    }

    #[test]
    fn parses_word_boxes() {
        let html = r#"<doc><page width="612.000000" height="792.000000"><flow><block><line>
<word xMin="72.0" yMin="80.5" xMax="110.2" yMax="104.0">John</word>
<word xMin="115.0" yMin="80.5" xMax="160.0" yMax="104.0">&amp;Smith</word></line></block></flow></page>
<page width="300" height="400"><word xMin="1" yMin="2" xMax="3" yMax="4">x</word></page></doc>"#;
        let (w, pages) = words(html);
        assert_eq!(pages, vec![(612.0, 792.0), (300.0, 400.0)]);
        assert_eq!(w.len(), 3);
        assert_eq!(w[1].text, "&Smith");
        assert_eq!(w[2].page, 1);
    }

    #[test]
    fn compares_versions() {
        let a = "The meeting on Tuesday approved the transfer of funds.";
        let b = "The meeting on Wednesday approved the transfer of all funds.";
        let p = compare_texts(a, b);
        assert!(p.contains(&Passage { kind: "removed", text: "Tuesday".into() }));
        assert!(p.contains(&Passage { kind: "added", text: "Wednesday".into() }));
        assert!(p.iter().any(|x| x.kind == "added" && x.text == "all"));
        assert!(compare_texts(a, a).is_empty());
    }
}
