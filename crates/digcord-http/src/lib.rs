//! REST istemcisi ve rate limit yönetimi.
//!
//! Tüm istekler resmi web istemcisinin başlıklarını taşır (PROTOCOL_NOTES §4.1); başlık
//! değerleri sadece [`ClientProperties`]'ten üretilir. Bu aşamada 429 + `retry_after` ve
//! global kilit var; bucket bazlı yönetim Aşama 3'te.

use std::time::Duration;

use digcord_model::{ClientProperties, Message, Snowflake, Token};
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{RequestBuilder, StatusCode};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep_until};

pub const API_BASE: &str = "https://discord.com/api/v9";
/// Aynı istek için en fazla bu kadar 429 sonrası yeniden deneme.
const MAX_RETRIES: u32 = 3;
/// Discord'un söylediğinden fazla bekleme (sunucu tarafı yuvarlama için pay).
const RETRY_PAD: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("token geçersiz (401)")]
    Unauthorized,
    #[error("rate limit aşıldı, {MAX_RETRIES} deneme sonrası vazgeçildi")]
    RateLimited,
    #[error("sunucu {0} döndü")]
    Status(u16),
    #[error("ağ hatası: {0}")]
    Network(#[from] reqwest::Error),
    #[error("geçersiz başlık değeri")]
    Header,
}

/// İsteğin hangi sayfadan yapıldığı (`Referer` başlığı).
#[derive(Debug, Clone, Copy)]
pub struct Page {
    /// `None`: DM (`@me`).
    pub guild: Option<Snowflake>,
    pub channel: Snowflake,
}

impl Page {
    fn referer(self) -> String {
        match self.guild {
            Some(g) => format!("https://discord.com/channels/{g}/{}", self.channel),
            None => format!("https://discord.com/channels/@me/{}", self.channel),
        }
    }
}

pub struct Http {
    client: reqwest::Client,
    base: String,
    auth: HeaderValue,
    /// Global 429 sonrası tüm isteklerin bekleyeceği an.
    blocked_until: Mutex<Option<Instant>>,
}

#[derive(Deserialize)]
struct RateLimitBody {
    retry_after: f64,
    #[serde(default)]
    global: bool,
}

impl Http {
    /// `timezone` IANA adı (ör. `Europe/Istanbul`).
    pub fn new(token: &Token, props: &ClientProperties, timezone: &str) -> Result<Self, HttpError> {
        let value = |s: &str| HeaderValue::from_str(s).map_err(|_| HttpError::Header);
        let mut h = HeaderMap::new();
        h.insert(header::USER_AGENT, value(props.user_agent())?);
        h.insert(header::ACCEPT_LANGUAGE, value(&props.accept_language())?);
        h.insert(
            name("x-super-properties"),
            value(&props.super_properties())?,
        );
        h.insert(name("x-discord-locale"), value(props.discord_locale())?);
        h.insert(name("x-discord-timezone"), value(timezone)?);
        h.insert(
            name("x-debug-options"),
            HeaderValue::from_static("bugReporterEnabled"),
        );
        h.insert(name("sec-ch-ua"), value(&props.sec_ch_ua())?);
        h.insert(name("sec-ch-ua-mobile"), HeaderValue::from_static("?0"));
        h.insert(
            name("sec-ch-ua-platform"),
            HeaderValue::from_static("\"Linux\""),
        );

        // reqwest `rustls-no-provider` ile derleniyor: süreç genelinde ring kurulur.
        // Zaten kuruluysa (ör. gateway kurduysa) hata döner; yok sayılır.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut auth = value(token.expose())?;
        auth.set_sensitive(true);
        let client = reqwest::Client::builder()
            .default_headers(h)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            base: API_BASE.to_owned(),
            auth,
            blocked_until: Mutex::new(None),
        })
    }

    /// Testler için farklı taban adres.
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// `GET /channels/{id}/messages`. Resmi istemci açılışta `limit=10`, yukarı kaydırınca
    /// `before=<en eski>&limit=20` kullanıyor. Dönüş en yeniden en eskiye sıralıdır.
    pub async fn get_messages(
        &self,
        page: Page,
        before: Option<Snowflake>,
        limit: u8,
    ) -> Result<Vec<Message>, HttpError> {
        let url = match before {
            Some(b) => format!(
                "{}/channels/{}/messages?before={b}&limit={limit}",
                self.base, page.channel
            ),
            None => format!(
                "{}/channels/{}/messages?limit={limit}",
                self.base, page.channel
            ),
        };
        let referer = page.referer();
        let resp = self
            .send(|| self.client.get(&url).header(header::REFERER, &referer))
            .await?;
        Ok(resp.json().await?)
    }

    /// İsteği gönderir; 429'da bekleyip yeniden dener, 401'i ayrı hata yapar.
    async fn send(
        &self,
        build: impl Fn() -> RequestBuilder,
    ) -> Result<reqwest::Response, HttpError> {
        for _ in 0..=MAX_RETRIES {
            let blocked = *self.blocked_until.lock().await;
            if let Some(until) = blocked {
                sleep_until(until).await;
            }
            let resp = build()
                .header(header::AUTHORIZATION, self.auth.clone())
                .send()
                .await?;
            match resp.status() {
                StatusCode::TOO_MANY_REQUESTS => {
                    let header_wait = retry_after_header(&resp);
                    let body: Option<RateLimitBody> = resp.json().await.ok();
                    let wait = body
                        .as_ref()
                        .map(|b| Duration::from_secs_f64(b.retry_after.max(0.0)))
                        .or(header_wait)
                        .unwrap_or(Duration::from_secs(1))
                        + RETRY_PAD;
                    let until = Instant::now() + wait;
                    tracing::warn!(
                        ?wait,
                        global = body.as_ref().is_some_and(|b| b.global),
                        "http: 429"
                    );
                    if body.is_some_and(|b| b.global) {
                        *self.blocked_until.lock().await = Some(until);
                    }
                    sleep_until(until).await;
                }
                StatusCode::UNAUTHORIZED => return Err(HttpError::Unauthorized),
                s if !s.is_success() => return Err(HttpError::Status(s.as_u16())),
                _ => return Ok(resp),
            }
        }
        Err(HttpError::RateLimited)
    }
}

