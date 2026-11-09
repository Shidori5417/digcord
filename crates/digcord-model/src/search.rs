//! Mesaj arama ve sabitlenmiş mesajlar: sorgu ayrıştırma ve yanıt biçimleri.
//!
//! Arama uç noktaları kullanıcı istemcisine özgü ve belgesiz (PROTOCOL_NOTES §4.5,
//! doğrulanmadı). Burada ağ yok: sadece metin → sorgu ve JSON → mesaj listesi dönüşümleri.

use serde_json::Value;

use crate::{Message, Snowflake};

/// `has:` süzgeci değerleri (resmi istemcinin `has=` parametresi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Has {
    Link,
    File,
    Image,
    Embed,
}

impl Has {
    /// Sorgu parametresindeki değer.
    pub fn as_param(self) -> &'static str {
        match self {
            Has::Link => "link",
            Has::File => "file",
            Has::Image => "image",
            Has::Embed => "embed",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "link" => Some(Has::Link),
            "file" => Some(Has::File),
            "image" => Some(Has::Image),
            "embed" => Some(Has::Embed),
            _ => None,
        }
    }
}

/// Yazılan arama metninin ham hali: süzgeçler henüz kimliğe çözülmedi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawSearch {
    /// Süzgeç olmayan sözcükler (tek boşlukla birleşik).
    pub content: String,
    /// `from:` değeri (kimlik, `<@id>` ya da `@ad`), `@` dahil değil/dahil olabilir.
    pub from: Option<String>,
    /// `in:` değeri (`#ad`, `<#id>` ya da kimlik).
    pub channel: Option<String>,
    pub has: Vec<Has>,
    /// Tanınmayan `has:` değerleri (kullanıcıya bildirilir).
    pub bad_has: Vec<String>,
}

/// Sorguyu sözcüklere böler: boşlukla ayrılır, `"çift tırnak"` içi tek sözcüktür.
/// Tırnaklı sözcük her zaman düz içerik sayılır (süzgeç değil); tırnaklar atılır.
fn tokens(input: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut in_quote = false;
    let mut started = false;
    for c in input.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                quoted = true;
                started = true;
            }
            c if c.is_whitespace() && !in_quote => {
                if started {
                    out.push((std::mem::take(&mut cur), quoted));
                    quoted = false;
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push((cur, quoted));
    }
    out
}

/// `from:x in:#y has:link kelime "tam ifade"` biçimindeki metni ayrıştırır.
/// Süzgeçlerde son yazılan kazanır; boş değerli süzgeç yok sayılır.
pub fn parse_search(input: &str) -> RawSearch {
    let mut raw = RawSearch::default();
    let mut words: Vec<String> = Vec::new();
    for (tok, quoted) in tokens(input) {
        if !quoted && let Some((key, val)) = tok.split_once(':') {
            let val = val.trim();
            match key.to_ascii_lowercase().as_str() {
                "from" if !val.is_empty() => {
                    raw.from = Some(val.to_owned());
                    continue;
                }
                "in" if !val.is_empty() => {
                    raw.channel = Some(val.to_owned());
                    continue;
                }
                "has" if !val.is_empty() => {
                    match Has::parse(val) {
                        Some(h) if !raw.has.contains(&h) => raw.has.push(h),
                        Some(_) => {}
                        None => raw.bad_has.push(val.to_owned()),
                    }
                    continue;
                }
                _ => {}
            }
        }
        words.push(tok);
    }
    raw.content = words.join(" ");
    raw
}

/// `<@123>`, `<@!123>`, `123` → kimlik. Başka biçim `None`.
pub fn parse_user_ref(s: &str) -> Option<Snowflake> {
    let inner = s
        .strip_prefix("<@")
        .and_then(|r| r.strip_suffix('>'))
        .map(|r| r.trim_start_matches('!'))
        .unwrap_or(s);
    inner.parse::<u64>().ok().filter(|n| *n > 0).map(Snowflake)
}

