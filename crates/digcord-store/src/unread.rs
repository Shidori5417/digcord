//! Aşama 4: okunmamış/bahsetme sayaçları, sessize alma, bildirim kararı, arkadaşlar.

use std::str::FromStr;

use digcord_model::{
    GuildSettings, Message, MessageAck, MuteConfig, Ready, Snowflake, channel_kind,
};

use crate::{State, TreeKind};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReadMark {
    last_read: Option<Snowflake>,
    mentions: u32,
}

/// Hangi mesajlarda masaüstü bildirimi istendiği (config `notifications.on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyFilter {
    pub mention: bool,
    pub dm: bool,
    pub all: bool,
}

/// Bildirim seviyesi: 0 tüm mesajlar, 1 bahsetmeler, 2 hiçbiri, 3 üst ayar.
const NOTIFY_ALL: u8 = 0;
const NOTIFY_MENTIONS: u8 = 1;
const NOTIFY_NONE: u8 = 2;
const NOTIFY_INHERIT: u8 = 3;

fn readable(kind: u8) -> bool {
    matches!(
        kind,
        channel_kind::TEXT | channel_kind::ANNOUNCEMENT | channel_kind::DM | channel_kind::GROUP_DM
    ) || channel_kind::is_thread(kind)
}

/// Sessize alma şu an geçerli mi (süreli ise bitişi geçmemiş mi).
fn mute_active(muted: bool, config: Option<&MuteConfig>, now_unix: i64) -> bool {
    if !muted {
        return false;
    }
    let Some(end) = config.and_then(|c| c.end_time.as_deref()) else {
        return true;
    };
    jiff::Timestamp::from_str(end).map_or(true, |t| t.as_second() > now_unix)
}

impl State {
    pub(crate) fn load_read_state(&mut self, ready: &Ready) {
        self.read = ready
            .read_state
            .entries
            .iter()
            .filter(|e| e.read_state_type.unwrap_or(0) == 0)
            .map(|e| {
                (
                    e.id,
                    ReadMark {
                        last_read: e.last_message_id,
                        mentions: e.mention_count,
                    },
                )
            })
            .collect();
        self.notify_settings = ready
            .user_guild_settings
            .entries
            .iter()
            .map(|s| (s.guild_id, s.clone()))
            .collect();
        self.relationships = ready.relationships.clone();
    }

    /// Kanalda okunmamış mesaj var mı. Okunma kaydı olmayan kanal okunmuş sayılır.
    pub fn is_unread(&self, channel: Snowflake) -> bool {
        let Some(ch) = self.channels.get(&channel) else {
            return false;
        };
        if !readable(ch.kind) {
            return false;
        }
        match (ch.last_message_id, self.read.get(&channel)) {
            (Some(last), Some(mark)) => mark.last_read.is_none_or(|r| r < last),
            _ => false,
        }
    }

    pub fn mention_count(&self, channel: Snowflake) -> u32 {
        self.read.get(&channel).map_or(0, |m| m.mentions)
    }

    /// Okundu: `message`'a kadar (geriye gitmez); bahsetmeler sıfırlanır.
    pub fn mark_read(&mut self, channel: Snowflake, message: Snowflake) {
        let mark = self.read.entry(channel).or_default();
        if mark.last_read.is_none_or(|r| r < message) {
            mark.last_read = Some(message);
        }
        mark.mentions = 0;
        self.version += 1;
    }

    /// Başka oturumdan gelen okundu işareti.
    pub fn apply_ack(&mut self, ack: &MessageAck) {
        self.mark_read(ack.channel_id, ack.message_id);
        if let Some(m) = self.read.get_mut(&ack.channel_id) {
            m.mentions = ack.mention_count;
        }
    }