fn name(n: &'static str) -> HeaderName {
    HeaderName::from_static(n)
}

fn retry_after_header(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<f64>()
        .ok()
        .map(|s| Duration::from_secs_f64(s.max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Sırayla verilen yanıtları dönen tek bağlantılık HTTP/1.1 sunucusu; gelen istekleri kaydeder.
    async fn server(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("adres"));
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            for resp in responses {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let mut buf = vec![0u8; 16 * 1024];
                let mut n = 0;
                while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    n += sock.read(&mut buf[n..]).await.expect("oku");
                }
                seen.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                sock.write_all(resp.as_bytes()).await.expect("yaz");
            }
            seen
        });
        (base, task)
    }

    fn reply(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn http(base: &str) -> Http {
        let token = Token::new("sahte-token").expect("token");
        let props = ClientProperties::chrome_linux("tr-TR", 155, 1);
        Http::new(&token, &props, "Europe/Istanbul")
            .expect("istemci")
            .with_base(base)
    }

    const PAGE: Page = Page {
        guild: Some(Snowflake(7)),
        channel: Snowflake(42),
    };
    const MSG: &str = r#"[{"id":"10","channel_id":"42","author":{"id":"1","username":"a","global_name":null},"content":"selam"}]"#;

    #[tokio::test]
    async fn sends_observed_headers_and_query() {
        let (base, srv) = server(vec![reply("200 OK", "", MSG)]).await;
        let msgs = http(&base)
            .get_messages(PAGE, None, 10)
            .await
            .expect("mesajlar");
        assert_eq!(msgs[0].content, "selam");
        let req = srv.await.expect("sunucu").remove(0).to_ascii_lowercase();
        assert!(
            req.starts_with("get /channels/42/messages?limit=10 "),
            "{req}"
        );
        for h in [
            "authorization: sahte-token",
            "user-agent: mozilla/5.0 (x11; linux x86_64)",
            "x-super-properties: ",
            "x-discord-locale: tr",
            "x-discord-timezone: europe/istanbul",
            "x-debug-options: bugreporterenabled",
            "referer: https://discord.com/channels/7/42",
            "sec-ch-ua: \"google chrome\";v=\"155\"",
            "sec-ch-ua-platform: \"linux\"",
            "accept-language: tr-tr,tr;q=0.9",
        ] {
            assert!(req.contains(h), "eksik başlık {h}:\n{req}");
        }
        assert!(!req.contains("bot sahte-token"));
    }

    #[tokio::test]
    async fn retries_after_429() {
        let (base, srv) = server(vec![
            reply(
                "429 Too Many Requests",
                "retry-after: 5\r\n",
                r#"{"retry_after":0.2,"global":false}"#,
            ),
            reply("200 OK", "", MSG),
        ])
        .await;
        let started = Instant::now();
        let msgs = http(&base)
            .get_messages(PAGE, Some(Snowflake(99)), 20)
            .await
            .expect("yeniden deneme sonrası başarı");
        assert_eq!(msgs.len(), 1);
        assert!(started.elapsed() >= Duration::from_millis(200));
        let reqs = srv.await.expect("sunucu");
        assert_eq!(reqs.len(), 2);
        assert!(reqs[1].starts_with("GET /channels/42/messages?before=99&limit=20 "));
    }

    #[tokio::test]
    async fn unauthorized_and_errors() {
        let (base, _srv) = server(vec![
            reply(
                "401 Unauthorized",
                "",
                r#"{"message":"401: Unauthorized","code":0}"#,
            ),
            reply(
                "403 Forbidden",
                "",
                r#"{"message":"Missing Access","code":50001}"#,
            ),
        ])
        .await;
        let h = http(&base);
        assert!(matches!(
            h.get_messages(PAGE, None, 10).await,
            Err(HttpError::Unauthorized)
        ));
        assert!(matches!(
            h.get_messages(PAGE, None, 10).await,
            Err(HttpError::Status(403))
        ));
    }

    #[test]
    fn dm_referer() {
        let p = Page {
            guild: None,
            channel: Snowflake(5),
        };
        assert_eq!(p.referer(), "https://discord.com/channels/@me/5");
    }
}
