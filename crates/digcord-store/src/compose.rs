//! Aşama 3: iyimser gönderim, tepkiler, "yazıyor", izinler ve otomatik tamamlama adayları.

use digcord_model::{
    CustomEmoji, MemberContext, Message, Permissions, Reaction, ReactionEvent, Snowflake,
    TypingStart, channel_kind,
};

use crate::{State, TreeKind};

/// "Yazıyor" bildiriminin geçerlilik süresi (Discord istemcileri ~10 sn kullanır).
const TYPING_SECS: u64 = 10;

impl State {
    /// Gönderilmeden önce mesajı "gönderiliyor" olarak ekler. `msg.id` geçici id (nonce snowflake'i),
    /// `msg.nonce` gerçek mesajla eşleştirmek için zorunludur. Kanal yüklenmemişse eklenmez.
    pub fn add_pending(&mut self, msg: Message) {
        let Some(nonce) = msg.nonce.clone() else {
            return;
        };
        if !self.msgs.get(&msg.channel_id).is_some_and(|c| !c.detached) {
            return;
        }
        self.version += 1;
        self.pending.insert(nonce, (msg.channel_id, msg.id));
        if let Some(cm) = self.msgs.get_mut(&msg.channel_id) {
            crate::insert_sorted(&mut cm.messages, msg);
        }
    }

    /// Gönderim başarısız: mesaj kalır ama "gönderilemedi" olarak işaretlenir.
    pub fn mark_failed(&mut self, nonce: &str) {
        if let Some((_, id)) = self.pending.get(nonce) {
            self.failed.insert(*id);
            self.version += 1;
        }
    }

    /// Başarısız iyimser mesajı tamamen kaldırır (kullanıcı vazgeçti).
    pub fn discard_pending(&mut self, nonce: &str) {
        if let Some((channel, id)) = self.pending.remove(nonce) {
            self.failed.remove(&id);
            if let Some(cm) = self.msgs.get_mut(&channel)
                && let Ok(i) = cm.messages.binary_search_by_key(&id, |m| m.id)
            {
                cm.messages.remove(i);
            }
            self.version += 1;
        }
    }

    pub fn is_pending(&self, id: Snowflake) -> bool {
        self.pending.values().any(|(_, p)| *p == id)
    }

    pub fn is_failed(&self, id: Snowflake) -> bool {
        self.failed.contains(&id)
    }

    /// Gelen gerçek mesaj bir iyimser mesajın karşılığıysa geçicisini kaldırır.
    pub(crate) fn resolve_pending(&mut self, msg: &Message) {
        let Some((channel, temp)) = msg.nonce.as_ref().and_then(|n| self.pending.remove(n)) else {
            return;
        };
        self.failed.remove(&temp);
        if let Some(cm) = self.msgs.get_mut(&channel)
            && let Ok(i) = cm.messages.binary_search_by_key(&temp, |m| m.id)
        {
            cm.messages.remove(i);
        }
    }

    pub fn reaction_add(&mut self, ev: &ReactionEvent) {
        let mine = self.me.as_ref().is_some_and(|u| u.id == ev.user_id);
        let Some(m) = self.find_message_mut(ev.channel_id, ev.message_id) else {
            return;
        };
        match m.reactions.iter_mut().find(|r| r.emoji == ev.emoji) {
            Some(r) => {
                if mine && r.me {
                    return; // REST yanıtı ve gateway yankısı iki kez saymasın
                }
                r.count += 1;
                r.me |= mine;
            }
            None => m.reactions.push(Reaction {
                count: 1,
                me: mine,
                emoji: ev.emoji.clone(),
            }),
        }
        self.version += 1;
    }

    pub fn reaction_remove(&mut self, ev: &ReactionEvent) {
        let mine = self.me.as_ref().is_some_and(|u| u.id == ev.user_id);
        let Some(m) = self.find_message_mut(ev.channel_id, ev.message_id) else {
            return;
        };
        let Some(i) = m.reactions.iter().position(|r| r.emoji == ev.emoji) else {
            return;
        };
        if mine && !m.reactions[i].me {
            return;
        }
        let r = &mut m.reactions[i];
        r.count = r.count.saturating_sub(1);
        if mine {
            r.me = false;
        }
        if r.count == 0 {
            m.reactions.remove(i);
        }
        self.version += 1;
    }

