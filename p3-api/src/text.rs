//! Text for the game's UI.
//!
//! The game's fonts and strings are Latin-1, Rust strings are UTF-8. Encode text with
//! [`latin1`] before passing it to the game's render functions, or umlauts come out garbled.
//! Mods that draw their own text use [`Localized`] to pick the game's language.

use std::ffi::CString;

use crate::version::{game_version, GameVersion};

/// Encode `text` as a NUL-terminated Latin-1 string. Characters outside Latin-1 become `?`.
pub fn latin1(text: &str) -> CString {
    let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }).collect();
    CString::new(bytes).unwrap_or_default()
}

/// A text in the languages of the supported builds.
pub struct Localized {
    pub en: &'static str,
    pub de: &'static str,
}

impl Localized {
    /// The text in the running build's language (English for unknown builds), Latin-1 encoded.
    pub fn get(&self) -> CString {
        latin1(match game_version() {
            GameVersion::German => self.de,
            GameVersion::International | GameVersion::Unknown => self.en,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_encodes_umlauts() {
        assert_eq!(latin1("Gewürze").as_bytes(), b"Gew\xfcrze");
        assert_eq!(latin1("Qualitätsstufe €").as_bytes(), b"Qualit\xe4tsstufe ?");
    }
}
