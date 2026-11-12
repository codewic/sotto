//! Runs **inside** the review sandbox (no network, empty /etc, read-only /usr,
//! private /tmp). Turns one untrusted file into raw pixels and plain text:
//!
//! ```text
//! /out/page-N.ppm   one P6 image per page at 150 DPI
//! /out/info.txt     "Key: value" metadata lines (pdfinfo or image markers)
//! /out/text.txt     extracted text, if any
//! /out/words.html   word boxes (poppler bbox format), if any
//! /out/status.txt   "ok <kind>" or "error <reason>"
//! ```
//!
//! With a third argument `photo`, an image becomes one upright, flattened, size-capped
//! `/out/photo.ppm` instead, and `info.txt` says only `Format: png` or `Format: jpeg` (the kind of
//! file the clean copy should be).
//!
//! Nothing here is trusted by the caller: the agent parses only the simple
//! formats above, with strict limits, outside the sandbox.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_PAGES: &str = "300";
const DPI: &str = "150";

#[derive(Debug, PartialEq)]
enum Kind {
    Pdf,
    Image,
    Office(&'static str),
    Text,
    Unknown,
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn detect(head: &[u8]) -> Kind {
    if head.starts_with(b"%PDF") || find(&head[..head.len().min(1024)], b"%PDF-") {
        return Kind::Pdf;
    }
    if image::guess_format(head).is_ok() {
        return Kind::Image;
    }
    if head.starts_with(b"PK\x03\x04") {
        for (marker, ext) in [
            (&b"application/vnd.oasis.opendocument.text"[..], "odt"),
            (b"application/vnd.oasis.opendocument.spreadsheet", "ods"),
            (b"application/vnd.oasis.opendocument.presentation", "odp"),
            (b"word/", "docx"),
            (b"xl/", "xlsx"),
            (b"ppt/", "pptx"),
        ] {
            if find(head, marker) {
                return Kind::Office(ext);
            }
        }
        return Kind::Unknown;
    }
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Kind::Office("doc");
    }
    if head.starts_with(b"{\\rtf") {
        return Kind::Office("rtf");
    }
    if !head.is_empty() && std::str::from_utf8(head).is_ok() && !head.contains(&0) {
        return Kind::Text;
    }
    Kind::Unknown
}

fn status(out: &Path, s: &str) {
    let _ = fs::write(out.join("status.txt"), s);
}

fn run(cmd: &mut Command) -> bool {
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn render_pdf(pdf: &Path, out: &Path) -> Result<(), String> {
    if let Ok(o) = Command::new("pdfinfo").arg(pdf).output() {
        let _ = fs::write(out.join("info.txt"), o.stdout);
    }
    let _ = run(Command::new("pdftotext").args(["-layout", "-l", MAX_PAGES]).arg(pdf).arg(out.join("text.txt")));
    let _ = run(Command::new("pdftotext").args(["-bbox-layout", "-l", MAX_PAGES]).arg(pdf).arg(out.join("words.html")));
    if !run(Command::new("pdftoppm").args(["-r", DPI, "-l", MAX_PAGES]).arg(pdf).arg(out.join("page"))) {
        return Err("the PDF could not be rendered".into());
    }
    Ok(())
}

fn image_markers(data: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let head = &data[..data.len().min(256 * 1024)];
    if find(head, b"Exif\0\0") || find(head, b"eXIf") {
        lines.push("EXIF: present (can include camera model, capture time and location)".to_string());
    }
    if find(head, b"http://ns.adobe.com/xap/1.0/") {
        lines.push("XMP: present (editing software and history)".to_string());
    }
    if find(head, b"Photoshop 3.0") {
        lines.push("IPTC: present (captions, author, copyright)".to_string());
    }
    for chunk in [&b"tEXt"[..], b"iTXt", b"zTXt"] {
        if find(head, chunk) {
            lines.push("Embedded text: present".to_string());
            break;
        }
    }
    lines
}

fn render_image(input: &Path, out: &Path) -> Result<(), String> {
    let data = fs::read(input).map_err(|_| "the file could not be read")?;
    let _ = fs::write(out.join("info.txt"), image_markers(&data).join("\n"));
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().map_err(|_| "unknown image format")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| "the image could not be decoded")?.to_rgb8();
    let (w, h) = img.dimensions();
    let mut f = fs::File::create(out.join("page-1.ppm")).map_err(|_| "cannot write output")?;
    write!(f, "P6\n{w} {h}\n255\n").map_err(|_| "cannot write output")?;
    f.write_all(img.as_raw()).map_err(|_| "cannot write output")?;
    Ok(())
}

/// Longest side of a photo's clean copy.
const PHOTO_MAX: u32 = 2560;

fn render_photo(input: &Path, out: &Path) -> Result<(), String> {
    let data = fs::read(input).map_err(|_| "the file could not be read")?;
    let format = image::guess_format(&data).map_err(|_| "this isn't a photo")?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| "the photo could not be decoded")?;
    // Phones store photos sideways and say how to turn them; turn them before the tag is dropped.
    let orientation = image::ImageDecoder::orientation(&mut decoder).unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|_| "the photo could not be decoded")?;
    img.apply_orientation(orientation);
    // Transparent parts become white.
    let rgba = img.to_rgba8();
    drop(img);
    let (w, h) = rgba.dimensions();
    let mut rgb = image::RgbImage::new(w, h);
    for (p, q) in rgba.pixels().zip(rgb.pixels_mut()) {
        let a = u32::from(p[3]);
        for c in 0..3 {
            q[c] = ((u32::from(p[c]) * a + 255 * (255 - a)) / 255) as u8;
        }
    }
    drop(rgba);
    let rgb = if w.max(h) > PHOTO_MAX {
        let scale = PHOTO_MAX as f32 / w.max(h) as f32;
        let (nw, nh) = (((w as f32 * scale).round() as u32).clamp(1, PHOTO_MAX), ((h as f32 * scale).round() as u32).clamp(1, PHOTO_MAX));
        image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        rgb
    };
    let (w, h) = rgb.dimensions();
    let mut f = fs::File::create(out.join("photo.ppm")).map_err(|_| "cannot write output")?;
    write!(f, "P6\n{w} {h}\n255\n").map_err(|_| "cannot write output")?;
    f.write_all(rgb.as_raw()).map_err(|_| "cannot write output")?;
    // Screenshots and drawings stay sharp as PNG; photographs become JPEG.
    let lossless = matches!(format, image::ImageFormat::Png | image::ImageFormat::Gif | image::ImageFormat::Bmp);
    fs::write(out.join("info.txt"), if lossless { "Format: png" } else { "Format: jpeg" }).map_err(|_| "cannot write output")?;
    Ok(())
}