    fn find_message_mut(&mut self, channel: Snowflake, id: Snowflake) -> Option<&mut Message> {
        let cm = self.msgs.get_mut(&channel)?;
        let i = cm.messages.binary_search_by_key(&id, |m| m.id).ok()?;
        cm.messages.get_mut(i)
    }

    pub fn find_message(&self, channel: Snowflake, id: Snowflake) -> Option<&Message> {
        let cm = self.msgs.get(&channel)?;
        let i = cm.messages.binary_search_by_key(&id, |m| m.id).ok()?;
        cm.messages.get(i)
    }

    pub fn typing_start(&mut self, ev: &TypingStart) {
        if self.me.as_ref().is_some_and(|u| u.id == ev.user_id) {
            return;
        }
        if let (Some(m), Some(gid)) = (&ev.member, ev.guild_id) {
            if let Some(u) = &m.user {
                self.users.insert(u.id, u.clone());
            }
            self.members.insert(
                (gid, ev.user_id),
                crate::MemberInfo {
                    nick: m.nick.clone(),
                    roles: m.roles.clone(),
                },
            );
        }
        let until = ev.timestamp + TYPING_SECS;
        let list = self.typing.entry(ev.channel_id).or_default();
        match list.iter_mut().find(|(u, _)| *u == ev.user_id) {
            Some(entry) => entry.1 = until,
            None => list.push((ev.user_id, until)),
        }
        self.version += 1;
    }

    pub(crate) fn stop_typing(&mut self, channel: Snowflake, user: Snowflake) {
        if let Some(list) = self.typing.get_mut(&channel) {
            list.retain(|(u, _)| *u != user);
        }
    }

    /// `now` (Unix saniyesi) itibarıyla kanalda yazan kullanıcıların adları.
    pub fn typing_names(&self, channel: Snowflake, now: u64) -> Vec<String> {
        let guild = self.channels.get(&channel).and_then(|c| c.guild_id);
        self.typing
            .get(&channel)
            .into_iter()
            .flatten()
            .filter(|(_, until)| *until > now)
            .map(|(u, _)| match self.users.get(u) {
                Some(user) => self.author_name(guild, user),
                None => "biri".to_owned(),
            })
            .collect()
    }

    /// Bu kanaldaki izinlerim; DM'lerde her şey.
    pub fn my_permissions(&self, channel: Snowflake) -> Permissions {
        let (Some(ch), Some(me)) = (self.channels.get(&channel), self.me.as_ref()) else {
            return Permissions::default();
        };
        let Some(g) = ch
            .guild_id
            .and_then(|gid| self.guilds.get(&gid).map(|g| (gid, g)))
        else {
            return Permissions::ALL;
        };
        let (gid, g) = g;
        // Thread'lerin kendi overwrite'ı yoktur: üst kanalınkiler geçerlidir.
        let overwrites = if ch.is_thread() {
            ch.parent_id
                .and_then(|p| self.channels.get(&p))
                .map_or(&[][..], |p| &p.permission_overwrites[..])
        } else {
            &ch.permission_overwrites[..]
        };
        MemberContext {
            guild_id: gid,
            owner_id: g.owner_id,
            user_id: me.id,
            member_roles: &g.my_roles,
            roles: &g.roles,
        }
        .channel(overwrites)
    }

    /// Thread'de `SEND_MESSAGES_IN_THREADS` gerekir; kilitliyse ayrıca `MANAGE_THREADS`.
    /// Arşivli (kilitsiz) thread'e yazmak onu sunucuda açar, bu yüzden engellenmez.
    pub fn can_send(&self, channel: Snowflake) -> bool {
        let perms = self.my_permissions(channel);
        match self.channels.get(&channel) {
            Some(ch) if ch.is_thread() => {
                perms.contains(Permissions::SEND_MESSAGES_IN_THREADS)
                    && (!ch.is_locked() || perms.contains(Permissions::MANAGE_THREADS))
            }
            _ => perms.contains(Permissions::SEND_MESSAGES),
        }
    }

    /// Kanalda benim gönderdiğim en yeni (geçici olmayan) mesaj — boş girişte ↑ ile düzenleme için.
    pub fn last_own_message(&self, channel: Snowflake) -> Option<&Message> {
        let me = self.me.as_ref()?.id;
        self.msgs
            .get(&channel)?
            .messages
            .iter()
            .rev()
            .find(|m| m.author.id == me && !m.is_system() && !self.is_pending(m.id))
    }

