//! Real files through the real sandbox.

use std::path::PathBuf;

use sotto_review::{Options, review};

fn opts() -> Options {
    Options { render: PathBuf::from(env!("CARGO_BIN_EXE_sotto-render")), ..Options::default() }
}

/// A one-page PDF with author metadata and a black box drawn over "John Smith" (a fake redaction).
fn fake_redaction_pdf() -> Vec<u8> {
    let content = "BT /F1 24 Tf 72 700 Td (Signed by) Tj ET BT /F1 24 Tf 300 700 Td (John Smith) Tj ET 0 0 0 rg 290 688 170 36 re f";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Author (J. Smith) /Title (Q3 audit) /Creator (Word) >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// A white PNG with a grid of pale yellow specks (like printer tracking dots) and a text chunk.
fn dotted_png() -> Vec<u8> {
    let (w, h) = (600usize, 600usize);
    let mut px = vec![255u8; w * h * 3];
    for gy in 0..6 {
        for gx in 0..6 {
            let (x, y) = (40 + gx * 90, 40 + gy * 90);
            let i = (y * w + x) * 3;
            px[i..i + 3].copy_from_slice(&[255, 244, 140]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.add_text_chunk("Author".into(), "Scanner at HQ floor 3".into()).unwrap();
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&px).unwrap();
    }
    out
}

fn decode_png_as_ppm(path: &std::path::Path) -> Vec<u8> {
    let png = std::fs::read(path).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let mut ppm = format!("P6\n{} {}\n255\n", info.width, info.height).into_bytes();
    ppm.extend_from_slice(&buf[..info.buffer_size()]);
    ppm
}

#[test]
fn pdf_with_fake_redaction_and_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.bin");
    std::fs::write(&input, fake_redaction_pdf()).unwrap();
    let work = dir.path().join("work");
    let r = review(&input, &work, &opts()).unwrap();
    assert_eq!(r.kind, "pdf");
    assert_eq!(r.pages, 1);
    let keys: Vec<&str> = r.metadata.iter().map(|m| m.key.as_str()).collect();
    assert!(keys.contains(&"Author") && keys.contains(&"Title"), "metadata found: {keys:?}");
    assert_eq!(r.hidden_text.len(), 1, "fake redaction detected");
    assert!(r.hidden_text[0].sample.iter().any(|w| w == "John" || w == "Smith"));
    assert!(!r.hidden_text[0].sample.iter().any(|w| w == "Signed"), "visible words are not flagged");
    // The clean copy is pixels only: no text layer, no metadata.
    let clean = std::fs::read(&r.clean_pdf).unwrap();
    assert!(clean.starts_with(b"%PDF"));
    let s = String::from_utf8_lossy(&clean);
    assert!(!s.contains("Smith") && !s.contains("/Author") && !s.contains("/Font"));
    assert!(std::fs::read(&r.previews[0]).unwrap().starts_with(b"\x89PNG"));
    assert!(!work.join("raw").exists(), "raw sandbox output is cleaned up");
}

#[test]
fn image_with_dots_and_text_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.bin");
    std::fs::write(&input, dotted_png()).unwrap();
    let r = review(&input, &dir.path().join("work"), &opts()).unwrap();
    assert_eq!(r.kind, "image");
    assert_eq!(r.printer_dots.len(), 1);
    assert!(r.printer_dots[0].specks >= 30);
    assert!(r.metadata.iter().any(|m| m.key == "Embedded text"));
    // Yellow specks are removed from the clean copy by default.
    let clean = sotto_review::raster::parse_ppm(&decode_png_as_ppm(&r.previews[0])).unwrap();
    assert_eq!(sotto_review::raster::yellow_specks(&clean).0, 0);
}

#[test]
fn office_document_through_libreoffice() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.bin");
    std::fs::write(&input, br"{\rtf1\ansi\deff0 {\fonttbl {\f0 Helvetica;}}\f0\fs28 Notes from the source about the contract.\par}").unwrap();
    let r = review(&input, &dir.path().join("work"), &opts()).unwrap();
    assert_eq!(r.kind, "office");
    assert!(r.pages >= 1);
    assert!(r.text.contains("Notes from the source"));
}