    /// Kanal (veya sunucusu) şu an sessizde mi.
    pub fn is_muted(&self, channel: Snowflake, now_unix: i64) -> bool {
        let guild = self.channels.get(&channel).and_then(|c| c.guild_id);
        let Some(s) = self.notify_settings.get(&guild) else {
            return false;
        };
        if guild.is_some() && mute_active(s.muted, s.mute_config.as_ref(), now_unix) {
            return true;
        }
        // Thread'in kendi ayarı yoksa üst kanalın sessize alması geçerli (doğrulanmadı).
        let parent = self.thread_parent(channel);
        [Some(channel), parent].into_iter().flatten().any(|id| {
            s.channel_overrides
                .iter()
                .find(|o| o.channel_id == id)
                .is_some_and(|o| mute_active(o.muted, o.mute_config.as_ref(), now_unix))
        })
    }

    pub fn is_guild_muted(&self, guild: Snowflake, now_unix: i64) -> bool {
        self.notify_settings
            .get(&Some(guild))
            .is_some_and(|s| mute_active(s.muted, s.mute_config.as_ref(), now_unix))
    }

    /// Sunucuda (sessizde olmayan, görünür) okunmamış kanal var mı ve toplam bahsetme.
    pub fn guild_unread(&self, guild: Snowflake, now_unix: i64) -> (bool, u32) {
        let mut unread = false;
        let mut mentions = 0;
        for item in self.guild_tree(guild) {
            if item.kind != TreeKind::Channel {
                continue;
            }
            mentions += self.mention_count(item.id);
            unread |= self.is_unread(item.id) && !self.is_muted(item.id, now_unix);
            for t in self.threads_of(item.id, None) {
                mentions += self.mention_count(t.id);
                unread |= self.is_unread(t.id) && !self.is_muted(t.id, now_unix);
            }
        }
        (unread && !self.is_guild_muted(guild, now_unix), mentions)
    }

    /// DM'lerde okunmamış var mı ve toplam bahsetme (DM'de her mesaj sayılır).
    pub fn dm_unread(&self) -> (bool, u32) {
        let mut unread = false;
        let mut mentions = 0;
        for id in self.dm_channels() {
            unread |= self.is_unread(*id);
            mentions += self.mention_count(*id);
        }
        (unread, mentions)
    }

    fn settings_for(&self, guild: Option<Snowflake>) -> Option<&GuildSettings> {
        self.notify_settings.get(&guild)
    }

    /// Mesaj beni ilgilendiriyor mu: doğrudan bahsetme, (bastırılmamışsa) @everyone veya rolüm.
    pub fn mentions_me(&self, msg: &Message) -> bool {
        let Some(me) = self.me.as_ref().map(|u| u.id) else {
            return false;
        };
        if msg.author.id == me {
            return false;
        }
        if msg.mentions.iter().any(|u| u.id == me) {
            return true;
        }
        let settings = self.settings_for(msg.guild_id);
        if msg.mention_everyone && !settings.is_some_and(|s| s.suppress_everyone) {
            return true;
        }
        if settings.is_some_and(|s| s.suppress_roles) {
            return false;
        }
        msg.guild_id
            .and_then(|g| self.guilds.get(&g))
            .is_some_and(|g| msg.mention_roles.iter().any(|r| g.my_roles.contains(r)))
    }

    fn is_dm_channel(&self, channel: Snowflake) -> bool {
        self.channels.get(&channel).is_some_and(|c| {
            c.guild_id.is_none() && matches!(c.kind, channel_kind::DM | channel_kind::GROUP_DM)
        })
    }

    /// Yeni mesajda sayaçlar: kendi mesajım okundu sayılır; DM'de ve bahsetmede +1.
    pub(crate) fn count_unread(&mut self, msg: &Message) {
        let me = self.me.as_ref().map(|u| u.id);
        if Some(msg.author.id) == me {
            let mark = self.read.entry(msg.channel_id).or_default();
            if mark.last_read.is_none_or(|r| r < msg.id) {
                mark.last_read = Some(msg.id);
            }
            return;
        }
        if self.is_dm_channel(msg.channel_id) || self.mentions_me(msg) {
            self.read.entry(msg.channel_id).or_default().mentions += 1;
        } else {
            // Kaydı olmayan kanal da yeni mesajla okunmamış görünsün.
            self.read.entry(msg.channel_id).or_default();
        }
    }

