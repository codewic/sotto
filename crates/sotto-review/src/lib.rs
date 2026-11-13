//! Safe review and publish check.
//!
//! 1. The untrusted file is rendered to raw pixels by `sotto-render` inside a
//!    bubblewrap sandbox with no network (see [`sandbox`]).
//! 2. Outside the sandbox, only strictly parsed raw pixels and plain text are
//!    used: a clean PDF is rebuilt from pixels (so no scripts, fonts, links or
//!    metadata survive), and the publish checks run.

#![forbid(unsafe_code)]

pub mod analyze;
pub mod media;
pub mod pdf;
pub mod raster;
pub mod sandbox;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

pub use analyze::{HiddenText, MetaField, Passage, compare_texts};

pub const DPI: f32 = 150.0;
const MAX_TEXT: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReviewError {
    #[error("Safe review needs bubblewrap, which isn't available on this computer.")]
    NoSandbox,
    #[error("The file took too long to open safely and was stopped.")]
    Timeout,
    #[error("{0}")]
    Render(String),
    #[error("Couldn't write the clean copy.")]
    Io,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub render: PathBuf,
    pub timeout: Duration,
    /// Rebuild the clean copy in greyscale (removes colour tracking dots completely).
    pub greyscale: bool,
    /// Turn print-yellow specks white in the clean copy.
    pub strip_yellow: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { render: sandbox::default_render_path(), timeout: Duration::from_secs(180), greyscale: false, strip_yellow: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DotsFinding {
    pub page: usize,
    pub specks: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    /// `pdf`, `image`, `office` or `text`.
    pub kind: String,
    pub pages: usize,
    pub metadata: Vec<MetaField>,
    pub printer_dots: Vec<DotsFinding>,
    pub hidden_text: Vec<HiddenText>,
    pub has_text: bool,
    #[serde(skip)]
    pub text: String,
    #[serde(skip)]
    pub clean_pdf: PathBuf,
    #[serde(skip)]
    pub previews: Vec<PathBuf>,
}

/// Pages that look like they carry tracking dots: many isolated yellow specks, little yellow otherwise.
fn dots_verdict(specks: u32, fraction: f32) -> bool {
    specks >= 12 && fraction < 0.02
}

fn page_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<(u32, PathBuf)> = fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let n = name.strip_prefix("page-")?.strip_suffix(".ppm")?.parse().ok()?;
                    Some((n, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.into_iter().map(|(_, p)| p).collect()
}

/// Read a file the sandbox produced: regular files only (never a symlink or FIFO a compromised
/// renderer might plant), and never more than `cap` bytes.
fn read_untrusted(path: &Path, cap: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let mut out = Vec::new();
    fs::File::open(path).ok()?.take(cap as u64).read_to_end(&mut out).ok()?;
    Some(out)
}

const MAX_PAGES: usize = 300;
const MAX_PAGE_BYTES: usize = raster::MAX_DIM * raster::MAX_DIM * 3 + 64;

fn read_capped(path: &Path, cap: usize) -> String {
    let data = read_untrusted(path, cap).unwrap_or_default();
    String::from_utf8_lossy(&data).chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// Longest side of a photo's clean copy (the renderer enforces it; this checks it).
pub const PHOTO_MAX: usize = 2560;

/// A photo rebuilt from pixels: no metadata (location, camera, times) can survive.
#[derive(Debug)]
pub struct Photo {
    pub width: usize,
    pub height: usize,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Clean a photo: it's decoded only inside the sandbox, which hands back raw pixels, and the
/// clean copy is encoded here from those pixels alone. Used for photos you send and photos you
/// receive alike.
pub fn photo(input: &Path, work: &Path, render: &Path, timeout: Duration) -> Result<Photo, ReviewError> {
    let raw = work.join("raw");
    fs::create_dir_all(&raw).map_err(|_| ReviewError::Io)?;
    sandbox::run_mode(render, input, &raw, timeout, Some("photo"))?;
    let photo = photo_from(&raw);
    let _ = fs::remove_dir_all(&raw);
    photo
}

/// The clean copy from a renderer output folder holding `photo.ppm` and `info.txt`.
pub(crate) fn photo_from(raw: &Path) -> Result<Photo, ReviewError> {
    let lossless = read_capped(&raw.join("info.txt"), 1024).trim() == "Format: png";
    let data = read_untrusted(&raw.join("photo.ppm"), PHOTO_MAX * PHOTO_MAX * 3 + 64).ok_or_else(|| ReviewError::Render("unexpected output from the renderer".into()))?;
    let r = raster::parse_ppm(&data)?;
    drop(data);
    if r.width.max(r.height) > PHOTO_MAX {
        return Err(ReviewError::Render("unexpected output from the renderer".into()));
    }
    let (mime, bytes) = if lossless { ("image/png", raster::encode_png(&r, r.width)?) } else { ("image/jpeg", raster::encode_jpeg(&r, 86)?) };
    Ok(Photo { width: r.width, height: r.height, mime, bytes })
}

/// Review `input`, writing everything into `work` (an empty directory, ideally on tmpfs).
pub fn review(input: &Path, work: &Path, opts: &Options) -> Result<Report, ReviewError> {
    let raw = work.join("raw");
    fs::create_dir_all(&raw).map_err(|_| ReviewError::Io)?;
    let kind = sandbox::run(&opts.render, input, &raw, opts.timeout)?;

    let metadata = analyze::metadata(&read_capped(&raw.join("info.txt"), 64 * 1024));
    let text = read_capped(&raw.join("text.txt"), MAX_TEXT);
    let (words, page_sizes) = analyze::words(&read_capped(&raw.join("words.html"), 64 * 1024 * 1024));

    let mut pages = page_files(&raw);
    pages.truncate(MAX_PAGES);
    if pages.is_empty() {
        return Err(ReviewError::Render("the file has no pages to show".into()));
    }
    let clean_path = work.join("clean.pdf");
    let file = fs::File::create(&clean_path).map_err(|_| ReviewError::Io)?;
    let mut writer = pdf::PdfWriter::new(std::io::BufWriter::new(file), DPI).map_err(|_| ReviewError::Io)?;
    let mut previews = Vec::new();
    let mut printer_dots = Vec::new();
    let mut hidden_text = Vec::new();
    for (i, path) in pages.iter().enumerate() {
        let data = read_untrusted(path, MAX_PAGE_BYTES).ok_or_else(|| ReviewError::Render("unexpected output from the renderer".into()))?;
        let mut r = raster::parse_ppm(&data)?;
        drop(data);
        let _ = fs::remove_file(path);

        // Hidden text: words whose box is almost entirely dark on the rendered page.
        if let Some((pw, _)) = page_sizes.get(i) {
            let scale = r.width as f32 / pw;
            let mut found: Vec<String> = Vec::new();
            for w in words.iter().filter(|w| w.page == i) {
                let (x0, y0) = ((w.x0 * scale).floor().max(0.0) as usize, (w.y0 * scale).floor().max(0.0) as usize);
                let (x1, y1) = ((w.x1 * scale).ceil() as usize, (w.y1 * scale).ceil() as usize);
                if (x1.saturating_sub(x0)) * (y1.saturating_sub(y0)) >= 20 && raster::dark_fraction(&r, x0, y0, x1, y1) > 0.92 {
                    found.push(w.text.clone());
                }
            }
            if !found.is_empty() {
                hidden_text.push(HiddenText { page: i + 1, count: found.len(), sample: found.into_iter().take(8).collect() });
            }
        }

        let (specks, fraction) = raster::yellow_specks(&r);
        if dots_verdict(specks, fraction) {
            printer_dots.push(DotsFinding { page: i + 1, specks });
        }
        if opts.greyscale {
            raster::to_grey(&mut r);
        } else if opts.strip_yellow {
            raster::remove_yellow(&mut r);
        }
        writer.add_page(&r).map_err(|_| ReviewError::Io)?;
        let png = raster::encode_png(&r, 1200)?;
        let p = work.join(format!("page-{:04}.png", i + 1));
        fs::write(&p, png).map_err(|_| ReviewError::Io)?;
        previews.push(p);
    }
    writer.finish().map_err(|_| ReviewError::Io)?;
    let _ = fs::remove_dir_all(&raw);
    Ok(Report {
        kind,
        pages: previews.len(),
        metadata,
        printer_dots,
        hidden_text,
        has_text: !text.trim().is_empty(),
        text,
        clean_pdf: clean_path,
        previews,
    })
}
