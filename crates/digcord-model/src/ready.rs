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
    /// Büyük guild: op 37 aboneliği genişletilmiş biçimde gönderilir (PROTOCOL_NOTES §4.1).
    #[serde(default)]
    pub large: bool,
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
