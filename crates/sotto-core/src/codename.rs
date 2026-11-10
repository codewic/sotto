//! Drop codenames: eight words from the BIP-39 English list (88 bits).
//!
//! A codename is the only thing a source needs to come back to a drop. It is
//! generated on the server, shown once, and never stored: the drop keeps only
//! a public key derived from it with Argon2id.

use bip39::Language;

use crate::{Error, Result};

pub const WORDS: usize = 8;

pub fn generate() -> String {
    let list = Language::English.word_list();
    (0..WORDS).map(|_| list[crate::rand::below(2048) as usize]).collect::<Vec<_>>().join(" ")
}

/// Normalise what a source typed: case, extra spaces, line breaks and hyphens are forgiven,
/// unknown words are not.
pub fn normalize(input: &str) -> Result<String> {
    let words: Vec<String> = input
        .split(|c: char| c.is_whitespace() || c == '-' || c == ',' || c == '.')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    if words.len() != WORDS {
        return Err(Error::Malformed("codename must have eight words"));
    }
    for w in &words {
        if Language::English.find_word(w).is_none() {
            return Err(Error::Malformed("codename word not recognised"));
        }
    }
    Ok(words.join(" "))
}

/// 24 words (with checksum) for 32 bytes of entropy: the travel-mode recovery phrase.
pub fn recovery_words(entropy: &[u8; 32]) -> String {
    bip39::Mnemonic::from_entropy_in(Language::English, entropy).expect("32 bytes is valid entropy").words().collect::<Vec<_>>().join(" ")
}

/// Parse a recovery phrase back into its entropy. Case, spacing and line breaks are forgiven; the checksum is not.
pub fn recovery_entropy(words: &str) -> Result<[u8; 32]> {
    let clean = words.split_whitespace().map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" ");
    let m = bip39::Mnemonic::parse_in(Language::English, &clean).map_err(|_| Error::Malformed("recovery phrase"))?;
    let e = m.to_entropy();
    e.try_into().map_err(|_| Error::Malformed("recovery phrase length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_phrase_round_trip() {
        let e = [9u8; 32];
        let w = recovery_words(&e);
        assert_eq!(w.split(' ').count(), 24);
        assert_eq!(recovery_entropy(&w.to_uppercase().replace(' ', "\n ")).unwrap(), e);
        let mut wrong: Vec<&str> = w.split(' ').collect();
        wrong.swap(0, 1);
        assert!(recovery_entropy(&wrong.join(" ")).is_err() || recovery_entropy(&wrong.join(" ")).unwrap() != e);
    }

    #[test]
    fn generate_and_normalize() {
        let c = generate();
        assert_eq!(c.split(' ').count(), WORDS);
        assert_eq!(normalize(&c).unwrap(), c);
        let messy = format!("  {}\n", c.to_uppercase().replace(' ', "  -  "));
        assert_eq!(normalize(&messy).unwrap(), c);
        assert!(normalize("abandon ability").is_err());
        assert!(normalize("abandon ability able about above absent absorb notaword").is_err());
    }
}