    pub fn guild_emojis(&self, guild: Snowflake) -> &[CustomEmoji] {
        self.guilds.get(&guild).map_or(&[], |g| &g.emojis)
    }

    /// `@` tamamlaması: kanalda görülen yazarlar ve DM alıcıları; ad başı eşleşmesi
    /// (büyük/küçük harf duyarsız). En fazla `max`, gösterim adına göre sıralı.
    pub fn mention_candidates(
        &self,
        channel: Snowflake,
        prefix: &str,
        max: usize,
    ) -> Vec<(Snowflake, String)> {
        let prefix = prefix.to_lowercase();
        let guild = self.channels.get(&channel).and_then(|c| c.guild_id);
        let mut ids: Vec<Snowflake> = Vec::new();
        if let Some(cm) = self.msgs.get(&channel) {
            for m in cm.messages.iter().rev() {
                if !ids.contains(&m.author.id) {
                    ids.push(m.author.id);
                }
            }
        }
        if let Some(ch) = self.channels.get(&channel) {
            for r in &ch.recipient_ids {
                if !ids.contains(r) {
                    ids.push(*r);
                }
            }
        }
        let mut out: Vec<(Snowflake, String)> = ids
            .into_iter()
            .filter_map(|id| self.users.get(&id))
            .filter(|u| {
                let name = self.author_name(guild, u).to_lowercase();
                name.starts_with(&prefix) || u.username.to_lowercase().starts_with(&prefix)
            })
            .map(|u| (u.id, self.author_name(guild, u)))
            .collect();
        out.sort_by_key(|a| a.1.to_lowercase());
        out.truncate(max);
        out
    }

