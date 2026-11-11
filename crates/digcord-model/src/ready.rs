//! READY olayının ve ilgili nesnelerin kullanılan alt kümesi. Bilinmeyen alanlar yok sayılır.

use serde::Deserialize;

use crate::{Overwrite, Role, Snowflake};

#[derive(Debug, Deserialize)]
pub struct Ready {
    pub v: u8,
    pub user: User,
    pub session_id: String,
    pub resume_gateway_url: String,
    #[serde(default)]
    pub guilds: Vec<Guild>,
    #[serde(default)]
    pub private_channels: Vec<Channel>,
    /// DM alıcıları gibi READY'de id ile anılan kullanıcılar.
    #[serde(default)]
    pub users: Vec<User>,
    /// `guilds` ile aynı sırada; her biri bu kullanıcının o guild'deki üyeliği(leri).
    #[serde(default)]
    pub merged_members: Vec<Vec<Member>>,
    #[serde(default)]
    pub read_state: Entries<ReadStateEntry>,
    #[serde(default)]
    pub user_guild_settings: Entries<GuildSettings>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
}

/// `{"version":..,"partial":..,"entries":[..]}` sarmalayıcısı.
#[derive(Debug, Deserialize)]
#[serde(bound = "T: Deserialize<'de>")]
pub struct Entries<T> {
    #[serde(default)]
    pub entries: Vec<T>,
}

impl<T> Default for Entries<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

/// Kanal okunma durumu. `read_state_type` dolu olanlar kanal değil (bildirim merkezi vb.).
#[derive(Debug, Clone, Deserialize)]
pub struct ReadStateEntry {
    pub id: Snowflake,
    pub last_message_id: Option<Snowflake>,
    #[serde(default)]
    pub mention_count: u32,
    pub read_state_type: Option<u8>,
}

/// Sunucu (veya `guild_id == None` → DM) bildirim ayarları.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuildSettings {
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub muted: bool,
    pub mute_config: Option<MuteConfig>,
    /// 0 tüm mesajlar, 1 sadece bahsetmeler, 2 hiçbiri, 3 üst ayara uy.
    #[serde(default)]
    pub message_notifications: u8,
    #[serde(default)]
    pub suppress_everyone: bool,
    #[serde(default)]
    pub suppress_roles: bool,
    #[serde(default)]
    pub channel_overrides: Vec<ChannelOverride>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelOverride {
    pub channel_id: Snowflake,
    #[serde(default)]
    pub muted: bool,
    pub mute_config: Option<MuteConfig>,
    #[serde(default = "inherit")]
    pub message_notifications: u8,
}

fn inherit() -> u8 {
    3
}

/// Süreli sessize alma; `end_time` ISO 8601 (null = süresiz).
#[derive(Debug, Clone, Deserialize)]
pub struct MuteConfig {
    pub end_time: Option<String>,
}

/// 1 arkadaş, 2 engelli, 3 gelen istek, 4 giden istek.
#[derive(Debug, Clone, Deserialize)]
pub struct Relationship {
    /// Karşı kullanıcının id'si.
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: u8,
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct User {
    pub id: Snowflake,
    pub username: String,
    pub global_name: Option<String>,
}

impl User {
    /// Gösterilecek ad: global ad, yoksa kullanıcı adı.
    pub fn display_name(&self) -> &str {
        self.global_name.as_deref().unwrap_or(&self.username)
    }
}

/// Guild üyeliği. READY `merged_members`'ta `user_id`, olaylarda `user` gelir.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Member {
    pub user_id: Option<Snowflake>,
    pub user: Option<User>,
    pub nick: Option<String>,
    #[serde(default)]
    pub roles: Vec<Snowflake>,
}

/// Kullanıcı istemcisi biçimi: ad, ikon vb. `properties` alt nesnesinde.
#[derive(Debug, Deserialize)]
pub struct Guild {
    pub id: Snowflake,
    #[serde(default)]
    pub unavailable: bool,
    pub properties: Option<GuildProperties>,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub emojis: Vec<CustomEmoji>,
    /// Büyük guild (op 37 her guild için aynı tam biçimle gönderilir; PROTOCOL_NOTES §4.1).
    #[serde(default)]
    pub large: bool,
    /// Ses kanallarındaki kullanıcılar (`guild_id` alanı burada yok).
    #[serde(default)]
    pub voice_states: Vec<crate::VoiceState>,
    /// Kullanıcının görebildiği aktif thread'ler (kullanıcı hesabı READY biçimi
    /// doğrulanmadı, PROTOCOL_NOTES §4.4).
    #[serde(default)]
    pub threads: Vec<Channel>,
}

/// Sunucunun özel emojisi.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomEmoji {
    pub id: Snowflake,
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    #[serde(default = "yes")]
    pub available: bool,
}

fn yes() -> bool {
    true
}

/// `GUILD_MEMBERS_CHUNK` (op 8 yanıtı).
#[derive(Debug, Deserialize)]
pub struct GuildMembersChunk {
    pub guild_id: Snowflake,
    #[serde(default)]
    pub members: Vec<Member>,
}

#[derive(Debug, Deserialize)]
pub struct GuildProperties {
    pub name: String,
    pub owner_id: Option<Snowflake>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: u8,
    /// READY içindeki guild kanallarında yok; kanal olaylarında var.
    pub guild_id: Option<Snowflake>,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub parent_id: Option<Snowflake>,
    pub position: Option<i32>,
    pub last_message_id: Option<Snowflake>,
    #[serde(default)]
    pub permission_overwrites: Vec<Overwrite>,
    /// DM / grup DM alıcıları.
    #[serde(default)]
    pub recipient_ids: Vec<Snowflake>,
    /// Thread'i açan kullanıcı.
    pub owner_id: Option<Snowflake>,
    /// Sadece thread'lerde.
    pub thread_metadata: Option<ThreadMetadata>,
    pub message_count: Option<u32>,
    pub member_count: Option<u32>,
    /// Forum / medya kanalındaki etiketler (kanal üzerinde).
    #[serde(default)]
    pub available_tags: Vec<ForumTag>,
    /// Thread'e uygulanmış etiket id'leri.
    #[serde(default)]
    pub applied_tags: Vec<Snowflake>,
    /// Sadece thread'lerde ve yalnızca benim üyeliğim varsa dolu (doğrulanmadı).
    pub member: Option<ThreadMember>,
}