#[test]
fn unsupported_and_hostile_inputs_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.bin");
    std::fs::write(&input, [0u8, 1, 2, 3, 255, 254]).unwrap();
    let e = review(&input, &dir.path().join("w1"), &opts()).unwrap_err();
    assert!(e.to_string().contains("isn't supported"));
    // A PDF that is garbage after the header renders nothing and is refused.
    std::fs::write(&input, b"%PDF-1.7\n this is not really a pdf").unwrap();
    assert!(review(&input, &dir.path().join("w2"), &opts()).is_err());
}

#[test]
fn sandbox_has_no_network_or_home() {
    let argv = sotto_review::sandbox::command(std::path::Path::new("/r"), std::path::Path::new("/i"), std::path::Path::new("/o"), std::path::Path::new("/o.id"), None);
    let joined = argv.join(" ");
    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--clearenv"));
    assert!(!joined.contains("/home"));
    assert!(!joined.contains("--share-net"));
}

/// A 40 x 20 JPEG (left half red, right half blue) that says "turn me 90 degrees clockwise" in
/// its EXIF, and carries a comment with a place name, the way cameras and phones leave details.
fn sideways_jpeg_with_details() -> Vec<u8> {
    let (w, h) = (40u32, 20u32);
    let mut px = Vec::new();
    for _y in 0..h {
        for x in 0..w {
            px.extend(if x < w / 2 { [220, 30, 30] } else { [30, 30, 220] });
        }
    }
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode(&px, w, h, image::ExtendedColorType::Rgb8).unwrap();
    // EXIF: little-endian TIFF with one entry, Orientation (0x0112) = 6.
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    tiff.extend([1, 0]);
    tiff.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
    tiff.extend([0, 0, 0, 0]);
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(tiff);
    let comment = b"Taken at Apapa port, 6.4474 N 3.3640 E";
    let mut out = vec![0xFF, 0xD8];
    out.extend([0xFF, 0xE1]);
    out.extend(((exif.len() + 2) as u16).to_be_bytes());
    out.extend(exif);
    out.extend([0xFF, 0xFE]);
    out.extend(((comment.len() + 2) as u16).to_be_bytes());
    out.extend(comment);
    out.extend(&jpeg[2..]);
    out
}

#[test]
fn photos_come_out_upright_and_without_details() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.jpg");
    std::fs::write(&input, sideways_jpeg_with_details()).unwrap();
    let render = PathBuf::from(env!("CARGO_BIN_EXE_sotto-render"));
    let p = sotto_review::photo(&input, &dir.path().join("work"), &render, std::time::Duration::from_secs(60)).expect("photo");
    // Turned upright: 20 wide, 40 tall, red on top after a clockwise turn.
    assert_eq!((p.width, p.height, p.mime), (20, 40, "image/jpeg"));
    let hay = |n: &[u8]| p.bytes.windows(n.len()).any(|w| w == n);
    assert!(!hay(b"Exif") && !hay(b"Apapa") && !hay(&[0xFF, 0xFE]), "details survived the clean copy");
    let img = image::load_from_memory(&p.bytes).unwrap().to_rgb8();
    let (top, bottom) = (img.get_pixel(10, 5), img.get_pixel(10, 35));
    assert!(top[0] > 150 && top[2] < 100, "top should be red, got {top:?}");
    assert!(bottom[2] > 150 && bottom[0] < 100, "bottom should be blue, got {bottom:?}");
}

#[test]
fn screenshots_stay_png_and_huge_photos_shrink() {
    let dir = tempfile::tempdir().unwrap();
    let render = PathBuf::from(env!("CARGO_BIN_EXE_sotto-render"));
    // A wide PNG with transparency: kept as PNG, longest side capped, transparent parts white.
    let (w, h) = (3000u32, 1000u32);
    let img = image::RgbaImage::from_fn(w, h, |x, _| if x < 1500 { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([10, 120, 60, 255]) });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let input = dir.path().join("shot.png");
    std::fs::write(&input, png).unwrap();
    let p = sotto_review::photo(&input, &dir.path().join("work"), &render, std::time::Duration::from_secs(60)).expect("photo");
    assert_eq!(p.mime, "image/png");
    assert_eq!(p.width.max(p.height), sotto_review::PHOTO_MAX);
    let out = image::load_from_memory(&p.bytes).unwrap().to_rgb8();
    assert_eq!(out.get_pixel(10, 10).0, [255, 255, 255]);
    // Not a photo at all: refused.
    std::fs::write(&input, b"%PDF-1.4 not a photo").unwrap();
    assert!(sotto_review::photo(&input, &dir.path().join("work2"), &render, std::time::Duration::from_secs(60)).is_err());
}