fn convert_office(input: &Path, ext: &str) -> Result<PathBuf, String> {
    let work = Path::new("/tmp/conv");
    fs::create_dir_all(work).map_err(|_| "no scratch space")?;
    let named = work.join(format!("document.{ext}"));
    fs::copy(input, &named).map_err(|_| "cannot stage the document")?;
    let ok = run(Command::new("soffice")
        .args(["-env:UserInstallation=file:///tmp/lo", "--headless", "--norestore", "--nolockcheck", "--nodefault", "--convert-to", "pdf", "--outdir"])
        .arg(work)
        .arg(&named)
        .env("HOME", "/tmp"));
    let pdf = work.join("document.pdf");
    if ok && pdf.exists() { Ok(pdf) } else { Err("the document could not be converted".into()) }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: sotto-render INPUT OUTDIR");
        std::process::exit(2);
    };
    let (input, out) = (Path::new(input), Path::new(out));
    let head = {
        use std::io::Read;
        let mut b = vec![0u8; 64 * 1024];
        let n = fs::File::open(input).and_then(|mut f| f.read(&mut b)).unwrap_or(0);
        b.truncate(n);
        b
    };
    if args.get(2).map(String::as_str) == Some("photo") {
        let result = match detect(&head) {
            Kind::Image => render_photo(input, out),
            _ => Err("this isn't a photo".to_string()),
        };
        match result {
            Ok(()) => status(out, "ok photo"),
            Err(e) => {
                status(out, &format!("error {e}"));
                std::process::exit(3);
            }
        }
        return;
    }
    let result = match detect(&head) {
        Kind::Pdf => render_pdf(input, out).map(|_| "pdf"),
        Kind::Image => render_image(input, out).map(|_| "image"),
        Kind::Office(ext) => convert_office(input, ext).and_then(|pdf| render_pdf(&pdf, out)).map(|_| "office"),
        Kind::Text => convert_office(input, "txt").and_then(|pdf| render_pdf(&pdf, out)).map(|_| "text"),
        Kind::Unknown => Err("this file type isn't supported for safe review".into()),
    };
    match result {
        Ok(k) => status(out, &format!("ok {k}")),
        Err(e) => {
            status(out, &format!("error {e}"));
            std::process::exit(3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_types_by_content() {
        assert_eq!(detect(b"%PDF-1.7"), Kind::Pdf);
        assert_eq!(detect(b"\x89PNG\r\n\x1a\n...."), Kind::Image);
        assert_eq!(detect(b"PK\x03\x04....word/document.xml"), Kind::Office("docx"));
        assert_eq!(detect(b"{\\rtf1 hello}"), Kind::Office("rtf"));
        assert_eq!(detect(b"plain notes"), Kind::Text);
        assert_eq!(detect(&[0, 1, 2, 3]), Kind::Unknown);
    }
}
