//! Kullanıcı profili (`GET /users/{id}/profile`): açılır profil penceresinin verisi.
//!
//! Uç nokta kullanıcı istemcisine özgü ve belgesiz (PROTOCOL_NOTES §4.7; gözlenen yanıt
//! biçimi, doğrulanmadı). Resmi istemci alanları sürümler arasında ekleyip çıkardığından
//! ayrıştırma bilerek hoşgörülüdür: eksik ya da beklenmeyen türde alan varsayılana düşer,
//! yanıtın tamamı reddedilmez.

use serde_json::Value;

use crate::Snowflake;

/// Bir rozetin (ör. "Active Developer") açıklaması.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBadge {
    pub id: String,
    pub description: String,
}

/// Ortak sunucu: kimlik ve kişinin orada kullandığı takma ad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutualGuild {
    pub id: Snowflake,
    pub nick: Option<String>,
}

/// Bağlı hesap (`type`: `github`, `steam`...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedAccount {
    pub kind: String,
    pub name: String,
    pub verified: bool,
}

/// Ayrıştırılmış profil. Metin alanları boşsa `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserProfile {
    pub id: Option<Snowflake>,
    pub username: Option<String>,
    pub global_name: Option<String>,
    pub bot: bool,
    /// `user_profile.bio`, yoksa `user.bio`.
    pub bio: Option<String>,
    /// Sunucu profilindeki zamirler, yoksa genel profildeki.
    pub pronouns: Option<String>,
    pub badges: Vec<ProfileBadge>,
    pub mutual_guilds: Vec<MutualGuild>,
    /// Sunucunun bildirdiği sayı ile gelen liste uzunluğunun büyüğü.
    pub mutual_friends_count: u32,
    pub connected: Vec<ConnectedAccount>,
    /// Sorgudaki sunucudaki takma ad.
    pub guild_nick: Option<String>,
    /// Sorgudaki sunucudaki roller (DM'de ya da üye değilse boş).
    pub guild_roles: Vec<Snowflake>,
    /// Sunucuya katılma anı (ISO 8601 metni).
    pub joined_at: Option<String>,
    /// Nitro abonelik başlangıcı (ISO 8601 metni).
    pub premium_since: Option<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn objects<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| o.is_object())
}

fn snowflake(v: &Value) -> Option<Snowflake> {
    match v {
        Value::String(s) => s.parse().ok().map(Snowflake),
        Value::Number(n) => n.as_u64().map(Snowflake),
        _ => None,
    }
}

