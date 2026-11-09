//! İstemci kimliği. Discord'a giden tüm istemci bilgisi (IDENTIFY `properties`,
//! `User-Agent`, `X-Super-Properties`, `Accept-Language`) **sadece** buradan üretilir.
//! Alan adları ve sırası docs/PROTOCOL_NOTES.md §3.1'deki gözleme birebir uyar.

use serde::Serialize;

/// Gözlenen web istemcisi (2026-11-09). Config ile geçersiz kılınabilir.
pub const DEFAULT_BUILD_NUMBER: u32 = 667_580;
pub const DEFAULT_CHROME_MAJOR: u32 = 155;

/// "Linux + Chrome web istemcisi" profili.
#[derive(Debug, Clone, Serialize)]
pub struct ClientProperties {
    os: &'static str,
    browser: &'static str,
    device: &'static str,
    device_model_name: &'static str,
    system_locale: String,
    has_client_mods: bool,
    browser_user_agent: String,
    browser_version: String,
    os_version: &'static str,
    referrer: &'static str,
    referring_domain: &'static str,
    referrer_current: &'static str,
    referring_domain_current: &'static str,
    release_channel: &'static str,
    client_build_number: u32,
    client_event_source: Option<()>,
    client_launch_id: String,
    // Kaynağı doğrulanmadı (PROTOCOL_NOTES §3.1); bilinene kadar gönderilmez.
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_signature: Option<String>,
    client_app_state: &'static str,
    is_fast_connect: bool,
    gateway_connect_reasons: &'static str,
    // Kaynağı doğrulanmadı (PROTOCOL_NOTES §3.1); bilinene kadar gönderilmez.
    #[serde(skip_serializing_if = "Option::is_none")]
    installation_id: Option<String>,
    /// Sadece `X-Super-Properties`'te ve op 41'de; oturum başına üretilir (PROTOCOL_NOTES §4.1).
    #[serde(skip)]
    client_heartbeat_session_id: String,
    #[serde(skip)]
    chrome_major: u32,
}

/// `X-Super-Properties` içeriği: IDENTIFY `properties`'ten farklı alan kümesi (PROTOCOL_NOTES §4.1).
#[derive(Serialize)]
struct SuperProperties<'a> {
    os: &'a str,
    browser: &'a str,
    device: &'a str,
    device_model_name: &'a str,
    system_locale: &'a str,
    has_client_mods: bool,
    browser_user_agent: &'a str,
    browser_version: &'a str,
    os_version: &'a str,
    referrer: &'a str,
    referring_domain: &'a str,
    referrer_current: &'a str,
    referring_domain_current: &'a str,
    release_channel: &'a str,
    client_build_number: u32,
    client_event_source: Option<()>,
    client_launch_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_signature: Option<&'a str>,
    client_heartbeat_session_id: &'a str,
    client_app_state: &'a str,
}

impl ClientProperties {
    /// `locale` BCP-47 biçiminde (ör. `tr-TR`). `client_launch_id` her açılışta yeni üretilir.
    pub fn chrome_linux(locale: &str, chrome_major: u32, build_number: u32) -> Self {
        let browser_version = format!("{chrome_major}.0.0.0");
        Self {
            os: "Linux",
            browser: "Chrome",
            device: "",
            device_model_name: "",
            system_locale: locale.to_owned(),
            has_client_mods: false,
            browser_user_agent: format!(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{browser_version} Safari/537.36"
            ),
            browser_version,
            os_version: "",
            referrer: "",
            referring_domain: "",
            referrer_current: "",
            referring_domain_current: "",
            release_channel: "stable",
            client_build_number: build_number,
            client_event_source: None,
            client_launch_id: uuid_v4(),
            launch_signature: None,
            client_app_state: "unfocused",
            is_fast_connect: false,
            gateway_connect_reasons: "AppSkeleton",
            installation_id: None,
            client_heartbeat_session_id: uuid_v4(),
            chrome_major,
        }
    }

    /// `X-Super-Properties` başlığı: alt kümenin JSON'unun base64'ü.
    pub fn super_properties(&self) -> String {
        use base64::Engine;
        let sp = SuperProperties {
            os: self.os,
            browser: self.browser,
            device: self.device,
            device_model_name: self.device_model_name,
            system_locale: &self.system_locale,
            has_client_mods: self.has_client_mods,
            browser_user_agent: &self.browser_user_agent,
            browser_version: &self.browser_version,
            os_version: self.os_version,
            referrer: self.referrer,
            referring_domain: self.referring_domain,
            referrer_current: self.referrer_current,
            referring_domain_current: self.referring_domain_current,
            release_channel: self.release_channel,
            client_build_number: self.client_build_number,
            client_event_source: self.client_event_source,
            client_launch_id: &self.client_launch_id,
            launch_signature: self.launch_signature.as_deref(),
            client_heartbeat_session_id: &self.client_heartbeat_session_id,
            client_app_state: self.client_app_state,
        };
        // Sabit yapılı, string anahtarlı tip; serileştirme başarısız olamaz.
        let json = serde_json::to_vec(&sp).unwrap_or_default();
        base64::engine::general_purpose::STANDARD.encode(json)
    }

