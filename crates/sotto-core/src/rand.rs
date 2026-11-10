//! OS randomness. Sotto never uses a userspace PRNG for key material.

/// Fill a fixed array with OS randomness.
///
/// Panics only if the operating system cannot provide randomness at all, in
/// which case no safe operation is possible.
pub fn bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    fill(&mut b);
    b
}

/// Fill a buffer with OS randomness.
pub fn fill(buf: &mut [u8]) {
    getrandom::fill(buf).expect("operating system randomness is unavailable");
}

/// Uniform random number below `bound` without modulo bias.
pub fn below(bound: u32) -> u32 {
    assert!(bound > 0);
    let zone = u32::MAX - (u32::MAX % bound);
    loop {
        let v = u32::from_le_bytes(bytes::<4>());
        if v < zone {
            return v % bound;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn below_stays_in_range() {
        for _ in 0..1000 {
            assert!(super::below(7) < 7);
        }
        assert_ne!(super::bytes::<32>(), super::bytes::<32>());
    }
}
