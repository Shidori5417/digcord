//! Mesaj ve gateway mesaj olayları. Alanlar PROTOCOL_NOTES §4.1'deki gözleme göre;
//! bilinmeyen alanlar yok sayılır.

use serde::Deserialize;

use crate::{Member, Snowflake, User};

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub author: User,
    /// Guild mesajlarında yazarın üyelik bilgisi (`user` alanı olmadan).
    pub member: Option<Member>,
    #[serde(default)]
    pub content: String,
    pub edited_timestamp: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: u8,
    #[serde(default)]
    pub mentions: Vec<User>,
    #[serde(default)]
    pub mention_roles: Vec<Snowflake>,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Yanıt verilen mesaj (varsa; silinmişse `null`).
    pub referenced_message: Option<Box<Message>>,
    #[serde(default)]
    pub sticker_items: Vec<StickerItem>,
    #[serde(default)]
    pub embeds: Vec<Embed>,
    /// Arama mesajı (tür 3).
    pub call: Option<Call>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StickerItem {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Embed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Call {
    pub ended_timestamp: Option<String>,
}

/// Mesaj türleri (kullanılanlar). Bunların dışındaki türler sistem mesajıdır.
pub mod message_kind {
    pub const DEFAULT: u8 = 0;
    pub const RECIPIENT_ADD: u8 = 1;
    pub const RECIPIENT_REMOVE: u8 = 2;
    pub const CALL: u8 = 3;
    pub const CHANNEL_NAME_CHANGE: u8 = 4;
    pub const CHANNEL_ICON_CHANGE: u8 = 5;
    pub const PINNED: u8 = 6;
    pub const MEMBER_JOIN: u8 = 7;
    pub const BOOST: u8 = 8;
    pub const BOOST_TIER_1: u8 = 9;
    pub const BOOST_TIER_2: u8 = 10;
    pub const BOOST_TIER_3: u8 = 11;
    pub const THREAD_CREATED: u8 = 18;
    pub const REPLY: u8 = 19;
    pub const CHAT_INPUT_COMMAND: u8 = 20;
    pub const CONTEXT_MENU_COMMAND: u8 = 23;
}

impl Message {
    /// Oluşturulma zamanı; ISO zaman damgası yerine id'den hesaplanır.
    pub fn timestamp_ms(&self) -> u64 {
        self.id.timestamp_ms()
    }

    pub fn is_edited(&self) -> bool {
        self.edited_timestamp.is_some()
    }

    /// Kullanıcının yazdığı değil, Discord'un ürettiği mesaj (arama, katılma, sabitleme...).
    pub fn is_system(&self) -> bool {
        use message_kind::*;
        !matches!(
            self.kind,
            DEFAULT | REPLY | CHAT_INPUT_COMMAND | CONTEXT_MENU_COMMAND
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attachment {
    pub id: Snowflake,
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageDelete {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub guild_id: Option<Snowflake>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observed_shape() {
        let json = r#"{
            "id":"1300000000000000000","channel_id":"2","guild_id":"3","channel_type":0,
            "author":{"id":"4","username":"yazar","global_name":null,"avatar":null,"clan":null},
            "member":{"nick":"Takma","roles":["5"],"joined_at":"2020-01-01T00:00:00+00:00","deaf":false},
            "content":"merhaba <@4>","timestamp":"2025-01-01T00:00:00+00:00","edited_timestamp":null,
            "type":0,"flags":0,"mentions":[{"id":"4","username":"yazar","global_name":null}],
            "mention_roles":[],"mention_everyone":false,"attachments":[],"embeds":[],"components":[],
            "pinned":false,"tts":false,"nonce":"x"
        }"#;
        let m: Message = serde_json::from_str(json).expect("mesaj");
        assert_eq!(m.author.username, "yazar");
        assert_eq!(
            m.member.as_ref().and_then(|x| x.nick.as_deref()),
            Some("Takma")
        );
        assert_eq!(m.mentions.len(), 1);
        assert!(!m.is_edited());
        assert!(m.referenced_message.is_none());
        assert!(m.timestamp_ms() > 1_420_070_400_000);
    }

    #[test]
    fn parses_reply_and_attachment() {
        let json = r#"{
            "id":"10","channel_id":"2","author":{"id":"4","username":"a","global_name":"A"},
            "content":"","edited_timestamp":"2025-01-01T00:00:00+00:00","type":19,
            "attachments":[{"id":"7","filename":"ekran.png","size":1234,"width":1280,"height":720,"url":"u"}],
            "referenced_message":{"id":"9","channel_id":"2","author":{"id":"5","username":"b","global_name":null},"content":"selam"}
        }"#;
        let m: Message = serde_json::from_str(json).expect("mesaj");
        assert!(m.is_edited());
        assert_eq!(m.attachments[0].filename, "ekran.png");
        assert_eq!(
            m.referenced_message.as_ref().map(|r| r.content.as_str()),
            Some("selam")
        );
    }

    #[test]
    fn parses_call_sticker_embed() {
        let json = r#"{
            "id":"10","channel_id":"2","author":{"id":"4","username":"a","global_name":null},
            "content":"","type":3,"edited_timestamp":"2026-01-01T00:00:00+00:00",
            "call":{"participants":["4"],"ended_timestamp":"2026-01-01T00:05:00+00:00"},
            "sticker_items":[{"id":"1","name":"kedi","format_type":1}],
            "embeds":[{"type":"rich","title":"Başlık","description":"açıklama"}]
        }"#;
        let m: Message = serde_json::from_str(json).expect("mesaj");
        assert!(m.is_system());
        assert!(m.call.is_some_and(|c| c.ended_timestamp.is_some()));
        assert_eq!(m.sticker_items[0].name, "kedi");
        assert_eq!(m.embeds[0].title.as_deref(), Some("Başlık"));
    }

    #[test]
    fn parses_delete() {
        let d: MessageDelete =
            serde_json::from_str(r#"{"id":"1","channel_id":"2","guild_id":"3"}"#).expect("silme");
        assert_eq!(d.id, Snowflake(1));
    }
}