    /// `#` tamamlaması: aynı sunucudaki okunabilir kanallar.
    pub fn channel_candidates(
        &self,
        guild: Snowflake,
        prefix: &str,
        max: usize,
    ) -> Vec<(Snowflake, String)> {
        let prefix = prefix.to_lowercase();
        self.guild_tree(guild)
            .iter()
            .filter(|t| {
                t.kind == TreeKind::Channel
                    && matches!(
                        t.channel_kind,
                        channel_kind::TEXT | channel_kind::ANNOUNCEMENT
                    )
                    && t.name.to_lowercase().starts_with(&prefix)
            })
            .take(max)
            .map(|t| (t.id, t.name.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use digcord_model::{Message, ReactionEmoji, ReactionEvent, Ready, Snowflake, TypingStart};

    use crate::State;

    const ME: u64 = 100;
    const FRIEND: u64 = 200;
    const CH: u64 = 11;

    fn state() -> State {
        let ready: Ready = serde_json::from_value(serde_json::json!({
            "v": 9, "session_id": "s", "resume_gateway_url": "wss://x",
            "user": {"id": ME.to_string(), "username": "ben", "global_name": null},
            "guilds": [{"id": "1", "properties": {"name": "G", "owner_id": "9"},
                "roles": [{"id": "1", "name": "@everyone", "permissions": "1024"}],
                "emojis": [{"id": "77", "name": "kedi", "animated": false}],
                "channels": [
                    {"id": CH.to_string(), "type": 0, "name": "sohbet"},
                    {"id": "12", "type": 0, "name": "salt-okunur"},
                    {"id": "13", "type": 0, "name": "sanat",
                     "permission_overwrites": [{"id": "1", "type": 0, "allow": "2048", "deny": "0"}]}
                ]}],
            "private_channels": [{"id": "50", "type": 1, "recipient_ids": [FRIEND.to_string()]}],
            "users": [{"id": FRIEND.to_string(), "username": "arkadas", "global_name": "Arkadaş"}],
            "merged_members": [[{"user_id": ME.to_string(), "roles": []}]]
        }))
        .expect("ready");
        let mut st = State::new();
        st.load_ready(&ready);
        st.history_loaded(Snowflake(CH), None, 50, vec![msg(5, FRIEND, None)]);
        st
    }

    fn msg(id: u64, author: u64, nonce: Option<&str>) -> Message {
        serde_json::from_value(serde_json::json!({
            "id": id.to_string(), "channel_id": CH.to_string(), "guild_id": "1",
            "author": {"id": author.to_string(), "username": if author == ME {"ben"} else {"arkadas"},
                       "global_name": if author == ME { None } else { Some("Arkadaş") }},
            "content": "x", "nonce": nonce
        }))
        .expect("mesaj")
    }

    fn ids(st: &State) -> Vec<u64> {
        st.messages(Snowflake(CH))
            .expect("kanal")
            .iter()
            .map(|m| m.id.0)
            .collect()
    }

    #[test]
    fn pending_resolved_by_nonce_failed_and_discarded() {
        let mut st = state();
        st.add_pending(msg(1000, ME, Some("n1")));
        assert!(st.is_pending(Snowflake(1000)));
        assert_eq!(ids(&st), [5, 1000]);
        // Gerçek mesaj (farklı id, aynı nonce) geçiciyi değiştirir; tekrar gelirse çiftlenmez.
        st.message_create(msg(1001, ME, Some("n1")));
        st.message_create(msg(1001, ME, Some("n1")));
        assert_eq!(ids(&st), [5, 1001]);
        assert!(!st.is_pending(Snowflake(1000)));

        st.add_pending(msg(2000, ME, Some("n2")));
        st.mark_failed("n2");
        assert!(st.is_failed(Snowflake(2000)));
        assert!(
            st.last_own_message(Snowflake(CH))
                .is_some_and(|m| m.id == Snowflake(1001))
        );
        st.discard_pending("n2");
        assert_eq!(ids(&st), [5, 1001]);
        assert!(!st.is_failed(Snowflake(2000)));
    }

    #[test]
    fn reactions_count_and_me_without_double_counting() {
        let mut st = state();
        let ev = |user: u64| ReactionEvent {
            user_id: Snowflake(user),
            channel_id: Snowflake(CH),
            message_id: Snowflake(5),
            guild_id: None,
            emoji: ReactionEmoji::unicode("👍"),
        };
        st.reaction_add(&ev(FRIEND));
        st.reaction_add(&ev(ME));
        st.reaction_add(&ev(ME)); // yankı
        let r = &st
            .find_message(Snowflake(CH), Snowflake(5))
            .expect("mesaj")
            .reactions;
        assert_eq!((r[0].count, r[0].me), (2, true));
        st.reaction_remove(&ev(ME));
        st.reaction_remove(&ev(ME));
        let r = &st
            .find_message(Snowflake(CH), Snowflake(5))
            .expect("mesaj")
            .reactions;
        assert_eq!((r[0].count, r[0].me), (1, false));
        st.reaction_remove(&ev(FRIEND));
        assert!(
            st.find_message(Snowflake(CH), Snowflake(5))
                .expect("mesaj")
                .reactions
                .is_empty()
        );
    }

    #[test]
    fn typing_expires_and_stops_on_message() {
        let mut st = state();
        let t = |user: u64| TypingStart {
            channel_id: Snowflake(CH),
            guild_id: Some(Snowflake(1)),
            user_id: Snowflake(user),
            timestamp: 1_000,
            member: None,
        };
        st.typing_start(&t(FRIEND));
        st.typing_start(&t(ME)); // kendim gösterilmem
        assert_eq!(st.typing_names(Snowflake(CH), 1_005), ["Arkadaş"]);
        assert!(st.typing_names(Snowflake(CH), 1_011).is_empty());
        st.typing_start(&t(FRIEND));
        st.message_create(msg(6, FRIEND, None));
        assert!(st.typing_names(Snowflake(CH), 1_005).is_empty());
    }

    #[test]
    fn permissions_candidates_and_emojis() {
        let st = state();
        assert!(!st.can_send(Snowflake(CH)), "@everyone sadece VIEW");
        assert!(st.can_send(Snowflake(13)), "overwrite SEND verir");
        assert!(st.can_send(Snowflake(50)), "DM");
        assert_eq!(
            st.mention_candidates(Snowflake(CH), "ark", 5),
            [(Snowflake(FRIEND), "Arkadaş".to_owned())]
        );
        assert_eq!(
            st.mention_candidates(Snowflake(50), "a", 5).len(),
            1,
            "DM alıcısı"
        );
        assert!(st.mention_candidates(Snowflake(CH), "zz", 5).is_empty());
        let chans = st.channel_candidates(Snowflake(1), "sa", 5);
        assert_eq!(
            chans.iter().map(|c| c.1.as_str()).collect::<Vec<_>>(),
            ["salt-okunur", "sanat"]
        );
        assert_eq!(st.guild_emojis(Snowflake(1))[0].name, "kedi");
    }
}
