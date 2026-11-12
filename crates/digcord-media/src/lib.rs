//! CDN indirme, resim çözme/ölçekleme, disk önbelleği.
//!
//! Sadece Discord CDN'ine istek atılır (`ALLOWED_HOSTS`); embed'lerdeki rastgele dış adreslere
//! değil, Discord'un vekil (`proxy_url`) adreslerine gidilir. CDN isteklerinde token yoktur.

pub mod cdn;
mod lru;

use std::path::{Path, PathBuf};
use std::time::Duration;

use image::{DynamicImage, ImageReader, Limits};
use reqwest::Url;
use tokio::sync::Semaphore;

pub use lru::Lru;

/// İstek atılabilecek tek adresler: CDN, ek vekili ve embed resim vekilleri.
pub const ALLOWED_HOSTS: [&str; 4] = [
    "cdn.discordapp.com",
    "media.discordapp.net",
    "images-ext-1.discordapp.net",
    "images-ext-2.discordapp.net",
];
/// Aynı anda en fazla bu kadar CDN isteği.
const CONCURRENCY: usize = 4;
/// Bundan büyük yanıtlar indirilmez (önizlemeler vekil üzerinden küçültülmüş gelir).
const MAX_BYTES: usize = 16 * 1024 * 1024;
/// URL imzası: süreleri dolar ama içerik aynı kalır; önbellek anahtarına girmez.
const SIGNATURE_PARAMS: [&str; 3] = ["ex", "is", "hm"];
/// Çözülen resim için bellek ve boyut sınırları (sıkıştırma bombasına karşı).
const MAX_ALLOC: u64 = 128 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("izin verilmeyen adres")]
    NotAllowed,
    #[error("HTTP {0}")]
    Status(u16),
    #[error("istek başarısız: {0}")]
    Http(#[from] reqwest::Error),
    #[error("dosya çok büyük")]
    TooLarge,
    #[error("resim çözülemedi: {0}")]
    Decode(#[from] image::ImageError),
}

impl MediaError {
    /// İmzalı URL'nin süresi dolmuş olabilir; mesaj yeniden çekilirse yeni URL gelir.
    pub fn maybe_expired(&self) -> bool {
        matches!(self, Self::Status(403 | 404 | 410))
    }
}

/// `https` ve izin verilen bir Discord CDN adresi mi.
pub fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))
}

/// Disk önbelleği dosya adı: imza parametreleri atılmış URL'nin FNV-1a özeti.
pub fn cache_name(url: &Url) -> String {
    let mut h = Fnv::default();
    h.write(url.host_str().unwrap_or_default().as_bytes());
    h.write(url.path().as_bytes());
    for (k, v) in url.query_pairs() {
        if !SIGNATURE_PARAMS.contains(&k.as_ref()) {
            h.write(b"&");
            h.write(k.as_bytes());
            h.write(b"=");
            h.write(v.as_bytes());
        }
    }
    format!("{:016x}", h.0)
}

/// FNV-1a 64; sürümler arası sabit (std'nin hasher'ı değil), kriptografik değil.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// CDN istemcisi: eşzamanlılık sınırı ve disk önbelleği.
pub struct Fetcher {
    client: reqwest::Client,
    limit: Semaphore,
    cache: Option<PathBuf>,
}

impl Fetcher {
    /// `cache`: ham dosyaların dizini (`None` → disk önbelleği yok). Dizin yoksa oluşturulur.
    pub fn new(user_agent: &str, cache: Option<PathBuf>) -> Result<Self, MediaError> {
        // reqwest `rustls-no-provider`: süreç genelinde ring (zaten kuruluysa hata yok sayılır).
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            // Yönlendirme de sadece izinli adreslere.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 5 && allowed(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()?;
        let cache = cache.filter(|dir| match create_private_dir(dir) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("medya önbelleği kullanılamıyor: {e}");
                false
            }
        });
        Ok(Self {
            client,
            limit: Semaphore::new(CONCURRENCY),
            cache,
        })
    }

    /// Dosyayı önce disk önbelleğinden, yoksa CDN'den getirir.
    pub async fn get(&self, url: &str) -> Result<Vec<u8>, MediaError> {
        let url = Url::parse(url).map_err(|_| MediaError::NotAllowed)?;
        if !allowed(&url) {
            return Err(MediaError::NotAllowed);
        }
        let path = self.cache.as_ref().map(|d| d.join(cache_name(&url)));
        if let Some(p) = &path
            && let Ok(bytes) = tokio::fs::read(p).await
        {
            return Ok(bytes);
        }
        let bytes = {
            // Semaphore hiç kapatılmaz; hata imkânsız ama yine de istek atmadan dönülür.
            let _permit = self
                .limit
                .acquire()
                .await
                .map_err(|_| MediaError::TooLarge)?;
            self.download(url).await?
        };
        if let Some(p) = &path
            && let Err(e) = write_atomic(p, &bytes).await
        {
            tracing::debug!("medya önbelleğe yazılamadı: {e}");
        }
        Ok(bytes)
    }

    async fn download(&self, url: Url) -> Result<Vec<u8>, MediaError> {
        let mut resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(MediaError::Status(status.as_u16()));
        }
        if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
            return Err(MediaError::TooLarge);
        }
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if out.len() + chunk.len() > MAX_BYTES {
                return Err(MediaError::TooLarge);
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Yarım dosya kalmasın diye geçici dosyaya yazıp yeniden adlandırır.
async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

/// Önbellek `max_bytes`'ı aşıyorsa en eski dosyaları siler. Engelleyici; açılışta arka planda.
pub fn prune(dir: &Path, max_bytes: u64) -> std::io::Result<()> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            files.push((modified, meta.len(), entry.path()));
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        std::fs::remove_file(&path)?;
        total -= len;
    }
    Ok(())
}