/// `<#123>`, `123` → kimlik. Başka biçim (ör. `#ad`) `None`.
pub fn parse_channel_ref(s: &str) -> Option<Snowflake> {
    let inner = s
        .strip_prefix("<#")
        .and_then(|r| r.strip_suffix('>'))
        .unwrap_or(s);
    inner.parse::<u64>().ok().filter(|n| *n > 0).map(Snowflake)
}

/// Kimliğe çözülmüş, istek için hazır sorgu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub content: String,
    pub author: Option<Snowflake>,
    pub channel: Option<Snowflake>,
    pub has: Vec<Has>,
}

impl SearchQuery {
    /// İçerik de süzgeç de yoksa arama anlamsızdır.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
            && self.author.is_none()
            && self.channel.is_none()
            && self.has.is_empty()
    }
}

/// Arama sonuç sayfası: her biri bir isabet (varsa bağlam mesajları atılır).
#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    /// Sunucunun bildirdiği toplam isabet (sayfalar dahil).
    pub total: u64,
    pub hits: Vec<Message>,
}

/// `{ "total_results": N, "messages": [[...], ...] }` yanıtı. Her iç dizide `"hit": true`
/// olan eleman isabettir; yoksa ilk eleman. Ayrıştırılamayan gruplar atlanır.
pub fn parse_search_response(v: &Value) -> Option<SearchPage> {
    let groups = v.get("messages")?.as_array()?;
    let hits = groups
        .iter()
        .filter_map(|g| {
            let items = g.as_array()?;
            let hit = items
                .iter()
                .find(|m| m.get("hit").and_then(Value::as_bool) == Some(true))
                .or_else(|| items.first())?;
            serde_json::from_value::<Message>(hit.clone()).ok()
        })
        .collect::<Vec<_>>();
    let total = v
        .get("total_results")
        .and_then(Value::as_u64)
        .unwrap_or(hits.len() as u64);
    Some(SearchPage { total, hits })
}

/// 202 (dizin hazır değil) gövdesindeki `retry_after` saniyesi.
pub fn parse_retry_after(v: &Value) -> Option<f64> {
    v.get("retry_after")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s >= 0.0)
}

