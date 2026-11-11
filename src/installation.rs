//! Kalıcı kurulum kimliği (`installation_id` / `X-Installation-ID`).
//!
//! Resmi istemci bunu `GET /experiments` yanıtındaki `installation` alanından alıp kalıcı
//! saklıyor (PROTOCOL_NOTES §3.2; eşleme doğrulanmadı). İlk açılışta bir kez çekilir,
//! `state_dir()/digcord/installation_id` dosyasına (0600) yazılır, sonraki açılışlarda okunur.
//! Kimlik gizli değildir ama başkasıyla paylaşılmaz. Başarısızlıkta kimliksiz devam edilir.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use digcord_http::{Http, valid_installation_id};
use digcord_model::{ClientProperties, Token};

/// Varsayılan dosya yolu; platformda durum dizini yoksa `None`.
pub fn default_path() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("digcord").join("installation_id"))
}

/// Dosyadaki kimlik; yok ya da biçimi bozuksa `None`.
pub fn load_from(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.trim();
    valid_installation_id(id).then(|| id.to_owned())
}

/// Kimliği dosyaya yazar (dizin yoksa oluşturulur; dosya modu 0600).
pub fn save_to(path: &Path, id: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // Dosya önceden daha açık izinlerle varsa da kıs.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(id.as_bytes())
}

/// Kayıtlı kimliği okur; yoksa `/experiments`'tan bir kez çekip kaydeder. Başarısızlıkta
/// `None` (uyarı loglanır; token loglanmaz).
pub async fn resolve(token: &Token, props: &ClientProperties, timezone: &str) -> Option<String> {
    let path = default_path();
    if let Some(p) = path.clone() {
        match tokio::task::spawn_blocking(move || load_from(&p)).await {
            Ok(Some(id)) => return Some(id),
            Ok(None) => {}
            Err(e) => tracing::warn!("installation_id okunamadı: {e}"),
        }
    }
    let http = match Http::new(token, props, timezone) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("installation_id için istemci kurulamadı: {e}");
            return None;
        }
    };
    let id = match http.fetch_installation_id().await {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("installation_id alınamadı (kimliksiz devam): {e}");
            return None;
        }
    };
    if let Some(p) = path {
        let to_save = id.clone();
        match tokio::task::spawn_blocking(move || save_to(&p, &to_save)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("installation_id kaydedilemedi: {e}"),
            Err(e) => tracing::warn!("installation_id kaydedilemedi: {e}"),
        }
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "digcord-inst-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&d).expect("geçici dizin");
        d
    }

    #[test]
    fn save_then_load_roundtrip_with_private_mode() {
        let dir = temp_dir();
        let path = dir.join("alt").join("installation_id");
        assert_eq!(load_from(&path), None);
        save_to(&path, "123.AbC-d_E").expect("kaydet");
        assert_eq!(load_from(&path).as_deref(), Some("123.AbC-d_E"));
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // Üzerine yazma kısaltır; satır sonu tolere edilir.
        save_to(&path, "9.x").expect("kaydet");
        assert_eq!(load_from(&path).as_deref(), Some("9.x"));
        std::fs::write(&path, "5.y\n").expect("yaz");
        assert_eq!(load_from(&path).as_deref(), Some("5.y"));
        std::fs::remove_dir_all(&dir).expect("temizle");
    }

    #[test]
    fn bad_file_content_is_ignored() {
        let dir = temp_dir();
        let path = dir.join("installation_id");
        for bad in ["", "a b", "a\r\nx: y", "ü"] {
            std::fs::write(&path, bad).expect("yaz");
            assert_eq!(load_from(&path), None, "{bad:?}");
        }
        std::fs::remove_dir_all(&dir).expect("temizle");
    }
}
