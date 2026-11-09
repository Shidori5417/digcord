//! READY olayının şimdilik kullanılan alt kümesi. Bilinmeyen alanlar yok sayılır.

use serde::Deserialize;

use crate::Snowflake;

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
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub id: Snowflake,
    pub username: String,
    pub global_name: Option<String>,
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
}

#[derive(Debug, Deserialize)]
pub struct GuildProperties {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Channel {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: u8,
    pub name: Option<String>,
    pub parent_id: Option<Snowflake>,
    pub position: Option<i32>,
}

impl Ready {
    /// Erişilebilir guild'lerdeki toplam kanal sayısı (kategoriler hariç).
    pub fn channel_count(&self) -> usize {
        const CATEGORY: u8 = 4;
        self.guilds
            .iter()
            .flat_map(|g| &g.channels)
            .filter(|c| c.kind != CATEGORY)
            .count()
    }
}
