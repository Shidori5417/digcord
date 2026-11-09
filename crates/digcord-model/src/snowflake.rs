use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Discord epoch'u (2015-01-01T00:00:00Z), milisaniye.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// Discord kimliği. JSON'da string olarak gelir/gider, bellekte `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(pub u64);

impl Snowflake {
    /// Oluşturulma zamanı, Unix milisaniyesi.
    pub fn timestamp_ms(self) -> u64 {
        (self.0 >> 22) + DISCORD_EPOCH_MS
    }

    /// Verilen Unix milisaniyesinden snowflake (alt bitler 0). Mesaj `nonce`'u için.
    pub fn from_timestamp_ms(ms: u64) -> Self {
        Self(ms.saturating_sub(DISCORD_EPOCH_MS) << 22)
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Snowflake;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("string veya sayı olarak snowflake")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Snowflake, E> {
                Ok(Snowflake(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Snowflake, E> {
                v.parse().map(Snowflake).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_and_number() {
        let a: Snowflake = serde_json::from_str(r#""175928847299117063""#).expect("string");
        let b: Snowflake = serde_json::from_str("175928847299117063").expect("sayı");
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).expect("yaz"),
            r#""175928847299117063""#
        );
        assert!(serde_json::from_str::<Snowflake>(r#""abc""#).is_err());
    }

    #[test]
    fn from_timestamp_roundtrip() {
        let s = Snowflake::from_timestamp_ms(1_767_268_800_123);
        assert_eq!(s.timestamp_ms(), 1_767_268_800_123);
        assert_eq!(Snowflake::from_timestamp_ms(0), Snowflake(0));
    }

    #[test]
    fn timestamp() {
        // Discord dokümanlarındaki örnek: 2016-04-30T11:18:25.796Z
        assert_eq!(
            Snowflake(175_928_847_299_117_063).timestamp_ms(),
            1_462_015_105_796
        );
    }
}