    /// `sec-ch-ua` başlığı (gözlenen Chrome 155 biçimi; "Not(A:Brand" kısmı Chrome sürümüne göre
    /// değişebilir — diğer sürümler doğrulanmadı).
    pub fn sec_ch_ua(&self) -> String {
        let v = self.chrome_major;
        format!(r#""Google Chrome";v="{v}", "Chromium";v="{v}", "Not(A:Brand";v="24""#)
    }

    /// `X-Discord-Locale`: sadece dil kısmı (`tr-TR` → `tr`).
    pub fn discord_locale(&self) -> &str {
        self.system_locale
            .split('-')
            .next()
            .unwrap_or(&self.system_locale)
    }

    pub fn user_agent(&self) -> &str {
        &self.browser_user_agent
    }

    pub fn locale(&self) -> &str {
        &self.system_locale
    }

    /// Chrome'un bu dil için gönderdiği `Accept-Language` başlığı.
    /// Gözlenen: `tr-TR` → `tr-TR,tr;q=0.9,en-US;q=0.8,en;q=0.7`. Diğer diller doğrulanmadı.
    pub fn accept_language(&self) -> String {
        let full = self.system_locale.as_str();
        let lang = full.split('-').next().unwrap_or(full);
        if lang == "en" {
            format!("{full},en;q=0.9")
        } else {
            format!("{full},{lang};q=0.9,en-US;q=0.8,en;q=0.7")
        }
    }
}

/// Rastgele UUID v4 metni.
fn uuid_v4() -> String {
    let mut b: [u8; 16] = fastrand::u128(..).to_le_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = |r: std::ops::Range<usize>| b[r].iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        h(0..4),
        h(4..6),
        h(6..8),
        h(8..10),
        h(10..16)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_observed_web_client() {
        let p = ClientProperties::chrome_linux("tr-TR", 155, 667_580);
        let json = serde_json::to_value(&p).expect("json");
        // Gözlenen IDENTIFY'dan (PROTOCOL_NOTES §3.1); kaynağı belirsiz alanlar hariç.
        let launch_id = json["client_launch_id"].clone();
        let expected = serde_json::json!({
            "os":"Linux","browser":"Chrome","device":"","device_model_name":"","system_locale":"tr-TR",
            "has_client_mods":false,
            "browser_user_agent":"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36",
            "browser_version":"155.0.0.0","os_version":"","referrer":"","referring_domain":"",
            "referrer_current":"","referring_domain_current":"","release_channel":"stable",
            "client_build_number":667580,"client_event_source":null,
            "client_launch_id": launch_id,
            "client_app_state":"unfocused","is_fast_connect":false,"gateway_connect_reasons":"AppSkeleton"
        });
        assert_eq!(json, expected);
        assert_eq!(p.user_agent(), json["browser_user_agent"]);
    }

    #[test]
    fn field_order_matches_observed() {
        let s =
            serde_json::to_string(&ClientProperties::chrome_linux("tr-TR", 155, 1)).expect("json");
        let keys = [
            "\"os\"",
            "\"system_locale\"",
            "\"browser_user_agent\"",
            "\"client_build_number\"",
            "\"client_launch_id\"",
            "\"gateway_connect_reasons\"",
        ];
        let pos: Vec<usize> = keys.iter().map(|k| s.find(k).expect("anahtar")).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{s}");
    }

    #[test]
    fn launch_id_is_uuid_v4() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(uuid_v4(), id);
    }

    #[test]
    fn super_properties_matches_observed() {
        use base64::Engine;
        let p = ClientProperties::chrome_linux("tr-TR", 155, 667_580);
        let raw = base64::engine::general_purpose::STANDARD
            .decode(p.super_properties())
            .expect("base64");
        let text = String::from_utf8(raw).expect("utf8");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        let keys = [
            "os",
            "browser",
            "device",
            "device_model_name",
            "system_locale",
            "has_client_mods",
            "browser_user_agent",
            "browser_version",
            "os_version",
            "referrer",
            "referring_domain",
            "referrer_current",
            "referring_domain_current",
            "release_channel",
            "client_build_number",
            "client_event_source",
            "client_launch_id",
            "client_heartbeat_session_id",
            "client_app_state",
        ];
        assert_eq!(v.as_object().expect("nesne").len(), keys.len(), "{text}");
        let pos: Vec<usize> = keys
            .iter()
            .map(|k| text.find(&format!("\"{k}\"")).expect("anahtar"))
            .collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{text}");
        assert_eq!(
            v["client_launch_id"],
            serde_json::to_value(&p).expect("json")["client_launch_id"]
        );
        assert_ne!(v["client_heartbeat_session_id"], v["client_launch_id"]);
        assert_eq!(
            p.sec_ch_ua(),
            r#""Google Chrome";v="155", "Chromium";v="155", "Not(A:Brand";v="24""#
        );
        assert_eq!(p.discord_locale(), "tr");
    }

    #[test]
    fn accept_language() {
        let tr = ClientProperties::chrome_linux("tr-TR", 155, 1);
        assert_eq!(tr.accept_language(), "tr-TR,tr;q=0.9,en-US;q=0.8,en;q=0.7");
        let en = ClientProperties::chrome_linux("en-US", 155, 1);
        assert_eq!(en.accept_language(), "en-US,en;q=0.9");
    }
}
