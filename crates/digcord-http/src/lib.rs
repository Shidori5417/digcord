//! REST istemcisi ve rate limit yönetimi.
//!
//! Tüm istekler resmi web istemcisinin başlıklarını taşır (PROTOCOL_NOTES §4.1); başlık
//! değerleri sadece [`ClientProperties`]'ten üretilir. Rate limit mantığı [`ratelimit`]
//! modülünde: rota bazlı bucket, global kilit, 429'da en fazla 3 yeniden deneme.

pub mod ratelimit;

use std::fmt::Write as _;
use std::time::Duration;

use digcord_model::{
    ClientProperties, Message, ReactionEmoji, SearchPage, SearchQuery, Snowflake, ThreadList,
    Token, UserProfile,
};
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use ratelimit::{Limits, RateLimiter, secs_to_duration};

pub const API_BASE: &str = "https://discord.com/api/v9";
/// Aynı istek için en fazla bu kadar 429 sonrası yeniden deneme.
const MAX_RETRIES: u32 = 3;
/// Discord'un söylediğinden fazla bekleme (sunucu tarafı yuvarlama için pay).
const RETRY_PAD: Duration = Duration::from_millis(50);

/// Thread listesi isteklerinde sayfa boyutu.
pub const THREAD_PAGE: u8 = 25;

/// Forum aktif gönderileri için sorgu yolu (son mesaja göre azalan).
pub fn forum_threads_path(forum: Snowflake, limit: u8) -> String {
    format!(
        "/channels/{forum}/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit={limit}"
    )
}

/// Arşivli herkese açık thread'ler için yol.
pub fn archived_threads_path(channel: Snowflake, limit: u8) -> String {
    format!("/channels/{channel}/threads/archived/public?limit={limit}")
}

/// Açılır profil isteğinin yolu (sorgu dahil): `GET /users/{id}/profile?...`. Resmi web
/// istemcisinde gözlenen sorgu (2026-11-11); `guild_id` DM'de yok (doğrulanmadı,
/// PROTOCOL_NOTES §4.7).
pub fn profile_path(user: Snowflake, guild: Option<Snowflake>) -> String {
    let mut path = format!(
        "/users/{user}/profile?type=popout&with_mutual_guilds=true&with_mutual_friends=true&with_mutual_friends_count=false"
    );
    if let Some(g) = guild {
        let _ = write!(path, "&guild_id={g}");
    }
    path
}

/// Aramanın kapsamı: bir sunucu ya da tek bir (DM) kanal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    Guild(Snowflake),
    Channel(Snowflake),
}

/// Arama yolu (sorgu dahil): `GET /guilds/{g}/messages/search?...` ya da
/// `GET /channels/{c}/messages/search?...` (belgesiz, doğrulanmadı: PROTOCOL_NOTES §4.5).
pub fn search_path(scope: SearchScope, query: &SearchQuery, offset: u32) -> String {
    let mut path = match scope {
        SearchScope::Guild(g) => format!("/guilds/{g}/messages/search?"),
        SearchScope::Channel(c) => format!("/channels/{c}/messages/search?"),
    };
    let mut first = true;
    let mut param = |path: &mut String, k: &str, v: &str| {
        if !first {
            path.push('&');
        }
        first = false;
        path.push_str(k);
        path.push('=');
        path.push_str(v);
    };
    if !query.content.is_empty() {
        param(&mut path, "content", &percent_encode(&query.content));
    }
    if let Some(a) = query.author {
        param(&mut path, "author_id", &a.to_string());
    }
    if let Some(c) = query.channel {
        param(&mut path, "channel_id", &c.to_string());
    }
    for h in &query.has {
        param(&mut path, "has", h.as_param());
    }
    param(&mut path, "offset", &offset.to_string());
    path
}

/// Arama isteğinin sonucu.
#[derive(Debug)]
pub enum SearchOutcome {
    Results(SearchPage),
    /// 202: arama dizini hazır değil. Otomatik yeniden denenmez; kullanıcı tekrar dener.
    NotReady {
        retry_after: Option<Duration>,
    },
}

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
    /// Yükleme adresi `https://` değil (adres, imza içerebileceğinden hataya konmaz).
    #[error("güvensiz yükleme adresi")]
    InsecureUploadUrl,
    #[error("beklenmeyen yanıt biçimi")]
    BadResponse,
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

/// Gönderilecek mesaj.
pub struct NewMessage<'a> {
    pub content: &'a str,
    pub nonce: Snowflake,
    pub reply_to: Option<Snowflake>,
    pub attachments: Vec<UploadedAttachment>,
}

/// [`Http::upload_attachment`] sonucu; [`NewMessage::attachments`] içine konur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedAttachment {
    pub id: u32,
    pub filename: String,
    pub uploaded_filename: String,
}

pub struct Http {
    client: reqwest::Client,
    base: String,
    auth: HeaderValue,
    /// Sadece Discord API isteklerine eklenen başlıklar (harici yükleme adresine gitmez).
    api_headers: HeaderMap,
    limiter: RateLimiter,
}

/// `Referer` sayfa bilgisi olmayan istekler için (PROTOCOL_NOTES §3.2).
const DEFAULT_REFERER: &str = "https://discord.com/channels/@me";

#[derive(Deserialize)]
struct RateLimitBody {
    retry_after: f64,
    #[serde(default)]
    global: bool,
}

#[derive(Deserialize)]
struct AttachmentSlots {
    attachments: Vec<AttachmentSlot>,
}

#[derive(Deserialize)]
struct AttachmentSlot {
    id: u32,
    upload_url: String,
    upload_filename: String,
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