    /// Bu mesaj için masaüstü bildirimi gösterilmeli mi (odak/açık kanal kontrolü çağıranda).
    pub fn should_notify(&self, msg: &Message, filter: NotifyFilter, now_unix: i64) -> bool {
        if self.me.as_ref().is_some_and(|u| u.id == msg.author.id) || msg.is_system() {
            return false;
        }
        if self.is_muted(msg.channel_id, now_unix) {
            return false;
        }
        let dm = self.is_dm_channel(msg.channel_id);
        let settings = self.settings_for(if dm { None } else { msg.guild_id });
        let parent = self.thread_parent(msg.channel_id);
        let channel_level = settings
            .and_then(|s| {
                [Some(msg.channel_id), parent]
                    .into_iter()
                    .flatten()
                    .find_map(|id| {
                        s.channel_overrides.iter().find(|o| {
                            o.channel_id == id && o.message_notifications != NOTIFY_INHERIT
                        })
                    })
            })
            .map(|o| o.message_notifications);
        // Kullanıcı ayarı yoksa: DM'de tümü, sunucuda sadece bahsetmeler.
        let default = if dm { NOTIFY_ALL } else { NOTIFY_MENTIONS };
        let level = channel_level
            .or(settings.map(|s| s.message_notifications))
            .filter(|l| *l != NOTIFY_INHERIT)
            .unwrap_or(default);
        if level == NOTIFY_NONE {
            return false;
        }
        if dm {
            return filter.dm;
        }
        if self.mentions_me(msg) {
            return filter.mention;
        }
        level == NOTIFY_ALL && filter.all
    }

    /// Okunmamış (sessizde olmayan) kanallar, gezinme sırasıyla: önce DM'ler, sonra sunucular.
    pub fn unread_channels(&self, now_unix: i64) -> Vec<(Option<Snowflake>, Snowflake)> {
        let mut out: Vec<(Option<Snowflake>, Snowflake)> = self
            .dm_channels()
            .iter()
            .filter(|c| self.is_unread(**c))
            .map(|c| (None, *c))
            .collect();
        for g in self.guilds() {
            if self.is_guild_muted(g.id, now_unix) {
                continue;
            }
            for item in self.guild_tree(g.id) {
                if item.kind == TreeKind::Channel
                    && self.is_unread(item.id)
                    && !self.is_muted(item.id, now_unix)
                {
                    out.push((Some(g.id), item.id));
                }
                if item.kind == TreeKind::Channel {
                    for t in self.threads_of(item.id, None) {
                        if !t.archived && self.is_unread(t.id) && !self.is_muted(t.id, now_unix) {
                            out.push((Some(g.id), t.id));
                        }
                    }
                }
            }
        }
        out
    }

    /// Arkadaşlar (ilişki türü 1), gösterim adına göre.
    pub fn friends(&self) -> Vec<(Snowflake, String)> {
        let mut out: Vec<(Snowflake, String)> = self
            .relationships
            .iter()
            .filter(|r| r.kind == 1)
            .map(|r| {
                let name = r.nickname.clone().unwrap_or_else(|| {
                    self.users
                        .get(&r.id)
                        .map_or_else(|| r.id.to_string(), |u| u.display_name().to_owned())
                });
                (r.id, name)
            })
            .collect();
        out.sort_by_key(|(_, n)| n.to_lowercase());
        out
    }
}

#[cfg(test)]
mod tests {
    use digcord_model::{Message, MessageAck, Ready, Snowflake};

    use super::NotifyFilter;
    use crate::State;

    const ME: u64 = 100;
    const FRIEND: u64 = 200;
    const GUILD: u64 = 1;
    const GENEL: u64 = 11;
    const SESSIZ: u64 = 12;
    const DM: u64 = 50;
    const NOW: i64 = 1_800_000_000;
    const ALL: NotifyFilter = NotifyFilter {
        mention: true,
        dm: true,
        all: true,
    };
    const DEFAULT: NotifyFilter = NotifyFilter {
        mention: true,
        dm: true,
        all: false,
    };

