//! Minimal PDF writer: one full-page image per page, no metadata at all.

use std::io::{Seek, Write};

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::raster::Raster;

pub struct PdfWriter<W: Write + Seek> {
    out: W,
    offsets: Vec<u64>,
    pages: Vec<usize>,
    dpi: f32,
}

impl<W: Write + Seek> PdfWriter<W> {
    pub fn new(mut out: W, dpi: f32) -> std::io::Result<Self> {
        out.write_all(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")?;
        // Objects 1 (catalog) and 2 (page tree) are written at the end.
        Ok(Self { out, offsets: vec![0, 0], pages: Vec::new(), dpi })
    }

    fn begin(&mut self) -> std::io::Result<usize> {
        self.offsets.push(self.out.stream_position()?);
        let n = self.offsets.len();
        write!(self.out, "{n} 0 obj\n")?;
        Ok(n)
    }

    pub fn add_page(&mut self, r: &Raster) -> std::io::Result<()> {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&r.rgb)?;
        let data = z.finish()?;
        let img = self.begin()?;
        write!(
            self.out,
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            r.width,
            r.height,
            data.len()
        )?;
        self.out.write_all(&data)?;
        self.out.write_all(b"\nendstream\nendobj\n")?;
        let (w, h) = (r.width as f32 * 72.0 / self.dpi, r.height as f32 * 72.0 / self.dpi);
        let content = format!("q {w:.2} 0 0 {h:.2} 0 0 cm /Im0 Do Q");
        let c = self.begin()?;
        write!(self.out, "<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n", content.len())?;
        let p = self.begin()?;
        write!(
            self.out,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /XObject << /Im0 {img} 0 R >> >> /Contents {c} 0 R >>\nendobj\n"
        )?;
        self.pages.push(p);
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        self.offsets[0] = self.out.stream_position()?;
        self.out.write_all(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")?;
        self.offsets[1] = self.out.stream_position()?;
        let kids: String = self.pages.iter().map(|p| format!("{p} 0 R ")).collect();
        write!(self.out, "2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {} >>\nendobj\n", self.pages.len())?;
        let xref = self.out.stream_position()?;
        write!(self.out, "xref\n0 {}\n0000000000 65535 f \n", self.offsets.len() + 1)?;
        for o in &self.offsets {
            write!(self.out, "{o:010} 00000 n \n")?;
        }
        write!(self.out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.offsets.len() + 1)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_structurally_valid_pdf() {
        let r = Raster { width: 4, height: 2, rgb: vec![200; 24] };
        let mut w = PdfWriter::new(std::io::Cursor::new(Vec::new()), 150.0).unwrap();
        w.add_page(&r).unwrap();
        w.add_page(&r).unwrap();
        let bytes = w.finish().unwrap().into_inner();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.starts_with("%PDF-1.7"));
        assert!(s.contains("/Count 2"));
        assert!(!s.contains("/Info"), "no metadata dictionary");
        assert!(s.trim_end().ends_with("%%EOF"));
        // The xref offset (a byte offset) points at the xref keyword.
        let tail = s.rsplit("startxref\n").next().unwrap();
        let start: usize = tail.lines().next().unwrap().parse().unwrap();
        assert!(bytes[start..].starts_with(b"xref"));
    }
}
