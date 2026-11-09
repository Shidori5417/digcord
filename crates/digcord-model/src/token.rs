use std::fmt;

use serde::{Serialize, Serializer};

/// Kullanıcı token'ı. `Debug`/`Display` içeriği asla göstermez; değere sadece
/// [`Token::expose`] ile bilerek erişilir.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Baştaki/sondaki boşlukları ve tırnakları temizler; boşsa `None`.
    pub fn new(raw: &str) -> Option<Self> {
        let t = raw.trim().trim_matches('"').trim();
        (!t.is_empty()).then(|| Self(t.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token([gizli])")
    }
}

impl Serialize for Token {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_value() {
        let t = Token::new("gizli-deger").expect("token");
        assert_eq!(format!("{t:?}"), "Token([gizli])");
        assert_eq!(t.expose(), "gizli-deger");
    }

    #[test]
    fn trims_input() {
        assert_eq!(Token::new("  \"abc\"\n").expect("token").expose(), "abc");
        assert!(Token::new("  \"\" ").is_none());
    }
}
