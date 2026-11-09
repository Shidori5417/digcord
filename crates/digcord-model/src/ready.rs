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

#[derive(Debug, Clone, Deserialize)]
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
    /// Büyük guild: op 37 aboneliği genişletilmiş biçimde gönderilir (PROTOCOL_NOTES §4.1).
    #[serde(default)]
    pub large: bool,
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
}

/// Kanal tipleri (kullanılanlar).
pub mod channel_kind {
    pub const TEXT: u8 = 0;
    pub const DM: u8 = 1;
    pub const VOICE: u8 = 2;
    pub const GROUP_DM: u8 = 3;
    pub const CATEGORY: u8 = 4;
    pub const ANNOUNCEMENT: u8 = 5;
    pub const STAGE: u8 = 13;
    pub const FORUM: u8 = 15;
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