impl Channel {
    pub fn is_thread(&self) -> bool {
        channel_kind::is_thread(self.kind)
    }

    pub fn is_archived(&self) -> bool {
        self.thread_metadata.as_ref().is_some_and(|m| m.archived)
    }

    pub fn is_locked(&self) -> bool {
        self.thread_metadata.as_ref().is_some_and(|m| m.locked)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThreadMetadata {
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub locked: bool,
    pub archive_timestamp: Option<String>,
}

/// Kullanıcının thread üyeliği (alanların çoğu kullanılmıyor).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThreadMember {
    pub flags: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForumTag {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
}

/// `THREAD_LIST_SYNC`: guild (veya `channel_ids` ile belirtilen üst kanallar) için aktif
/// thread listesinin tamamı.
#[derive(Debug, Deserialize)]
pub struct ThreadListSync {
    pub guild_id: Snowflake,
    /// Verilmişse yalnızca bu üst kanalların thread'leri senkronlanır; yoksa tüm guild.
    pub channel_ids: Option<Vec<Snowflake>>,
    #[serde(default)]
    pub threads: Vec<Channel>,
}

/// Thread listeleyen REST yanıtları (`/threads/search`, `/threads/archived/public`).
#[derive(Debug, Deserialize)]
pub struct ThreadList {
    #[serde(default)]
    pub threads: Vec<Channel>,
    #[serde(default)]
    pub has_more: bool,
}

/// Kanal tipleri (kullanılanlar).
pub mod channel_kind {
    pub const TEXT: u8 = 0;
    pub const DM: u8 = 1;
    pub const VOICE: u8 = 2;
    pub const GROUP_DM: u8 = 3;
    pub const CATEGORY: u8 = 4;
    pub const ANNOUNCEMENT: u8 = 5;
    pub const ANNOUNCEMENT_THREAD: u8 = 10;
    pub const PUBLIC_THREAD: u8 = 11;
    pub const PRIVATE_THREAD: u8 = 12;
    pub const STAGE: u8 = 13;
    pub const FORUM: u8 = 15;
    pub const MEDIA: u8 = 16;

    pub fn is_thread(kind: u8) -> bool {
        matches!(kind, ANNOUNCEMENT_THREAD | PUBLIC_THREAD | PRIVATE_THREAD)
    }

    /// Altında thread'ler listelenen kanal: metin, duyuru, forum, medya.
    pub fn has_threads(kind: u8) -> bool {
        matches!(kind, TEXT | ANNOUNCEMENT | FORUM | MEDIA)
    }

    /// Forum benzeri (içerik thread'lerden ibaret) kanal.
    pub fn is_forum(kind: u8) -> bool {
        matches!(kind, FORUM | MEDIA)
    }
}

impl Ready {
    /// Erişilebilir guild'lerdeki toplam kanal sayısı (kategoriler hariç).
    pub fn channel_count(&self) -> usize {
        self.guilds
            .iter()
            .flat_map(|g| &g.channels)
            .filter(|c| c.kind != channel_kind::CATEGORY)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_fields_and_kinds() {
        let c: Channel = serde_json::from_str(
            r#"{"id":"30","type":12,"parent_id":"2","owner_id":"7","name":"x","message_count":3,"member_count":2,
                "applied_tags":["5"],"thread_metadata":{"archived":true,"locked":false,"archive_timestamp":"2026-01-01T00:00:00+00:00"}}"#,
        )
        .expect("thread");
        assert!(c.is_thread() && c.is_archived() && !c.is_locked());
        assert_eq!(c.owner_id, Some(Snowflake(7)));
        assert_eq!(c.applied_tags, [Snowflake(5)]);
        assert_eq!((c.message_count, c.member_count), (Some(3), Some(2)));

        let forum: Channel = serde_json::from_str(
            r#"{"id":"2","type":15,"name":"f","available_tags":[{"id":"5","name":"soru","emoji_name":null}]}"#,
        )
        .expect("forum");
        assert!(!forum.is_thread() && channel_kind::is_forum(forum.kind));
        assert_eq!(forum.available_tags[0].name, "soru");
        // Metadata yoksa arşivli/kilitli değil.
        let plain: Channel = serde_json::from_str(r#"{"id":"1","type":0}"#).expect("kanal");
        assert!(!plain.is_archived() && !plain.is_locked());
        for k in [10, 11, 12] {
            assert!(channel_kind::is_thread(k));
        }
        assert!(channel_kind::has_threads(channel_kind::MEDIA));
        assert!(!channel_kind::has_threads(channel_kind::VOICE));
    }

    #[test]
    fn guild_threads_default_empty_and_list_shapes() {
        let g: Guild = serde_json::from_str(r#"{"id":"1"}"#).expect("guild");
        assert!(g.threads.is_empty());
        let l: ThreadList =
            serde_json::from_str(r#"{"threads":[{"id":"3","type":11}],"has_more":true}"#)
                .expect("liste");
        assert!(l.has_more && l.threads.len() == 1);
        let empty: ThreadList = serde_json::from_str("{}").expect("boş");
        assert!(empty.threads.is_empty() && !empty.has_more);
    }
}