impl UserProfile {
    /// Yanıt gövdesini ayrıştırır; gövde bir nesne değilse `None`.
    pub fn from_json(body: &Value) -> Option<Self> {
        if !body.is_object() {
            return None;
        }
        let null = Value::Null;
        let user = body.get("user").unwrap_or(&null);
        let profile = body.get("user_profile").unwrap_or(&null);
        let member = body.get("guild_member").unwrap_or(&null);
        let member_profile = body.get("guild_member_profile").unwrap_or(&null);
        let listed = u32::try_from(
            body.get("mutual_friends")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        )
        .unwrap_or(u32::MAX);
        let reported = body
            .get("mutual_friends_count")
            .and_then(Value::as_u64)
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
        Some(Self {
            id: user.get("id").and_then(snowflake),
            username: text(user, "username"),
            global_name: text(user, "global_name"),
            bot: user.get("bot").and_then(Value::as_bool).unwrap_or(false),
            bio: text(profile, "bio").or_else(|| text(user, "bio")),
            pronouns: text(member_profile, "pronouns").or_else(|| text(profile, "pronouns")),
            badges: objects(body, "badges")
                .filter_map(|b| {
                    Some(ProfileBadge {
                        id: text(b, "id").unwrap_or_default(),
                        description: text(b, "description")?,
                    })
                })
                .collect(),
            mutual_guilds: objects(body, "mutual_guilds")
                .filter_map(|g| {
                    Some(MutualGuild {
                        id: g.get("id").and_then(snowflake)?,
                        nick: text(g, "nick"),
                    })
                })
                .collect(),
            mutual_friends_count: reported.max(listed),
            connected: objects(body, "connected_accounts")
                .filter_map(|c| {
                    Some(ConnectedAccount {
                        kind: text(c, "type").unwrap_or_default(),
                        name: text(c, "name")?,
                        verified: c.get("verified").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect(),
            guild_nick: text(member, "nick"),
            guild_roles: member
                .get("roles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(snowflake)
                .collect(),
            joined_at: text(member, "joined_at"),
            premium_since: text(body, "premium_since"),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Gözlenen biçimde, uydurma değerlerle.
    fn full() -> Value {
        json!({
            "user": {"id": "100", "username": "ornek", "global_name": "Örnek Kişi",
                     "avatar": "abc", "bio": "eski bio", "accent_color": 123, "banner_color": "#fff"},
            "connected_accounts": [
                {"type": "github", "id": "1", "name": "ornek-gh", "verified": true},
                {"type": "steam", "id": "2", "name": "ornek-steam", "verified": false},
                {"type": "x"}
            ],
            "premium_since": "2022-01-02T03:04:05.000000+00:00",
            "user_profile": {"bio": "merhaba\ndünya", "accent_color": 1, "pronouns": "o/o"},
            "badges": [{"id": "active_developer", "description": "Aktif Geliştirici", "icon": "i"}],
            "mutual_friends": [{"id": "5"}, {"id": "6"}],
            "mutual_friends_count": 0,
            "mutual_guilds": [{"id": "10", "nick": "Takma"}, {"id": "11", "nick": null}, {"nick": "x"}],
            "guild_member": {"nick": "Sunucu Adı", "roles": ["7", "8"], "joined_at": "2023-05-06T07:08:09.123000+00:00"},
            "guild_member_profile": {"guild_id": "10", "pronouns": "she/her"}
        })
    }

    #[test]
    fn parses_observed_shape() {
        let p = UserProfile::from_json(&full()).expect("nesne");
        assert_eq!(p.id, Some(Snowflake(100)));
        assert_eq!(p.username.as_deref(), Some("ornek"));
        assert_eq!(p.global_name.as_deref(), Some("Örnek Kişi"));
        assert_eq!(p.bio.as_deref(), Some("merhaba\ndünya"));
        assert_eq!(p.pronouns.as_deref(), Some("she/her"), "sunucu zamiri önde");
        assert_eq!(p.badges.len(), 1);
        assert_eq!(p.badges[0].description, "Aktif Geliştirici");
        assert_eq!(p.mutual_guilds.len(), 2, "kimliksiz öğe atlanır");
        assert_eq!(p.mutual_guilds[0].nick.as_deref(), Some("Takma"));
        assert_eq!(p.mutual_guilds[1].nick, None);
        assert_eq!(p.mutual_friends_count, 2, "liste uzunluğu sayıyı aşarsa o");
        assert_eq!(p.connected.len(), 2, "adsız hesap atlanır");
        assert!(p.connected[0].verified && !p.connected[1].verified);
        assert_eq!(p.guild_nick.as_deref(), Some("Sunucu Adı"));
        assert_eq!(p.guild_roles, vec![Snowflake(7), Snowflake(8)]);
        assert!(
            p.joined_at
                .as_deref()
                .is_some_and(|s| s.starts_with("2023"))
        );
        assert!(p.premium_since.is_some());
        assert!(!p.bot);
    }

    #[test]
    fn tolerates_missing_and_odd_fields() {
        let p = UserProfile::from_json(&json!({})).expect("boş nesne geçerli");
        assert_eq!(p, UserProfile::default());
        let p = UserProfile::from_json(&json!({
            "user": {"id": 5, "bio": "  "}, "badges": "yok", "mutual_guilds": [1, "a"],
            "guild_member": {"roles": [1, "x", "9"]}, "mutual_friends_count": "çok"
        }))
        .expect("nesne");
        assert_eq!(p.id, Some(Snowflake(5)), "sayı kimlik de kabul");
        assert_eq!(p.bio, None, "boşluk bio yok sayılır");
        assert!(p.badges.is_empty() && p.mutual_guilds.is_empty());
        assert_eq!(p.guild_roles, vec![Snowflake(1), Snowflake(9)]);
        assert_eq!(p.mutual_friends_count, 0);
        assert!(UserProfile::from_json(&json!([1])).is_none());
        assert!(UserProfile::from_json(&Value::Null).is_none());
    }

    #[test]
    fn bio_falls_back_to_user_bio() {
        let p = UserProfile::from_json(
            &json!({"user": {"bio": "x"}, "user_profile": {"pronouns": "p"}}),
        )
        .expect("nesne");
        assert_eq!(p.bio.as_deref(), Some("x"));
        assert_eq!(p.pronouns.as_deref(), Some("p"));
    }
}