/// Resmi çözer (hareketli biçimlerde ilk kare) ve `max_w`×`max_h` pikselin içine sığdırır.
/// Ağır iş: UI thread'inde çağrılmamalı.
pub fn decode_fit(bytes: &[u8], max_w: u32, max_h: u32) -> Result<DynamicImage, MediaError> {
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(image::ImageError::IoError)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    reader.limits(limits);
    let img = reader.decode()?;
    let (max_w, max_h) = (max_w.max(1), max_h.max(1));
    Ok(if img.width() > max_w || img.height() > max_h {
        img.resize(max_w, max_h, image::imageops::FilterType::Triangle)
    } else {
        img
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    #[test]
    fn only_discord_cdn_over_https() {
        assert!(allowed(&url("https://cdn.discordapp.com/emojis/1.png")));
        assert!(allowed(&url(
            "https://media.discordapp.net/attachments/1/2/a.png?width=10"
        )));
        assert!(allowed(&url(
            "https://images-ext-1.discordapp.net/external/abc/https/x.example/a.png"
        )));
        assert!(!allowed(&url("http://cdn.discordapp.com/emojis/1.png")));
        assert!(!allowed(&url("https://cdn.discordapp.com:8443/a.png")));
        assert!(!allowed(&url("https://evil.example/a.png")));
        assert!(!allowed(&url(
            "https://cdn.discordapp.com.evil.example/a.png"
        )));
        assert!(!allowed(&url("https://127.0.0.1/a.png")));
    }

    #[test]
    fn cache_name_ignores_signature_but_not_size() {
        let a = url("https://media.discordapp.net/attachments/1/2/a.png?ex=1&is=2&hm=3&width=10");
        let b = url("https://media.discordapp.net/attachments/1/2/a.png?ex=9&is=8&hm=7&width=10");
        let c = url("https://media.discordapp.net/attachments/1/2/a.png?ex=1&is=2&hm=3&width=20");
        assert_eq!(cache_name(&a), cache_name(&b));
        assert_ne!(cache_name(&a), cache_name(&c));
        assert_eq!(cache_name(&a).len(), 16);
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = DynamicImage::new_rgba8(w, h);
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .expect("png yaz");
        out.into_inner()
    }

    #[test]
    fn decode_fits_and_keeps_aspect() {
        let img = decode_fit(&png(400, 200), 100, 100).expect("çöz");
        assert_eq!((img.width(), img.height()), (100, 50));
        // Küçük resim büyütülmez.
        let img = decode_fit(&png(20, 10), 100, 100).expect("çöz");
        assert_eq!((img.width(), img.height()), (20, 10));
    }

    #[test]
    fn decode_rejects_garbage_and_huge() {
        assert!(decode_fit(b"not an image", 10, 10).is_err());
        assert!(decode_fit(&png(MAX_DIMENSION + 1, 1), 10, 10).is_err());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("digcord-media-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn serves_from_disk_cache_without_network() {
        let dir = temp_dir("hit");
        let fetcher = Fetcher::new("test", Some(dir.clone())).expect("fetcher");
        let u = "https://cdn.discordapp.com/emojis/1.png?size=48";
        std::fs::write(dir.join(cache_name(&url(u))), b"cached").expect("yaz");
        assert_eq!(fetcher.get(u).await.expect("önbellek"), b"cached");
        assert!(matches!(
            fetcher.get("https://evil.example/a.png").await,
            Err(MediaError::NotAllowed)
        ));
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&dir).expect("meta").permissions(),
        );
        assert_eq!(mode & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_removes_oldest_first() {
        let dir = temp_dir("prune");
        create_private_dir(&dir).expect("dizin");
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            let p = dir.join(name);
            std::fs::write(&p, [0u8; 10]).expect("yaz");
            let t = std::time::UNIX_EPOCH + Duration::from_secs(1_000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .and_then(|f| f.set_modified(t))
                .expect("mtime");
        }
        prune(&dir, 20).expect("prune");
        assert!(!dir.join("a").exists());
        assert!(dir.join("b").exists() && dir.join("c").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CDN isteği kimlik taşımaz: `Authorization`, `X-Super-Properties`, çerez ve diğer
    /// `x-discord-*` başlıkları gitmez; yalnızca User-Agent ve standart HTTP başlıkları gider.
    #[tokio::test]
    async fn cdn_request_carries_no_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("adres");
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.expect("oku");
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("yaz");
            String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
        });
        let fetcher = Fetcher::new("UA-test", None).expect("fetcher");
        // İzin listesi `get`te uygulanır; başlıklar `download`da belirlenir.
        let bytes = fetcher
            .download(url(&format!("http://{addr}/a.png")))
            .await
            .expect("indir");
        assert_eq!(bytes, b"ok");
        let request = server.await.expect("sunucu");
        assert!(request.contains("user-agent: ua-test"), "{request}");
        for forbidden in [
            "authorization",
            "x-super-properties",
            "x-discord",
            "x-debug-options",
            "x-installation-id",
            "cookie",
        ] {
            assert!(
                !request.contains(forbidden),
                "{forbidden} gitmemeli:\n{request}"
            );
        }
    }
}
