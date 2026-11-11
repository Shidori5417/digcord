//! Discord CDN adres biçimleri (belgeli, "Image Formatting"; PROTOCOL_NOTES §6).
//!
//! Hepsi `cdn.discordapp.com` altındadır ([`crate::ALLOWED_HOSTS`]). Hareketli (`a_`) karmalar
//! için de `.png` istenir: ilk kare yeter, hareketli GIF indirilmez.

/// İstenen resim kenarı (piksel); 2×4 hücrelik yer için yeterli, küçük indirme.
pub const SIZE: u32 = 64;

const CDN: &str = "https://cdn.discordapp.com";

/// Varsayılan avatar indeksi (yeni kullanıcı adı sistemi): `(id >> 22) % 6`.
pub fn default_avatar_index(user_id: u64) -> u64 {
    (user_id >> 22) % 6
}

/// Kullanıcı avatarı; karma yoksa varsayılan avatar.
pub fn user_avatar(user_id: u64, hash: Option<&str>) -> String {
    match hash.filter(|h| !h.is_empty()) {
        Some(h) => format!("{CDN}/avatars/{user_id}/{h}.png?size={SIZE}"),
        None => format!("{CDN}/embed/avatars/{}.png", default_avatar_index(user_id)),
    }
}

/// Sunucuya özel üye avatarı.
pub fn member_avatar(guild_id: u64, user_id: u64, hash: &str) -> String {
    format!("{CDN}/guilds/{guild_id}/users/{user_id}/avatars/{hash}.png?size={SIZE}")
}

/// Sunucu ikonu.
pub fn guild_icon(guild_id: u64, hash: &str) -> String {
    format!("{CDN}/icons/{guild_id}/{hash}.png?size={SIZE}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowed;

    #[test]
    fn avatar_urls() {
        assert_eq!(
            user_avatar(42, Some("abc")),
            "https://cdn.discordapp.com/avatars/42/abc.png?size=64"
        );
        // Hareketli karma da .png (ilk kare).
        assert_eq!(
            user_avatar(42, Some("a_abc")),
            "https://cdn.discordapp.com/avatars/42/a_abc.png?size=64"
        );
        assert_eq!(
            user_avatar(1 << 22, None),
            "https://cdn.discordapp.com/embed/avatars/1.png"
        );
        assert_eq!(user_avatar(7, Some("")), user_avatar(7, None));
    }

    #[test]
    fn default_index_wraps_at_six() {
        assert_eq!(default_avatar_index(0), 0);
        assert_eq!(default_avatar_index(5 << 22), 5);
        assert_eq!(default_avatar_index(6 << 22), 0);
        assert_eq!(default_avatar_index(13 << 22), 1);
    }

    #[test]
    fn member_and_icon_urls_are_allowed() {
        let m = member_avatar(1, 2, "h");
        assert_eq!(
            m,
            "https://cdn.discordapp.com/guilds/1/users/2/avatars/h.png?size=64"
        );
        let i = guild_icon(1, "h");
        assert_eq!(i, "https://cdn.discordapp.com/icons/1/h.png?size=64");
        for u in [m, i, user_avatar(3, None), user_avatar(3, Some("x"))] {
            assert!(allowed(&reqwest::Url::parse(&u).expect("url")));
        }
    }
}