    fn state(guild_settings: serde_json::Value) -> State {
        let ready: Ready = serde_json::from_value(serde_json::json!({
            "v": 9, "session_id": "s", "resume_gateway_url": "wss://x",
            "user": {"id": ME.to_string(), "username": "ben", "global_name": null},
            "guilds": [{"id": GUILD.to_string(), "properties": {"name": "G", "owner_id": "9"},
                "roles": [{"id": GUILD.to_string(), "name": "@everyone", "permissions": "3072"},
                          {"id": "5", "name": "mod", "permissions": "0"}],
                "channels": [
                    {"id": GENEL.to_string(), "type": 0, "name": "genel", "last_message_id": "300"},
                    {"id": SESSIZ.to_string(), "type": 0, "name": "sessiz", "last_message_id": "300"}
                ]}],
            "private_channels": [{"id": DM.to_string(), "type": 1, "recipient_ids": [FRIEND.to_string()], "last_message_id": "300"}],
            "users": [{"id": FRIEND.to_string(), "username": "arkadas", "global_name": "Arkadaş"}],
            "merged_members": [[{"user_id": ME.to_string(), "roles": ["5"]}]],
            "read_state": {"entries": [
                {"id": GENEL.to_string(), "last_message_id": "300", "mention_count": 0},
                {"id": SESSIZ.to_string(), "last_message_id": "200", "mention_count": 0},
                {"id": DM.to_string(), "last_message_id": "300", "mention_count": 0}
            ]},
            "user_guild_settings": {"entries": guild_settings},
            "relationships": [{"id": FRIEND.to_string(), "type": 1, "nickname": null},
                              {"id": "999", "type": 2, "nickname": null}]
        }))
        .expect("ready");
        let mut st = State::new();
        st.load_ready(&ready);
        st
    }

    fn msg(id: u64, channel: u64, author: u64, extra: serde_json::Value) -> Message {
        let mut v = serde_json::json!({
            "id": id.to_string(), "channel_id": channel.to_string(),
            "author": if author == FRIEND {
                serde_json::json!({"id": author.to_string(), "username": "arkadas", "global_name": "Arkadaş"})
            } else {
                serde_json::json!({"id": author.to_string(), "username": "ben", "global_name": null})
            },
            "content": "x"
        });
        if channel != DM {
            v["guild_id"] = serde_json::json!(GUILD.to_string());
        }
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        serde_json::from_value(v).expect("mesaj")
    }

    fn settings() -> serde_json::Value {
        serde_json::json!([
            {"guild_id": GUILD.to_string(), "muted": false, "message_notifications": 1,
             "suppress_everyone": true, "suppress_roles": false,
             "channel_overrides": [{"channel_id": SESSIZ.to_string(), "muted": true, "mute_config": null, "message_notifications": 3}]}
        ])
    }

    #[test]
    fn unread_from_ready_and_new_messages() {
        let mut st = state(settings());
        assert!(!st.is_unread(Snowflake(GENEL)));
        assert!(st.is_unread(Snowflake(SESSIZ)), "200 < 300");
        // SESSIZ sessizde: sunucu okunmamış görünmez.
        assert_eq!(st.guild_unread(Snowflake(GUILD), NOW), (false, 0));

        st.message_create(msg(400, GENEL, FRIEND, serde_json::json!({})));
        assert!(st.is_unread(Snowflake(GENEL)));
        assert_eq!(st.mention_count(Snowflake(GENEL)), 0);
        assert_eq!(st.guild_unread(Snowflake(GUILD), NOW), (true, 0));

        st.message_create(msg(401, GENEL, FRIEND, serde_json::json!({"mentions": [{"id": ME.to_string(), "username": "ben", "global_name": null}]})));
        assert_eq!(st.mention_count(Snowflake(GENEL)), 1);
        // @everyone bastırılmış → sayılmaz; rolüm (5) sayılır.
        st.message_create(msg(
            402,
            GENEL,
            FRIEND,
            serde_json::json!({"mention_everyone": true}),
        ));
        st.message_create(msg(
            403,
            GENEL,
            FRIEND,
            serde_json::json!({"mention_roles": ["5"]}),
        ));
        assert_eq!(st.mention_count(Snowflake(GENEL)), 2);

        st.mark_read(Snowflake(GENEL), Snowflake(403));
        assert!(!st.is_unread(Snowflake(GENEL)));
        assert_eq!(st.mention_count(Snowflake(GENEL)), 0);
        st.mark_read(Snowflake(GENEL), Snowflake(100));
        assert!(!st.is_unread(Snowflake(GENEL)), "okundu geriye gitmez");

        // Kendi mesajım okundu sayılır.
        st.message_create(msg(404, GENEL, ME, serde_json::json!({})));
        assert!(!st.is_unread(Snowflake(GENEL)));

        // DM: her mesaj bahsetme sayılır.
        st.message_create(msg(405, DM, FRIEND, serde_json::json!({})));
        assert_eq!(st.dm_unread(), (true, 1));
        st.apply_ack(&MessageAck {
            channel_id: Snowflake(DM),
            message_id: Snowflake(405),
            mention_count: 0,
        });
        assert_eq!(st.dm_unread(), (false, 0));
    }