        // Resmi istemcinin her API isteğinde gönderdiği ek başlıklar (PROTOCOL_NOTES §3.2).
        // `origin` GET'lerde gözlenmedi; eklenmiyor.
        let mut api = HeaderMap::new();
        api.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        api.insert(name("priority"), HeaderValue::from_static("u=1, i"));
        api.insert(name("sec-fetch-dest"), HeaderValue::from_static("empty"));
        api.insert(name("sec-fetch-mode"), HeaderValue::from_static("cors"));
        api.insert(
            name("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );
        if let Some(id) = props.installation_id() {
            api.insert(name("x-installation-id"), value(id)?);
        }

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
            api_headers: api,
            limiter: RateLimiter::new(),
        })
    }

    /// `GET /experiments` → `installation` alanı (kalıcı kurulum kimliği). Resmi istemci bunu
    /// kimlik doğrulamasız çağırdı; `Authorization` eklenmez. Doğrulanmadı: PROTOCOL_NOTES §3.2.
    pub async fn fetch_installation_id(&self) -> Result<String, HttpError> {
        let resp = self
            .send_with(Method::GET, "/experiments", DEFAULT_REFERER, None, false)
            .await?;
        let body: Value = resp.json().await.map_err(|_| HttpError::BadResponse)?;
        parse_installation(&body).ok_or(HttpError::BadResponse)
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
        let path = match before {
            Some(b) => format!(
                "/channels/{}/messages?before={b}&limit={limit}",
                page.channel
            ),
            None => format!("/channels/{}/messages?limit={limit}", page.channel),
        };
        let resp = self.send(Method::GET, &path, page, None).await?;
        Ok(resp.json().await?)
    }

    /// `GET /channels/{id}/messages?around=<id>&limit=<n>` (belgeli): `around` çevresindeki
    /// mesajlar, en yeniden en eskiye.
    pub async fn get_messages_around(
        &self,
        page: Page,
        around: Snowflake,
        limit: u8,
    ) -> Result<Vec<Message>, HttpError> {
        let path = format!(
            "/channels/{}/messages?around={around}&limit={limit}",
            page.channel
        );
        let resp = self.send(Method::GET, &path, page, None).await?;
        Ok(resp.json().await?)
    }

    /// Kullanıcı tarafından başlatılan tek arama isteği. 202 yanıtı [`SearchOutcome::NotReady`].
    pub async fn search_messages(
        &self,
        page: Page,
        scope: SearchScope,
        query: &SearchQuery,
        offset: u32,
    ) -> Result<SearchOutcome, HttpError> {
        let path = search_path(scope, query, offset);
        let resp = self.send(Method::GET, &path, page, None).await?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(|_| HttpError::BadResponse)?;
        if status == StatusCode::ACCEPTED {
            let retry_after = digcord_model::parse_retry_after(&body).and_then(secs_to_duration);
            return Ok(SearchOutcome::NotReady { retry_after });
        }
        digcord_model::parse_search_response(&body)
            .map(SearchOutcome::Results)
            .ok_or(HttpError::BadResponse)
    }

    /// `GET /channels/{id}/pins` (belgeli): sabitlenmiş mesajlar. Yeni sayfalı biçim de
    /// ayrıştırılır (doğrulanmadı, PROTOCOL_NOTES §4.5).
    pub async fn pins(&self, page: Page) -> Result<Vec<Message>, HttpError> {
        let path = format!("/channels/{}/pins", page.channel);
        let resp = self.send(Method::GET, &path, page, None).await?;
        let body: Value = resp.json().await.map_err(|_| HttpError::BadResponse)?;
        digcord_model::parse_pins(&body).ok_or(HttpError::BadResponse)
    }

    /// Kullanıcı eylemiyle (üyeye tıklama) açılan profil: [`profile_path`]. `Referer` sunucu
    /// sayfasıdır (kanal bilinmez; resmi istemcinin tam değeri doğrulanmadı).
    pub async fn user_profile(
        &self,
        user: Snowflake,
        guild: Option<Snowflake>,
    ) -> Result<UserProfile, HttpError> {
        let path = profile_path(user, guild);
        let referer = match guild {
            Some(g) => format!("https://discord.com/channels/{g}"),
            None => DEFAULT_REFERER.to_owned(),
        };
        let resp = self
            .send_with(Method::GET, &path, &referer, None, true)
            .await?;
        let body: Value = resp.json().await.map_err(|_| HttpError::BadResponse)?;
        UserProfile::from_json(&body).ok_or(HttpError::BadResponse)
    }

    /// Forum/medya kanalının aktif gönderileri: `GET /channels/{f}/threads/search?...`
    /// (kullanıcı istemcisi uç noktası, belgelenmemiş: doğrulanmadı, PROTOCOL_NOTES §4.4).
    pub async fn search_forum_threads(&self, page: Page) -> Result<ThreadList, HttpError> {
        let path = forum_threads_path(page.channel, THREAD_PAGE);
        let resp = self.send(Method::GET, &path, page, None).await?;
        Ok(resp.json().await?)
    }

    /// Arşivli herkese açık thread'ler: `GET /channels/{c}/threads/archived/public` (belgeli).
    /// Yalnızca ilk sayfa; `has_more` dönüşte bildirilir.
    pub async fn archived_threads(&self, page: Page) -> Result<ThreadList, HttpError> {
        let path = archived_threads_path(page.channel, THREAD_PAGE);
        let resp = self.send(Method::GET, &path, page, None).await?;
        Ok(resp.json().await?)
    }

    /// `POST /channels/{c}/messages`. Yanıt oluşan mesajdır.
    pub async fn create_message(
        &self,
        page: Page,
        msg: &NewMessage<'_>,
    ) -> Result<Message, HttpError> {
        // doğrulanmadı (PROTOCOL_NOTES): alanlar Discord'un herkese açık belgelerinden.
        let mut body = json!({
            "mobile_network_type": "unknown",
            "content": msg.content,
            "nonce": msg.nonce,
            "tts": false,
            "flags": 0,
        });
        if let Some(reply) = msg.reply_to {
            let mut reference = json!({
                "channel_id": page.channel,
                "message_id": reply,
            });
            if let (Some(g), Some(map)) = (page.guild, reference.as_object_mut()) {
                map.insert("guild_id".into(), json!(g));
            }
            body["message_reference"] = reference;
            body["allowed_mentions"] =
                json!({"parse": ["users", "roles", "everyone"], "replied_user": true});
        }
        if !msg.attachments.is_empty() {
            body["type"] = json!(0);
            body["sticker_ids"] = json!([]);
            body["attachments"] = msg
                .attachments
                .iter()
                .map(|a| {
                    json!({
                        "id": a.id.to_string(),
                        "filename": a.filename,
                        "uploaded_filename": a.uploaded_filename,
                    })
                })
                .collect();
        }
        let path = format!("/channels/{}/messages", page.channel);
        let resp = self.send(Method::POST, &path, page, Some(body)).await?;
        Ok(resp.json().await?)
    }

    /// `PATCH /channels/{c}/messages/{m}`.
    pub async fn edit_message(
        &self,
        page: Page,
        message: Snowflake,
        content: &str,
    ) -> Result<Message, HttpError> {
        // doğrulanmadı (PROTOCOL_NOTES)
        let body = json!({ "content": content });
        let path = format!("/channels/{}/messages/{message}", page.channel);
        let resp = self.send(Method::PATCH, &path, page, Some(body)).await?;
        Ok(resp.json().await?)
    }

    /// `DELETE /channels/{c}/messages/{m}` (204).
    pub async fn delete_message(&self, page: Page, message: Snowflake) -> Result<(), HttpError> {
        let path = format!("/channels/{}/messages/{message}", page.channel);
        self.send(Method::DELETE, &path, page, None).await?;
        Ok(())
    }

    /// `PUT .../reactions/{emoji}/@me?location=Message&type=0` (204).
    pub async fn add_reaction(
        &self,
        page: Page,
        message: Snowflake,
        emoji: &ReactionEmoji,
    ) -> Result<(), HttpError> {
        // doğrulanmadı (PROTOCOL_NOTES): sorgu parametreleri topluluk bilgisi.
        let path = format!(
            "/channels/{}/messages/{message}/reactions/{}/@me?location=Message&type=0",
            page.channel,
            percent_encode(&emoji.api_form())
        );
        self.send(Method::PUT, &path, page, None).await?;
        Ok(())
    }

    /// `DELETE .../reactions/{emoji}/0/@me?location=Message&burst=false` (204).
    pub async fn remove_reaction(
        &self,
        page: Page,
        message: Snowflake,
        emoji: &ReactionEmoji,
    ) -> Result<(), HttpError> {
        // doğrulanmadı (PROTOCOL_NOTES)
        let path = format!(
            "/channels/{}/messages/{message}/reactions/{}/0/@me?location=Message&burst=false",
            page.channel,
            percent_encode(&emoji.api_form())
        );
        self.send(Method::DELETE, &path, page, None).await?;
        Ok(())
    }

    /// `POST /channels/{c}/messages/{m}/ack` gövde `{"token":null}` — okundu işareti.
    pub async fn ack(&self, page: Page, message: Snowflake) -> Result<(), HttpError> {
        // doğrulanmadı (PROTOCOL_NOTES): resmi istemci `last_viewed` de gönderiyor olabilir.
        let path = format!("/channels/{}/messages/{message}/ack", page.channel);
        self.send(Method::POST, &path, page, Some(json!({ "token": null })))
            .await?;
        Ok(())
    }

    /// `POST /channels/{c}/typing` (204, gövdesiz).
    pub async fn typing(&self, page: Page) -> Result<(), HttpError> {
        let path = format!("/channels/{}/typing", page.channel);
        self.send(Method::POST, &path, page, None).await?;
        Ok(())
    }

    /// İki adımlı yükleme: `POST /channels/{c}/attachments` ile yer ayrılır, dönen
    /// `upload_url`'e ham baytlar `PUT` edilir. Sonuç [`NewMessage::attachments`]'a konur.
    pub async fn upload_attachment(
        &self,
        page: Page,
        filename: &str,
        bytes: Vec<u8>,
    ) -> Result<UploadedAttachment, HttpError> {
        // doğrulanmadı (PROTOCOL_NOTES)
        let body = json!({
            "files": [{
                "file_size": bytes.len(),
                "filename": filename,
                "id": "0",
                "is_clip": false,
            }]
        });
        let path = format!("/channels/{}/attachments", page.channel);
        let resp = self.send(Method::POST, &path, page, Some(body)).await?;
        let slots: AttachmentSlots = resp.json().await?;
        let slot = slots
            .attachments
            .into_iter()
            .next()
            .ok_or(HttpError::BadResponse)?;
        if !upload_url_allowed(&self.base, &slot.upload_url) {
            return Err(HttpError::InsecureUploadUrl);
        }
        // Harici depolama sunucusu: Authorization ASLA gönderilmez (istemcinin varsayılan
        // başlıkları token içermez; token sadece `send()` içinde eklenir).
        let put = self
            .client
            .put(&slot.upload_url)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes)
            .send()
            .await?;
        if !put.status().is_success() {
            return Err(HttpError::Status(put.status().as_u16()));
        }
        Ok(UploadedAttachment {
            id: slot.id,
            filename: filename.to_owned(),
            uploaded_filename: slot.upload_filename,
        })
    }

    /// İsteği rate limit kurallarıyla gönderir; 429'da bekleyip yeniden dener, 401'i ayrı
    /// hata yapar. `path` taban adresten sonrası (sorgu dahil).
    async fn send(
        &self,
        method: Method,
        path: &str,
        page: Page,
        body: Option<Value>,
    ) -> Result<reqwest::Response, HttpError> {
        self.send_with(method, path, &page.referer(), body, true)
            .await
    }

    /// `authed: false` ise `Authorization` eklenmez.
    async fn send_with(
        &self,
        method: Method,
        path: &str,
        referer: &str,
        body: Option<Value>,
        authed: bool,
    ) -> Result<reqwest::Response, HttpError> {
        let route = ratelimit::route_key(method.as_str(), path);
        let url = format!("{}{path}", self.base);
        // Aynı bucket'taki istekler sırayla gider; koruyucu istek bitene kadar tutulur.
        let _slot = self.limiter.acquire(&route).await;
        for _ in 0..=MAX_RETRIES {
            self.limiter.wait(&route).await;
            let mut req = self
                .client
                .request(method.clone(), &url)
                .headers(self.api_headers.clone())
                .header(header::REFERER, referer);
            if authed {
                req = req.header(header::AUTHORIZATION, self.auth.clone());
            }
            if let Some(b) = &body {
                req = req.json(b);
            } else if matches!(method, Method::POST | Method::PUT | Method::PATCH) {
                // reqwest gövdesiz POST/PUT'ta content-length yollamaz; resmi istemci 0 yollar.
                req = req.header(header::CONTENT_LENGTH, "0");
            }
            let resp = req.send().await?;
            let limits = Limits::from_headers(resp.headers());
            self.limiter.record(&route, &limits, Instant::now());
            match resp.status() {
                StatusCode::TOO_MANY_REQUESTS => {
                    let parsed: Option<RateLimitBody> = resp.json().await.ok();
                    let wait = parsed
                        .as_ref()
                        .and_then(|b| secs_to_duration(b.retry_after))
                        .or(limits.retry_after)
                        .unwrap_or(Duration::from_secs(1))
                        + RETRY_PAD;
                    let global = limits.global || parsed.is_some_and(|b| b.global);
                    tracing::warn!(%route, ?wait, global, "http: 429");
                    self.limiter
                        .record_429(&route, wait, global, Instant::now());
                }
                StatusCode::UNAUTHORIZED => return Err(HttpError::Unauthorized),
                s if !s.is_success() => return Err(HttpError::Status(s.as_u16())),
                _ => return Ok(resp),
            }
        }
        Err(HttpError::RateLimited)
    }
}