/// Sabitlenmiş mesajlar: belgeli düz dizi (`[Message]`) ya da yeni sayfalı biçim
/// (`{"items":[{"pinned_at":..,"message":{..}}],"has_more":..}`; doğrulanmadı).
pub fn parse_pins(v: &Value) -> Option<Vec<Message>> {
    let msgs = |items: &[Value], pick: fn(&Value) -> &Value| {
        items
            .iter()
            .filter_map(|m| serde_json::from_value::<Message>(pick(m).clone()).ok())
            .collect::<Vec<_>>()
    };
    match v {
        Value::Array(items) => Some(msgs(items, |m| m)),
        Value::Object(o) => {
            let items = o.get("items")?.as_array()?;
            Some(msgs(items, |m| m.get("message").unwrap_or(m)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_words_and_filters() {
        let r = parse_search("merhaba dünya from:123 in:#genel has:link has:IMAGE");
        assert_eq!(r.content, "merhaba dünya");
        assert_eq!(r.from.as_deref(), Some("123"));
        assert_eq!(r.channel.as_deref(), Some("#genel"));
        assert_eq!(r.has, vec![Has::Link, Has::Image]);
        assert!(r.bad_has.is_empty());
    }

    #[test]
    fn quotes_keep_filters_literal() {
        let r = parse_search(r#""from:bob" "tam ifade burada" has:file"#);
        assert_eq!(r.content, "from:bob tam ifade burada");
        assert_eq!(r.from, None);
        assert_eq!(r.has, vec![Has::File]);
        // Kapanmayan tırnak sonuna kadar uzanır.
        assert_eq!(parse_search(r#"a "b c"#).content, "a b c");
    }

    #[test]
    fn unicode_and_odd_input() {
        let r = parse_search("  İstanbul   ğüşiöç  from:<@!42>  ");
        assert_eq!(r.content, "İstanbul ğüşiöç");
        assert_eq!(r.from.as_deref(), Some("<@!42>"));
        assert_eq!(parse_search("FROM:7").from.as_deref(), Some("7"));
        // Boş değer ve bilinmeyen anahtar düz sözcüktür.
        assert_eq!(
            parse_search("from: x:y http://a.b").content,
            "from: x:y http://a.b"
        );
        let r = parse_search("has:video has:link has:link");
        assert_eq!(r.has, vec![Has::Link]);
        assert_eq!(r.bad_has, vec!["video"]);
        assert_eq!(parse_search("").content, "");
    }

    #[test]
    fn last_filter_wins() {
        let r = parse_search("from:1 from:2 in:3 in:4");
        assert_eq!(r.from.as_deref(), Some("2"));
        assert_eq!(r.channel.as_deref(), Some("4"));
    }

    #[test]
    fn user_and_channel_refs() {
        assert_eq!(parse_user_ref("<@5>"), Some(Snowflake(5)));
        assert_eq!(parse_user_ref("<@!5>"), Some(Snowflake(5)));
        assert_eq!(parse_user_ref("5"), Some(Snowflake(5)));
        assert_eq!(parse_user_ref("@ali"), None);
        assert_eq!(parse_user_ref("0"), None);
        assert_eq!(parse_channel_ref("<#9>"), Some(Snowflake(9)));
        assert_eq!(parse_channel_ref("9"), Some(Snowflake(9)));
        assert_eq!(parse_channel_ref("#genel"), None);
    }

    fn m(id: u64, extra: Value) -> Value {
        let mut v = json!({
            "id": id.to_string(), "channel_id": "2",
            "author": {"id": "1", "username": "a", "global_name": null},
            "content": format!("m{id}")
        });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        v
    }

    #[test]
    fn search_response_picks_hit_else_first() {
        let v = json!({
            "total_results": 57,
            "messages": [
                [m(1, json!({})), m(2, json!({"hit": true})), m(3, json!({}))],
                [m(10, json!({}))],
                [{"bozuk": true}],
            ]
        });
        let page = parse_search_response(&v).expect("sayfa");
        assert_eq!(page.total, 57);
        let ids: Vec<u64> = page.hits.iter().map(|h| h.id.0).collect();
        assert_eq!(ids, vec![2, 10]);
        // total yoksa isabet sayısı.
        let page = parse_search_response(&json!({"messages": [[m(1, json!({}))]]})).expect("sayfa");
        assert_eq!(page.total, 1);
        assert!(parse_search_response(&json!({"message": "x"})).is_none());
    }

    #[test]
    fn retry_after_body() {
        let v = json!({"message": "Index not yet available", "code": 110000, "retry_after": 2.5});
        assert_eq!(parse_retry_after(&v), Some(2.5));
        assert_eq!(parse_retry_after(&json!({})), None);
        assert_eq!(parse_retry_after(&json!({"retry_after": -1})), None);
    }

    #[test]
    fn pins_both_shapes() {
        let old = json!([m(5, json!({})), m(6, json!({}))]);
        assert_eq!(parse_pins(&old).expect("dizi").len(), 2);
        let new =
            json!({"items": [{"pinned_at": "x", "message": m(7, json!({}))}], "has_more": false});
        let pins = parse_pins(&new).expect("sayfalı");
        assert_eq!(pins[0].id, Snowflake(7));
        assert!(parse_pins(&json!("x")).is_none());
        assert!(parse_pins(&json!({"foo": 1})).is_none());
    }

    #[test]
    fn query_emptiness() {
        assert!(SearchQuery::default().is_empty());
        let q = SearchQuery {
            has: vec![Has::Link],
            ..SearchQuery::default()
        };
        assert!(!q.is_empty());
    }
}