    #[test]
    fn notification_rules() {
        let st = state(settings());
        let mention = serde_json::json!({"mentions": [{"id": ME.to_string(), "username": "ben", "global_name": null}]});
        // Sunucu: sadece bahsetmeler.
        assert!(!st.should_notify(&msg(500, GENEL, FRIEND, serde_json::json!({})), ALL, NOW));
        assert!(st.should_notify(&msg(501, GENEL, FRIEND, mention.clone()), DEFAULT, NOW));
        // Sessiz kanal: bahsetmede bile yok.
        assert!(!st.should_notify(&msg(502, SESSIZ, FRIEND, mention.clone()), ALL, NOW));
        // DM: varsayılan açık; filtre kapalıysa yok; kendi mesajım yok.
        assert!(st.should_notify(&msg(503, DM, FRIEND, serde_json::json!({})), DEFAULT, NOW));
        assert!(!st.should_notify(
            &msg(504, DM, FRIEND, serde_json::json!({})),
            NotifyFilter {
                dm: false,
                ..DEFAULT
            },
            NOW
        ));
        assert!(!st.should_notify(&msg(505, DM, ME, serde_json::json!({})), ALL, NOW));

        // "Tüm mesajlar" sunucu ayarı + filtrede all.
        let all = serde_json::json!([{"guild_id": GUILD.to_string(), "muted": false, "message_notifications": 0, "channel_overrides": []}]);
        let st = state(all);
        assert!(st.should_notify(&msg(506, GENEL, FRIEND, serde_json::json!({})), ALL, NOW));
        assert!(!st.should_notify(
            &msg(507, GENEL, FRIEND, serde_json::json!({})),
            DEFAULT,
            NOW
        ));
    }

    #[test]
    fn timed_mute_expires() {
        let timed = |end: &str| {
            serde_json::json!([{"guild_id": GUILD.to_string(), "muted": true,
                "mute_config": {"end_time": end, "selected_time_window": 3600}, "channel_overrides": []}])
        };
        let end = jiff::Timestamp::from_second(NOW).expect("zaman");
        let future = end
            .checked_add(jiff::SignedDuration::from_hours(1))
            .expect("zaman");
        let past = end
            .checked_sub(jiff::SignedDuration::from_hours(1))
            .expect("zaman");
        let st = state(timed(&future.to_string()));
        assert!(st.is_guild_muted(Snowflake(GUILD), NOW));
        let st = state(timed(&past.to_string()));
        assert!(!st.is_guild_muted(Snowflake(GUILD), NOW));
    }

    #[test]
    fn unread_navigation_and_friends() {
        let mut st = state(serde_json::json!([]));
        st.message_create(msg(600, DM, FRIEND, serde_json::json!({})));
        st.message_create(msg(601, GENEL, FRIEND, serde_json::json!({})));
        assert_eq!(
            st.unread_channels(NOW),
            [
                (None, Snowflake(DM)),
                (Some(Snowflake(GUILD)), Snowflake(GENEL)),
                (Some(Snowflake(GUILD)), Snowflake(SESSIZ))
            ]
        );
        assert_eq!(st.friends(), [(Snowflake(FRIEND), "Arkadaş".to_owned())]);
    }
}