/// `/experiments` yanıtından `installation` değeri; başlığa ve dosyaya güvenle konabilecek
/// biçimde (ASCII harf/rakam ve `.`, `-`, `_`; en çok 128 karakter) değilse `None`.
pub fn parse_installation(body: &Value) -> Option<String> {
    let id = body.get("installation")?.as_str()?;
    valid_installation_id(id).then(|| id.to_owned())
}

/// Kurulum kimliği biçimi: ASCII harf/rakam ve `.`, `-`, `_`; 1..=128 karakter.
pub fn valid_installation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn name(n: &'static str) -> HeaderName {
    HeaderName::from_static(n)
}

/// Yükleme adresi yalnızca `https://` olabilir; test tabanı (`http://127.0.0.1`) için
/// yerel `http://127.0.0.1` adresine izin verilir.
fn upload_url_allowed(base: &str, url: &str) -> bool {
    const LOCAL: &str = "http://127.0.0.1";
    url.starts_with("https://") || (base.starts_with(LOCAL) && url.starts_with(LOCAL))
}

/// Yol parçası için yüzde kodlama: UTF-8 baytları, `A-Za-z0-9-._~` ve `:` (özel emoji) hariç.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b':') {
            out.push(char::from(b));
        } else {
            // String'e yazma hata vermez.
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn bind() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("adres"));
        (listener, base)
    }

    /// Sırayla verilen yanıtları dönen HTTP/1.1 sunucusu (istek başına bağlantı); gelen
    /// istekleri (başlık + gövde) kaydeder.
    fn serve(
        listener: TcpListener,
        responses: Vec<String>,
    ) -> tokio::task::JoinHandle<Vec<String>> {
        tokio::spawn(async move {
            let mut seen = Vec::new();
            for resp in responses {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = sock.read(&mut chunk).await.expect("oku");
                    assert!(n > 0, "bağlantı erken kapandı");
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                seen.push(String::from_utf8_lossy(&buf).into_owned());
                sock.write_all(resp.as_bytes()).await.expect("yaz");
            }
            seen
        })
    }

    async fn server(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let (listener, base) = bind().await;
        (base, serve(listener, responses))
    }

    fn reply(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn no_content() -> String {
        "HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n".to_owned()
    }

    fn http(base: &str) -> Http {
        let token = Token::new("sahte-token").expect("token");
        let props = ClientProperties::chrome_linux("tr-TR", 155, 1);
        Http::new(&token, &props, "Europe/Istanbul")
            .expect("istemci")
            .with_base(base)
    }

    /// İstek metninden JSON gövdeyi ayrıştırır.
    fn body_json(req: &str) -> Value {
        let (_, body) = req.split_once("\r\n\r\n").expect("gövde ayracı");
        serde_json::from_str(body).expect("json gövde")
    }

    fn lower(req: &str) -> String {
        req.to_ascii_lowercase()
    }

    const PAGE: Page = Page {
        guild: Some(Snowflake(7)),
        channel: Snowflake(42),
    };
    const MSG: &str = r#"[{"id":"10","channel_id":"42","author":{"id":"1","username":"a","global_name":null},"content":"selam"}]"#;
    const ONE: &str = r#"{"id":"10","channel_id":"42","author":{"id":"1","username":"a","global_name":null},"content":"selam"}"#;

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
            "accept: */*",
            "priority: u=1, i",
            "sec-fetch-dest: empty",
            "sec-fetch-mode: cors",
            "sec-fetch-site: same-origin",
        ] {
            assert!(req.contains(h), "eksik başlık {h}:\n{req}");
        }
        assert!(!req.contains("bot sahte-token"));
        assert!(!req.contains("\r\norigin:"), "GET'te origin yok:\n{req}");
        assert!(
            !req.contains("x-installation-id"),
            "bilinmiyorsa gönderilmez"
        );
    }

    #[tokio::test]
    async fn installation_header_and_referer_per_page() {
        let (base, srv) = server(vec![reply("200 OK", "", "[]"), reply("200 OK", "", "[]")]).await;
        let token = Token::new("sahte-token").expect("token");
        let mut props = ClientProperties::chrome_linux("tr-TR", 155, 1);
        props.set_installation_id(Some("111.abcDEF_-".into()));
        let h = Http::new(&token, &props, "Europe/Istanbul")
            .expect("istemci")
            .with_base(base);
        h.get_messages(PAGE, None, 10).await.expect("sunucu kanalı");
        let dm = Page {
            guild: None,
            channel: Snowflake(5),
        };
        h.get_messages(dm, None, 10).await.expect("dm");
        let seen = srv.await.expect("sunucu");
        let a = lower(&seen[0]);
        let b = lower(&seen[1]);
        assert!(a.contains("x-installation-id: 111.abcdef_-"), "{a}");
        assert!(
            a.contains("referer: https://discord.com/channels/7/42\r\n"),
            "{a}"
        );
        assert!(
            b.contains("referer: https://discord.com/channels/@me/5\r\n"),
            "{b}"
        );
    }

    #[tokio::test]
    async fn upload_put_has_no_api_headers() {
        let (listener, base) = bind().await;
        let slots = format!(
            r#"{{"attachments":[{{"id":0,"upload_url":"{base}/up","upload_filename":"x/a.png"}}]}}"#
        );
        let srv = serve(listener, vec![reply("200 OK", "", &slots), no_content()]);
        let mut props = ClientProperties::chrome_linux("tr-TR", 155, 1);
        props.set_installation_id(Some("1.a".into()));
        let token = Token::new("sahte-token").expect("token");
        let h = Http::new(&token, &props, "Europe/Istanbul")
            .expect("istemci")
            .with_base(base);
        h.upload_attachment(PAGE, "a.png", vec![1, 2, 3])
            .await
            .expect("yükleme");
        let seen = srv.await.expect("sunucu");
        let put = lower(&seen[1]);
        assert!(put.starts_with("put /up "), "{put}");
        for h in [
            "x-installation-id",
            "sec-fetch-site",
            "authorization",
            "referer",
        ] {
            assert!(!put.contains(h), "harici PUT'ta {h} olmamalı:\n{put}");
        }
    }

    #[tokio::test]
    async fn fetch_installation_id_is_unauthenticated() {
        let body = r#"{"fingerprint":"f","assignments":[],"installation":"123456.AbC-d_E"}"#;
        let (base, srv) = server(vec![reply("200 OK", "", body)]).await;
        let id = http(&base).fetch_installation_id().await.expect("kimlik");
        assert_eq!(id, "123456.AbC-d_E");
        let req = lower(&srv.await.expect("sunucu")[0]);
        assert!(req.starts_with("get /experiments "), "{req}");
        assert!(!req.contains("authorization"), "{req}");
        assert!(!req.contains("sahte-token"));
        assert!(
            req.contains("referer: https://discord.com/channels/@me\r\n"),
            "{req}"
        );
    }

    #[test]
    fn installation_parsing_rejects_odd_values() {
        let ok = json!({"installation":"1.a-b_C"});
        assert_eq!(parse_installation(&ok).as_deref(), Some("1.a-b_C"));
        for bad in [
            json!({}),
            json!({"installation":""}),
            json!({"installation":7}),
            json!({"installation":"a b"}),
            json!({"installation":"a\r\nx: y"}),
            json!({"installation":"é"}),
            json!({"installation":"x".repeat(129)}),
        ] {
            assert!(parse_installation(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn thread_paths() {
        assert_eq!(
            forum_threads_path(Snowflake(5), 25),
            "/channels/5/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25"
        );
        assert_eq!(
            archived_threads_path(Snowflake(5), 25),
            "/channels/5/threads/archived/public?limit=25"
        );
        // Sorgu dizesi rate limit rotasını bölmez.
        assert_eq!(
            ratelimit::route_key("GET", &forum_threads_path(Snowflake(5), 25)),
            "GET /channels/5/threads/search"
        );
    }

    #[tokio::test]
    async fn fetches_forum_and_archived_threads() {
        let body = r#"{"threads":[{"id":"60","type":11,"parent_id":"42","name":"gönderi","thread_metadata":{"archived":false,"locked":false}}],"members":[],"has_more":true,"total_results":3,"first_messages":[]}"#;
        let (base, srv) = server(vec![
            reply("200 OK", "", body),
            reply(
                "200 OK",
                "",
                r#"{"threads":[],"members":[],"has_more":false}"#,
            ),
        ])
        .await;
        let h = http(&base);
        let list = h.search_forum_threads(PAGE).await.expect("forum");
        assert_eq!(list.threads[0].id, Snowflake(60));
        assert!(list.has_more);
        let list = h.archived_threads(PAGE).await.expect("arşiv");
        assert!(list.threads.is_empty());
        let reqs = srv.await.expect("sunucu");
        assert!(
            reqs[0].starts_with("GET /channels/42/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25 "),
            "{}",
            reqs[0]
        );
        assert!(reqs[1].starts_with("GET /channels/42/threads/archived/public?limit=25 "));
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
    async fn retries_with_bucket_headers_then_succeeds() {
        let (base, srv) = server(vec![
            reply(
                "429 Too Many Requests",
                "x-ratelimit-bucket: bk1\r\nx-ratelimit-limit: 5\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset-after: 0.2\r\nx-ratelimit-scope: user\r\n",
                r#"{"message":"You are being rate limited.","retry_after":0.2,"global":false}"#,
            ),
            reply("200 OK", "", ONE),
        ])
        .await;
        let started = Instant::now();
        let m = http(&base)
            .edit_message(PAGE, Snowflake(10), "yeni")
            .await
            .expect("başarı");
        assert_eq!(m.content, "selam");
        assert!(started.elapsed() >= Duration::from_millis(200));
        let reqs = srv.await.expect("sunucu");
        assert_eq!(reqs.len(), 2);
        assert_eq!(body_json(&reqs[1])["content"], "yeni");
    }

    #[tokio::test]
    async fn gives_up_after_max_retries() {
        let one = reply(
            "429 Too Many Requests",
            "",
            r#"{"retry_after":0.01,"global":false}"#,
        );
        let (base, srv) = server(vec![one; 4]).await;
        let err = http(&base).typing(PAGE).await.expect_err("vazgeçmeli");
        assert!(matches!(err, HttpError::RateLimited));
        assert_eq!(srv.await.expect("sunucu").len(), 4);
    }

    #[tokio::test]
    async fn remaining_zero_delays_next_request() {
        let ok =
            |extra: &str| format!("HTTP/1.1 204 No Content\r\nconnection: close\r\n{extra}\r\n");
        let (base, srv) = server(vec![
            ok("x-ratelimit-bucket: t\r\nx-ratelimit-limit: 1\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset-after: 0.3\r\n"),
            ok(""),
        ])
        .await;
        let h = http(&base);
        let started = Instant::now();
        h.typing(PAGE).await.expect("ilk");
        h.typing(PAGE).await.expect("ikinci");
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert_eq!(srv.await.expect("sunucu").len(), 2);
    }

    #[tokio::test]
    async fn global_429_blocks_other_routes() {
        let (base, srv) = server(vec![
            reply(
                "429 Too Many Requests",
                "",
                r#"{"retry_after":0.25,"global":true}"#,
            ),
            no_content(),
            no_content(),
        ])
        .await;
        let h = http(&base);
        let started = Instant::now();
        h.typing(PAGE).await.expect("typing");
        assert!(started.elapsed() >= Duration::from_millis(250));
        // başka rota (farklı kanal) global engel bittiği için hemen gider
        let other = Page {
            guild: None,
            channel: Snowflake(43),
        };
        h.typing(other).await.expect("typing 2");
        assert_eq!(srv.await.expect("sunucu").len(), 3);
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

    #[tokio::test]
    async fn create_message_plain() {
        let (base, srv) = server(vec![reply("200 OK", "", ONE)]).await;
        let msg = NewMessage {
            content: "merhaba",
            nonce: Snowflake(1234),
            reply_to: None,
            attachments: vec![],
        };
        let m = http(&base)
            .create_message(PAGE, &msg)
            .await
            .expect("gönder");
        assert_eq!(m.content, "selam");
        let req = srv.await.expect("sunucu").remove(0);
        assert!(req.starts_with("POST /channels/42/messages "), "{req}");
        let l = lower(&req);
        assert!(l.contains("authorization: sahte-token"));
        assert!(l.contains("referer: https://discord.com/channels/7/42"));
        assert!(l.contains("content-type: application/json"));
        let b = body_json(&req);
        assert_eq!(b["content"], "merhaba");
        assert_eq!(b["nonce"], "1234");
        assert_eq!(b["tts"], false);
        assert_eq!(b["flags"], 0);
        assert_eq!(b["mobile_network_type"], "unknown");
        assert!(b.get("message_reference").is_none());
        assert!(b.get("allowed_mentions").is_none());
        assert!(b.get("attachments").is_none());
    }

    #[tokio::test]
    async fn create_message_reply_with_attachment() {
        let (base, srv) = server(vec![reply("200 OK", "", ONE)]).await;
        let msg = NewMessage {
            content: "cevap",
            nonce: Snowflake(5),
            reply_to: Some(Snowflake(99)),
            attachments: vec![UploadedAttachment {
                id: 0,
                filename: "a.png".into(),
                uploaded_filename: "cdn/xyz/a.png".into(),
            }],
        };
        http(&base)
            .create_message(PAGE, &msg)
            .await
            .expect("gönder");
        let b = body_json(&srv.await.expect("sunucu").remove(0));
        assert_eq!(
            b["message_reference"],
            json!({"guild_id":"7","channel_id":"42","message_id":"99"})
        );
        assert_eq!(
            b["allowed_mentions"],
            json!({"parse":["users","roles","everyone"],"replied_user":true})
        );
        assert_eq!(b["type"], 0);
        assert_eq!(b["sticker_ids"], json!([]));
        assert_eq!(
            b["attachments"],
            json!([{"id":"0","filename":"a.png","uploaded_filename":"cdn/xyz/a.png"}])
        );
    }

    #[tokio::test]
    async fn create_message_dm_reply_has_no_guild_id() {
        let (base, srv) = server(vec![reply("200 OK", "", ONE)]).await;
        let dm = Page {
            guild: None,
            channel: Snowflake(42),
        };
        let msg = NewMessage {
            content: "x",
            nonce: Snowflake(5),
            reply_to: Some(Snowflake(99)),
            attachments: vec![],
        };
        http(&base).create_message(dm, &msg).await.expect("gönder");
        let b = body_json(&srv.await.expect("sunucu").remove(0));
        assert_eq!(
            b["message_reference"],
            json!({"channel_id":"42","message_id":"99"})
        );
    }

    #[tokio::test]
    async fn edit_and_delete_message() {
        let (base, srv) = server(vec![reply("200 OK", "", ONE), no_content()]).await;
        let h = http(&base);
        h.edit_message(PAGE, Snowflake(10), "düzenli")
            .await
            .expect("düzenle");
        h.delete_message(PAGE, Snowflake(10)).await.expect("sil");
        let reqs = srv.await.expect("sunucu");
        assert!(reqs[0].starts_with("PATCH /channels/42/messages/10 "));
        assert_eq!(body_json(&reqs[0]), json!({"content":"düzenli"}));
        assert!(reqs[1].starts_with("DELETE /channels/42/messages/10 "));
        for r in &reqs {
            assert!(lower(r).contains("authorization: sahte-token"));
        }
    }

    #[tokio::test]
    async fn reactions_encode_emoji() {
        let (base, srv) = server(vec![no_content(), no_content(), no_content()]).await;
        let h = http(&base);
        let fire = ReactionEmoji::unicode("🔥");
        let custom = ReactionEmoji {
            id: Some(Snowflake(555)),
            name: Some("pepe".into()),
            animated: false,
        };
        h.add_reaction(PAGE, Snowflake(10), &fire)
            .await
            .expect("ekle");
        h.remove_reaction(PAGE, Snowflake(10), &fire)
            .await
            .expect("kaldır");
        h.add_reaction(PAGE, Snowflake(10), &custom)
            .await
            .expect("özel ekle");
        let reqs = srv.await.expect("sunucu");
        assert!(
            reqs[0].starts_with(
                "PUT /channels/42/messages/10/reactions/%F0%9F%94%A5/@me?location=Message&type=0 "
            ),
            "{}",
            reqs[0]
        );
        assert!(
            reqs[1].starts_with(
                "DELETE /channels/42/messages/10/reactions/%F0%9F%94%A5/0/@me?location=Message&burst=false "
            ),
            "{}",
            reqs[1]
        );
        assert!(
            reqs[2].starts_with(
                "PUT /channels/42/messages/10/reactions/pepe:555/@me?location=Message&type=0 "
            ),
            "{}",
            reqs[2]
        );
        assert!(lower(&reqs[0]).contains("authorization: sahte-token"));
    }

    #[tokio::test]
    async fn typing_has_empty_body() {
        let (base, srv) = server(vec![no_content()]).await;
        http(&base).typing(PAGE).await.expect("typing");
        let req = srv.await.expect("sunucu").remove(0);
        assert!(req.starts_with("POST /channels/42/typing "), "{req}");
        let l = lower(&req);
        assert!(l.contains("content-length: 0"), "{req}");
        assert!(l.contains("authorization: sahte-token"));
        assert!(req.ends_with("\r\n\r\n"));
    }

    #[tokio::test]
    async fn ack_posts_null_token() {
        let (base, srv) = server(vec![reply("200 OK", "", r#"{"token":null}"#)]).await;
        http(&base).ack(PAGE, Snowflake(77)).await.expect("ack");
        let req = srv.await.expect("sunucu").remove(0);
        assert!(
            req.starts_with("POST /channels/42/messages/77/ack "),
            "{req}"
        );
        assert!(req.ends_with(r#"{"token":null}"#), "{req}");
    }

    #[tokio::test]
    async fn upload_two_step_flow() {
        let (listener, base) = bind().await;
        let slots = format!(
            r#"{{"attachments":[{{"id":0,"upload_url":"{base}/upload/abc?sig=1","upload_filename":"cdn/xyz/a.txt"}}]}}"#
        );
        let srv = serve(
            listener,
            vec![
                reply("200 OK", "", &slots),
                "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned(),
                reply("200 OK", "", ONE),
            ],
        );
        let h = http(&base);
        let up = h
            .upload_attachment(PAGE, "a.txt", b"HAM-BAYTLAR".to_vec())
            .await
            .expect("yükle");
        assert_eq!(
            up,
            UploadedAttachment {
                id: 0,
                filename: "a.txt".into(),
                uploaded_filename: "cdn/xyz/a.txt".into()
            }
        );
        let msg = NewMessage {
            content: "",
            nonce: Snowflake(8),
            reply_to: None,
            attachments: vec![up],
        };
        h.create_message(PAGE, &msg).await.expect("mesaj");

        let reqs = srv.await.expect("sunucu");
        // 1) yer ayırma
        assert!(
            reqs[0].starts_with("POST /channels/42/attachments "),
            "{}",
            reqs[0]
        );
        assert!(lower(&reqs[0]).contains("authorization: sahte-token"));
        assert_eq!(
            body_json(&reqs[0]),
            json!({"files":[{"file_size":11,"filename":"a.txt","id":"0","is_clip":false}]})
        );
        // 2) ham PUT: Authorization yok, gövde ham baytlar
        assert!(reqs[1].starts_with("PUT /upload/abc?sig=1 "), "{}", reqs[1]);
        let l = lower(&reqs[1]);
        assert!(!l.contains("authorization"), "{}", reqs[1]);
        assert!(!l.contains("sahte-token"));
        assert!(l.contains("content-type: application/octet-stream"));
        assert!(reqs[1].ends_with("\r\n\r\nHAM-BAYTLAR"), "{}", reqs[1]);
        // 3) mesaj ek bilgisini taşır
        assert_eq!(
            body_json(&reqs[2])["attachments"][0]["uploaded_filename"],
            "cdn/xyz/a.txt"
        );
    }

    #[tokio::test]
    async fn upload_rejects_insecure_url() {
        // test tabanı olsa bile harici http adresi reddedilir
        let slots = r#"{"attachments":[{"id":0,"upload_url":"http://example.com/u","upload_filename":"f"}]}"#;
        let (base, _srv) = server(vec![reply("200 OK", "", slots)]).await;
        let err = http(&base)
            .upload_attachment(PAGE, "a", vec![1])
            .await
            .expect_err("reddedilmeli");
        assert!(matches!(err, HttpError::InsecureUploadUrl));
    }

    #[test]
    fn upload_url_policy() {
        assert!(upload_url_allowed(API_BASE, "https://cdn.example/x"));
        assert!(!upload_url_allowed(API_BASE, "http://cdn.example/x"));
        assert!(!upload_url_allowed(API_BASE, "http://127.0.0.1:1/x"));
        assert!(upload_url_allowed(
            "http://127.0.0.1:9",
            "http://127.0.0.1:1/x"
        ));
        assert!(!upload_url_allowed(
            "http://127.0.0.1:9",
            "http://evil.test/x"
        ));
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode("🔥"), "%F0%9F%94%A5");
        assert_eq!(percent_encode("pepe:555"), "pepe:555");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(percent_encode("A-z_0.~"), "A-z_0.~");
    }
    #[test]
    fn search_paths_encode_and_filter() {
        use digcord_model::Has;
        let q = SearchQuery {
            content: "çay & şeker #1".into(),
            author: Some(Snowflake(5)),
            channel: Some(Snowflake(9)),
            has: vec![Has::Link, Has::Image],
        };
        assert_eq!(
            search_path(SearchScope::Guild(Snowflake(7)), &q, 25),
            "/guilds/7/messages/search?content=%C3%A7ay%20%26%20%C5%9Feker%20%231&author_id=5&channel_id=9&has=link&has=image&offset=25"
        );
        let only_has = SearchQuery {
            has: vec![Has::File],
            ..SearchQuery::default()
        };
        assert_eq!(
            search_path(SearchScope::Channel(Snowflake(3)), &only_has, 0),
            "/channels/3/messages/search?has=file&offset=0"
        );
        assert_eq!(
            ratelimit::route_key("GET", &search_path(SearchScope::Guild(Snowflake(7)), &q, 0)),
            "GET /guilds/7/messages/search"
        );
    }

    #[tokio::test]
    async fn search_returns_hits_and_total() {
        let body = format!(r#"{{"total_results":31,"messages":[[{ONE}],[{ONE}]]}}"#);
        let (base, srv) = server(vec![reply("200 OK", "", &body)]).await;
        let q = SearchQuery {
            content: "selam".into(),
            ..SearchQuery::default()
        };
        let out = http(&base)
            .search_messages(PAGE, SearchScope::Guild(Snowflake(7)), &q, 25)
            .await
            .expect("arama");
        let SearchOutcome::Results(page) = out else {
            panic!("sonuç bekleniyordu");
        };
        assert_eq!(page.total, 31);
        assert_eq!(page.hits.len(), 2);
        let req = srv.await.expect("sunucu").remove(0);
        assert!(
            req.starts_with("GET /guilds/7/messages/search?content=selam&offset=25 "),
            "{req}"
        );
    }

    #[tokio::test]
    async fn search_202_is_not_ready_without_retry_loop() {
        // Tek yanıt hazırlanır: otomatik yeniden deneme olsaydı sunucu bağlantı beklerdi.
        let (base, srv) = server(vec![reply(
            "202 Accepted",
            "",
            r#"{"message":"Index not yet available. Try again later","code":110000,"retry_after":2}"#,
        )])
        .await;
        let q = SearchQuery {
            content: "x".into(),
            ..SearchQuery::default()
        };
        let out = http(&base)
            .search_messages(PAGE, SearchScope::Channel(Snowflake(42)), &q, 0)
            .await
            .expect("202 hata değil");
        match out {
            SearchOutcome::NotReady { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(2)));
            }
            SearchOutcome::Results(_) => panic!("hazır değil bekleniyordu"),
        }
        assert_eq!(srv.await.expect("sunucu").len(), 1);
    }

    #[tokio::test]
    async fn search_bad_body_is_error() {
        let (base, _srv) = server(vec![reply("200 OK", "", r#"{"foo":1}"#)]).await;
        let q = SearchQuery::default();
        let err = http(&base)
            .search_messages(PAGE, SearchScope::Channel(Snowflake(42)), &q, 0)
            .await
            .expect_err("bozuk gövde");
        assert!(matches!(err, HttpError::BadResponse));
    }

    #[tokio::test]
    async fn fetches_around_and_pins() {
        let (base, srv) = server(vec![reply("200 OK", "", MSG), reply("200 OK", "", MSG)]).await;
        let h = http(&base);
        let msgs = h
            .get_messages_around(PAGE, Snowflake(10), 50)
            .await
            .expect("around");
        assert_eq!(msgs.len(), 1);
        let pins = h.pins(PAGE).await.expect("pins");
        assert_eq!(pins.len(), 1);
        let reqs = srv.await.expect("sunucu");
        assert!(reqs[0].starts_with("GET /channels/42/messages?around=10&limit=50 "));
        assert!(reqs[1].starts_with("GET /channels/42/pins "));
    }

    #[test]
    fn profile_path_has_guild_only_in_servers() {
        assert_eq!(
            profile_path(Snowflake(7), Some(Snowflake(9))),
            "/users/7/profile?type=popout&with_mutual_guilds=true&with_mutual_friends=true&with_mutual_friends_count=false&guild_id=9"
        );
        assert_eq!(
            profile_path(Snowflake(7), None),
            "/users/7/profile?type=popout&with_mutual_guilds=true&with_mutual_friends=true&with_mutual_friends_count=false"
        );
    }

    #[tokio::test]
    async fn fetches_profile_with_server_referer() {
        let body = r#"{"user":{"id":"7","username":"u"},"guild_member":{"roles":["3"]}}"#;
        let (base, srv) = server(vec![reply("200 OK", "", body), reply("200 OK", "", "[]")]).await;
        let h = http(&base);
        let p = h
            .user_profile(Snowflake(7), Some(Snowflake(9)))
            .await
            .expect("profil");
        assert_eq!(p.username.as_deref(), Some("u"));
        assert_eq!(p.guild_roles, vec![Snowflake(3)]);
        assert!(matches!(
            h.user_profile(Snowflake(7), None).await,
            Err(HttpError::BadResponse)
        ));
        let reqs = srv.await.expect("sunucu");
        assert!(reqs[0].starts_with("GET /users/7/profile?type=popout"));
        assert!(reqs[0].contains("&guild_id=9 "));
        assert!(lower(&reqs[0]).contains("referer: https://discord.com/channels/9\r\n"));
        assert!(lower(&reqs[1]).contains("referer: https://discord.com/channels/@me\r\n"));
    }
}
