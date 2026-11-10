//! Strict parsing of the sandbox's pixel output, and pixel-level checks.

use crate::ReviewError;

pub const MAX_DIM: usize = 12_000;

pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Parse a binary PPM (P6, maxval 255). Anything else is rejected.
pub fn parse_ppm(data: &[u8]) -> Result<Raster, ReviewError> {
    let bad = || ReviewError::Render("unexpected output from the renderer".into());
    let mut pos = 0usize;
    let mut fields: Vec<usize> = Vec::new();
    if data.len() < 2 || &data[..2] != b"P6" {
        return Err(bad());
    }
    pos += 2;
    while fields.len() < 3 {
        while pos < data.len() && data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos < data.len() && data[pos] == b'#' {
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < data.len() && data[pos].is_ascii_digit() {
            pos += 1;
        }
        if start == pos || pos - start > 6 {
            return Err(bad());
        }
        fields.push(std::str::from_utf8(&data[start..pos]).map_err(|_| bad())?.parse().map_err(|_| bad())?);
    }
    if pos >= data.len() || !data[pos].is_ascii_whitespace() {
        return Err(bad());
    }
    pos += 1;
    let (w, h, max) = (fields[0], fields[1], fields[2]);
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || max != 255 {
        return Err(bad());
    }
    let len = w * h * 3;
    let px = data.get(pos..pos + len).ok_or_else(bad)?;
    Ok(Raster { width: w, height: h, rgb: px.to_vec() })
}

fn is_print_yellow(r: u8, g: u8, b: u8) -> bool {
    r >= 200 && g >= 190 && (r.min(g) as i16 - b as i16) >= 45
}

/// Count isolated pale-yellow specks on light backgrounds: the signature of printer tracking dots.
/// Returns `(specks, yellow fraction of the page)`.
pub fn yellow_specks(r: &Raster) -> (u32, f32) {
    let (w, h) = (r.width, r.height);
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        (r.rgb[i], r.rgb[i + 1], r.rgb[i + 2])
    };
    let mut yellow = 0u64;
    let mut specks = 0u32;
    for y in 2..h.saturating_sub(2) {
        for x in 2..w.saturating_sub(2) {
            let (pr, pg, pb) = px(x, y);
            if !is_print_yellow(pr, pg, pb) {
                continue;
            }
            yellow += 1;
            // Isolated: the ring two pixels out is mostly near-white paper.
            let ring = [(x - 2, y), (x + 2, y), (x, y - 2), (x, y + 2), (x - 2, y - 2), (x + 2, y + 2), (x - 2, y + 2), (x + 2, y - 2)];
            let paper = ring.iter().filter(|(a, b)| {
                let (rr, gg, bb) = px(*a, *b);
                rr > 225 && gg > 225 && bb > 200
            });
            if paper.count() >= 6 {
                specks += 1;
            }
        }
    }
    (specks, yellow as f32 / (w * h).max(1) as f32)
}

/// Turn print-yellow pixels white (removes tracking dots while keeping colour elsewhere).
pub fn remove_yellow(r: &mut Raster) {
    for p in r.rgb.chunks_exact_mut(3) {
        if is_print_yellow(p[0], p[1], p[2]) {
            p.copy_from_slice(&[255, 255, 255]);
        }
    }
}

pub fn to_grey(r: &mut Raster) {
    for p in r.rgb.chunks_exact_mut(3) {
        let l = ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8;
        p.copy_from_slice(&[l, l, l]);
    }
}

/// Fraction of dark pixels in a rectangle (pixel coordinates, clamped).
pub fn dark_fraction(r: &Raster, x0: usize, y0: usize, x1: usize, y1: usize) -> f32 {
    let (x1, y1) = (x1.min(r.width), y1.min(r.height));
    if x0 >= x1 || y0 >= y1 {
        return 0.0;
    }
    let mut dark = 0u64;
    let mut total = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * r.width + x) * 3;
            let l = (r.rgb[i] as u32 * 299 + r.rgb[i + 1] as u32 * 587 + r.rgb[i + 2] as u32 * 114) / 1000;
            if l < 60 {
                dark += 1;
            }
            total += 1;
        }
    }
    dark as f32 / total as f32
}

pub fn encode_png(r: &Raster, max_width: usize) -> Result<Vec<u8>, ReviewError> {
    // Downscale by integer steps for previews.
    let step = r.width.div_ceil(max_width).max(1);
    let (w, h) = (r.width / step, r.height / step);
    let mut small = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * step) * r.width + x * step) * 3;
            small.extend_from_slice(&r.rgb[i..i + 3]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|_| ReviewError::Io)?;
        writer.write_image_data(&small).map_err(|_| ReviewError::Io)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(w: usize, h: usize) -> Raster {
        Raster { width: w, height: h, rgb: vec![255; w * h * 3] }
    }

    #[test]
    fn ppm_parsing_is_strict() {
        let mut ok = b"P6\n# c\n2 1\n255\n".to_vec();
        ok.extend([1, 2, 3, 4, 5, 6]);
        let r = parse_ppm(&ok).unwrap();
        assert_eq!((r.width, r.height, r.rgb.len()), (2, 1, 6));
        assert!(parse_ppm(b"P3\n2 1\n255\n").is_err());
        assert!(parse_ppm(b"P6\n2 1\n65535\n").is_err());
        assert!(parse_ppm(b"P6\n99999999 1\n255\n").is_err());
        assert!(parse_ppm(b"P6\n2 2\n255\n123").is_err(), "truncated pixels");
    }

    #[test]
    fn finds_and_removes_tracking_dots() {
        let mut r = white(200, 200);
        for (x, y) in [(20, 20), (60, 20), (100, 20), (20, 60), (60, 60), (100, 60), (140, 140), (180, 100)] {
            let i = (y * 200 + x) * 3;
            r.rgb[i..i + 3].copy_from_slice(&[255, 245, 150]);
        }
        let (specks, frac) = yellow_specks(&r);
        assert_eq!(specks, 8);
        assert!(frac < 0.001);
        remove_yellow(&mut r);
        assert_eq!(yellow_specks(&r).0, 0);
        // A big yellow graphic is not "specks".
        let mut g = white(100, 100);
        for p in g.rgb.chunks_exact_mut(3) {
            p.copy_from_slice(&[255, 240, 120]);
        }
        assert_eq!(yellow_specks(&g).0, 0);
    }

    #[test]
    fn dark_fraction_and_png() {
        let mut r = white(10, 10);
        for y in 2..6 {
            for x in 2..8 {
                let i = (y * 10 + x) * 3;
                r.rgb[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        assert!(dark_fraction(&r, 2, 2, 8, 6) > 0.99);
        assert!(dark_fraction(&r, 0, 7, 10, 10) < 0.01);
        assert!(encode_png(&r, 5).unwrap().starts_with(b"\x89PNG"));
    }
}
