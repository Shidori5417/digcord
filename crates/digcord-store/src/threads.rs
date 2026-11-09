//! Thread'ler: üst kanalın altında tutulur; izinler üst kanaldan miras alınır.
//!
//! Thread'ler `channels` haritasında normal kanallar gibi durur (mesajlar, okunma durumu ve
//! gönderme yolları aynı), ayrıca `thread_children` ile üst kanala bağlanır. Guild ağacına
//! (`TreeItem`) girmezler; arayüz `threads_of` ile üst kanalın altına yerleştirir.
//!
//! Özel thread'ler: sunucu bunları yalnızca üyelere / MANAGE_THREADS sahiplerine gönderir.
//! Üyelik bilgisi (READY'de `member`) kullanıcı hesabında güvenilir olmadığından gelen her
//! thread gösterilir (doğrulanmadı, PROTOCOL_NOTES §4.4).

use digcord_model::{Channel, Snowflake, ThreadListSync};

use crate::State;

/// Kanal panelinde thread satırı için gereken alanlar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadItem {
    pub id: Snowflake,
    pub name: String,
    pub kind: u8,
    pub archived: bool,
    pub locked: bool,
}

impl State {
    /// Thread'i ekler/günceller. `THREAD_UPDATE` üyeliği ve (bazen) son mesajı taşımaz: eskisi korunur.
    pub(crate) fn upsert_thread(&mut self, thread: &Channel) {
        let Some(gid) = thread.guild_id else { return };
        if !self.guilds.contains_key(&gid) {
            return;
        }
        self.version += 1;
        let mut t = thread.clone();
        if let Some(old) = self.channels.get(&t.id) {
            if t.member.is_none() {
                t.member.clone_from(&old.member);
            }
            if t.last_message_id < old.last_message_id {
                t.last_message_id = old.last_message_id;
            }
        }
        if let Some(parent) = t.parent_id {
            let kids = self.thread_children.entry(parent).or_default();
            if !kids.contains(&t.id) {
                kids.push(t.id);
            }
        }
        self.channels.insert(t.id, t);
    }

    pub(crate) fn forget_thread(&mut self, id: Snowflake, parent: Option<Snowflake>) {
        if let Some(p) = parent
            && let Some(kids) = self.thread_children.get_mut(&p)
        {
            kids.retain(|k| *k != id);
            if kids.is_empty() {
                self.thread_children.remove(&p);
            }
        }
        self.channels.remove(&id);
        self.msgs.remove(&id);
    }

    /// Guild'in tüm thread kimlikleri (nadir işlemler için; kanal haritasını tarar).
    fn guild_thread_ids(&self, gid: Snowflake) -> Vec<Snowflake> {
        self.channels
            .values()
            .filter(|c| c.is_thread() && c.guild_id == Some(gid))
            .map(|c| c.id)
            .collect()
    }

    /// READY / GUILD_CREATE: guild'in aktif thread'lerini yazar (öncekiler atılır).
    pub(crate) fn load_guild_threads(&mut self, gid: Snowflake, threads: &[Channel]) {
        for id in self.guild_thread_ids(gid) {
            let parent = self.channels.get(&id).and_then(|c| c.parent_id);
            self.forget_thread(id, parent);
        }
        for t in threads {
            let mut t = t.clone();
            t.guild_id = Some(gid);
            self.upsert_thread(&t);
        }
    }

    /// `THREAD_LIST_SYNC`: kapsamdaki (belirtilen üst kanallar ya da tüm guild) aktif thread'ler
    /// gelen listeyle değiştirilir; REST ile yüklenmiş arşivli olanlara dokunulmaz.
    pub fn thread_list_sync(&mut self, sync: &ThreadListSync) {
        if !self.guilds.contains_key(&sync.guild_id) {
            return;
        }
        self.version += 1;
        let in_scope = |c: &Channel| {
            sync.channel_ids
                .as_ref()
                .is_none_or(|ids| c.parent_id.is_some_and(|p| ids.contains(&p)))
        };
        let stale: Vec<(Snowflake, Option<Snowflake>)> = self
            .channels
            .values()
            .filter(|c| {
                c.is_thread()
                    && c.guild_id == Some(sync.guild_id)
                    && !c.is_archived()
                    && in_scope(c)
                    && !sync.threads.iter().any(|t| t.id == c.id)
            })
            .map(|c| (c.id, c.parent_id))
            .collect();
        for (id, parent) in stale {
            self.forget_thread(id, parent);
        }
        for t in &sync.threads {
            let mut t = t.clone();
            t.guild_id = Some(sync.guild_id);
            self.upsert_thread(&t);
        }
    }

    /// REST ile gelen thread listesi `parent` kanalına yazılır. `archived`: arşiv listesi
    /// (üst kanalın altında dim olarak gösterilmeye başlar); aksi halde aktif liste alındı sayılır.
    pub fn threads_loaded(&mut self, parent: Snowflake, threads: &[Channel], archived: bool) {
        let Some(gid) = self.channels.get(&parent).and_then(|c| c.guild_id) else {
            return;
        };
        self.version += 1;
        if archived {
            self.archived_shown.insert(parent);
        } else {
            self.threads_fetched.insert(parent);
        }
        for t in threads {
            // Yanıtta başka üst kanalın thread'i olmamalı; olursa yok say.
            if t.parent_id.is_some_and(|p| p != parent) {
                continue;
            }
            let mut t = t.clone();
            t.guild_id = Some(gid);
            t.parent_id = Some(parent);
            self.upsert_thread(&t);
        }
    }

    /// Bu üst kanalın aktif thread'leri REST'ten alındı mı.
    pub fn threads_fetched(&self, parent: Snowflake) -> bool {
        self.threads_fetched.contains(&parent)
    }

    pub fn archived_shown(&self, parent: Snowflake) -> bool {
        self.archived_shown.contains(&parent)
    }

    /// Üst kanalın thread'leri, son etkinliğe göre yeniden eskiye. Arşivliler yalnızca
    /// `:archived` ile yüklendiyse (ya da `keep` ise: açık thread listeden düşmesin) gelir.
    pub fn threads_of(&self, parent: Snowflake, keep: Option<Snowflake>) -> Vec<ThreadItem> {
        let show_archived = self.archived_shown.contains(&parent);
        let mut v: Vec<(Snowflake, ThreadItem)> = self
            .thread_children
            .get(&parent)
            .into_iter()
            .flatten()
            .filter_map(|id| self.channels.get(id))
            .filter(|c| !c.is_archived() || show_archived || keep == Some(c.id))
            .map(|c| {
                (
                    c.last_message_id.unwrap_or(c.id).max(c.id),
                    ThreadItem {
                        id: c.id,
                        name: c.name.clone().unwrap_or_else(|| "?".to_owned()),
                        kind: c.kind,
                        archived: c.is_archived(),
                        locked: c.is_locked(),
                    },
                )
            })
            .collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.0));
        v.into_iter().map(|(_, t)| t).collect()
    }

    /// Thread'in üst kanalı.
    pub fn thread_parent(&self, thread: Snowflake) -> Option<Snowflake> {
        self.channels
            .get(&thread)
            .filter(|c| c.is_thread())
            .and_then(|c| c.parent_id)
    }

    /// Thread kilitli mi (kanal thread değilse `false`).
    pub fn is_thread_locked(&self, thread: Snowflake) -> bool {
        self.channels.get(&thread).is_some_and(Channel::is_locked)
    }
}
