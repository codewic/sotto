//! Safety numbers: a short string two people compare to confirm they hold each other's real keys.

use crate::kdf::sha512;

/// Eight groups of five digits, identical on both sides regardless of argument order.
pub fn safety_number(a_sign: &[u8; 32], b_sign: &[u8; 32]) -> String {
    let (lo, hi) = if a_sign <= b_sign { (a_sign, b_sign) } else { (b_sign, a_sign) };
    let mut data = b"sotto-safety-v1".to_vec();
    data.extend_from_slice(lo);
    data.extend_from_slice(hi);
    let digest = sha512(&data);
    digest[..40]
        .chunks(5)
        .map(|c| {
            let v = c.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64);
            format!("{:05}", v % 100_000)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetric_and_shaped() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let n = safety_number(&a, &b);
        assert_eq!(n, safety_number(&b, &a));
        assert_eq!(n.split(' ').count(), 8);
        assert!(n.split(' ').all(|g| g.len() == 5 && g.chars().all(|c| c.is_ascii_digit())));
        assert_ne!(n, safety_number(&a, &[3u8; 32]));
    }
}
