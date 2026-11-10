//! Fixed-size padding: `u32 LE length || data || zeros`.

use crate::{Error, Result};

pub fn pad(data: &[u8], size: usize) -> Result<Vec<u8>> {
    if data.len() + 4 > size {
        return Err(Error::TooLarge);
    }
    let mut out = vec![0u8; size];
    out[..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
    out[4..4 + data.len()].copy_from_slice(data);
    Ok(out)
}

pub fn unpad(buf: &[u8]) -> Result<&[u8]> {
    if buf.len() < 4 {
        return Err(Error::Malformed("padded buffer too short"));
    }
    let len = u32::from_le_bytes(buf[..4].try_into().expect("4 bytes")) as usize;
    buf.get(4..4 + len).ok_or(Error::Malformed("padding length"))
}

/// Largest payload that fits in a padded buffer of `size` bytes.
pub const fn capacity(size: usize) -> usize {
    size - 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_to_exact_size() {
        let p = pad(b"abc", 32).unwrap();
        assert_eq!(p.len(), 32);
        assert_eq!(unpad(&p).unwrap(), b"abc");
        assert_eq!(pad(&[0u8; 29], 32), Err(Error::TooLarge));
        assert!(pad(&[0u8; 28], 32).is_ok());
        let mut bad = p.clone();
        bad[0] = 200;
        assert!(unpad(&bad).is_err());
    }
}
