//! Bellek içi durum: guild/kanal ağacı, kanal başına mesaj penceresi, kullanıcı ve üye bilgisi.
//!
//! Saf, senkron ve tek yazarlı; ağ veya disk yok. Her değişiklik `version`'ı artırır,
//! arayüz yalnızca sürüm değiştiğinde yeniden çizebilir.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};

use digcord_model::{
    Channel, Guild, GuildMembersChunk, MemberContext, Message, MessageDelete, Permissions, Ready,
    Role, Snowflake, User, channel_kind,
};

/// `trim_channel` sonrası kanalda tutulan en yeni mesaj sayısı.
pub const DEFAULT_WINDOW: usize = 200;
/// Bir kanalda bellekte tutulan mesaj üst sınırı.
const HARD_CAP: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    Category,
    Channel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItem {
    pub id: Snowflake,
    pub kind: TreeKind,
    pub channel_kind: u8,
    pub name: String,
    pub depth: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildInfo {
    pub id: Snowflake,
    pub name: String,
    pub large: bool,
}

#[derive(Default)]
struct GuildData {
    available: bool,
    name: String,
    owner_id: Option<Snowflake>,
    large: bool,
    roles: Vec<Role>,
    my_roles: Vec<Snowflake>,
    channel_ids: Vec<Snowflake>,
    tree: Vec<TreeItem>,
    emojis: Vec<digcord_model::CustomEmoji>,
}

struct MemberInfo {
    nick: Option<String>,
}

#[derive(Default)]
struct ChannelMessages {
    messages: VecDeque<Message>,
    start_reached: bool,
    /// Pencere kanalın en yeni mesajlarını içermiyor (aramadan/sabitlenenden eski bir mesaja
    /// atlandı). Bu durumda gateway'den gelen yeni mesajlar pencereye eklenmez (arada boşluk
    /// kalmasın); kanal yeniden açılınca ilk sayfa çekilip pencere değiştirilir.
    detached: bool,
}

#[derive(Default)]
pub struct State {
    version: u64,
    me: Option<User>,
    users: HashMap<Snowflake, User>,
    guild_order: Vec<Snowflake>,
    guilds: HashMap<Snowflake, GuildData>,
    guild_infos: Vec<GuildInfo>,
    channels: HashMap<Snowflake, Channel>,
    dms: Vec<Snowflake>,
    members: HashMap<(Snowflake, Snowflake), MemberInfo>,
    msgs: HashMap<Snowflake, ChannelMessages>,
    /// Gönderilmekte olan iyimser mesajlar: nonce → (kanal, geçici id).
    pending: HashMap<String, (Snowflake, Snowflake)>,
    /// Gönderilemeyen iyimser mesajların geçici id'leri.
    failed: HashSet<Snowflake>,
    /// Kanal başına yazanlar: (kullanıcı, bitiş zamanı Unix saniyesi).
    typing: HashMap<Snowflake, Vec<(Snowflake, u64)>>,
    /// Kanal okunma durumu: son okunan mesaj ve bekleyen bahsetme sayısı.
    read: HashMap<Snowflake, unread::ReadMark>,
    /// Bildirim ayarları; anahtar `None` = DM'ler.
    notify_settings: HashMap<Option<Snowflake>, digcord_model::GuildSettings>,
    relationships: Vec<digcord_model::Relationship>,
    voice: voice::VoiceData,
    /// Üst kanal → thread'leri (arşivliler dahil, yüklü olanlar).
    thread_children: HashMap<Snowflake, Vec<Snowflake>>,
    /// Aktif thread listesi REST ile alınmış üst kanallar (forumlar).
    threads_fetched: HashSet<Snowflake>,
    /// Arşivli thread listesi istenmiş (altında dim gösterilen) üst kanallar.
    archived_shown: HashSet<Snowflake>,
    /// Guild başına güncel üye listesi (op 37 `channels` aboneliği).
    member_lists: HashMap<Snowflake, MemberList>,
}

fn is_voice(kind: u8) -> bool {
    matches!(kind, channel_kind::VOICE | channel_kind::STAGE)
}

fn is_dm(kind: u8) -> bool {
    matches!(kind, channel_kind::DM | channel_kind::GROUP_DM)
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn me(&self) -> Option<&User> {
        self.me.as_ref()
    }

    // ---------------------------------------------------------------- ingestion

    /// Yeni oturum: önceki her şeyi atar.
    pub fn load_ready(&mut self, ready: &Ready) {
        let version = self.version + 1;
        *self = Self::default();
        self.version = version;

        self.me = Some(ready.user.clone());
        self.users.insert(ready.user.id, ready.user.clone());
        for u in &ready.users {
            self.users.insert(u.id, u.clone());
        }
        for (i, g) in ready.guilds.iter().enumerate() {
            let my_roles = ready
                .merged_members
                .get(i)
                .and_then(|ms| ms.iter().find(|m| m.user_id == Some(ready.user.id)))
                .map(|m| m.roles.clone())
                .unwrap_or_default();
            self.guild_order.push(g.id);
            self.apply_guild(g, my_roles);
        }
        for c in &ready.private_channels {
            self.channels.insert(c.id, c.clone());
        }
        self.rebuild_dms();
        self.rebuild_guild_infos();
        self.load_read_state(ready);
    }

    /// GUILD_CREATE: kanalları, rolleri ve adı değiştirir; benim rollerim korunur.
    pub fn upsert_guild(&mut self, guild: &Guild) {
        self.version += 1;
        let my_roles = self
            .guilds
            .get(&guild.id)
            .map(|g| g.my_roles.clone())
            .unwrap_or_default();
        if !self.guilds.contains_key(&guild.id) {
            self.guild_order.push(guild.id);
        }
        // Eski kanalları temizle (yenide olmayanların mesajları da gider).
        if let Some(old) = self.guilds.get(&guild.id) {
            let old_ids = old.channel_ids.clone();
            for id in old_ids {
                if !guild.channels.iter().any(|c| c.id == id) {
                    self.channels.remove(&id);
                    self.msgs.remove(&id);
                }
            }
        }
        self.apply_guild(guild, my_roles);
        self.rebuild_guild_infos();
    }

    /// Guild verisini yazar (kanallar `guild_id` ile eşlenir) ve ağacı yeniler.
    fn apply_guild(&mut self, guild: &Guild, my_roles: Vec<Snowflake>) {
        if !guild.unavailable {
            self.load_guild_voice(guild);
        }
        let mut data = GuildData {
            available: !guild.unavailable,
            my_roles,
            ..GuildData::default()
        };
        if let Some(old) = self.guilds.remove(&guild.id) {
            // Kullanılamaz olarak gelen tekrar kayıt eski içeriği silmesin.
            if guild.unavailable {
                data.name = old.name;
                data.owner_id = old.owner_id;
                data.large = old.large;
                data.roles = old.roles;
                data.channel_ids = old.channel_ids;
                data.emojis = old.emojis;
            }
        }
        if !guild.unavailable {
            data.large = guild.large;
            data.roles = guild.roles.clone();
            data.channel_ids = guild.channels.iter().map(|c| c.id).collect();
            data.emojis = guild.emojis.clone();
            for c in &guild.channels {
                let mut c = c.clone();
                c.guild_id = Some(guild.id);
                self.channels.insert(c.id, c);
            }
        }
        if let Some(p) = &guild.properties {
            data.name = p.name.clone();
            data.owner_id = p.owner_id;
        }
        self.guilds.insert(guild.id, data);
        if !guild.unavailable {
            self.load_guild_threads(guild.id, &guild.threads);
        }
        self.rebuild_tree(guild.id);
    }

    pub fn upsert_channel(&mut self, channel: &Channel) {
        if channel.is_thread() {
            self.upsert_thread(channel);
            return;
        }
        self.version += 1;
        self.channels.insert(channel.id, channel.clone());
        if let Some(gid) = channel.guild_id {
            if let Some(g) = self.guilds.get_mut(&gid) {
                if !g.channel_ids.contains(&channel.id) {
                    g.channel_ids.push(channel.id);
                }
                self.rebuild_tree(gid);
            }
        } else if is_dm(channel.kind) {
            self.rebuild_dms();
        }
    }

    pub fn remove_channel(&mut self, id: Snowflake) {
        let Some(ch) = self.channels.remove(&id) else {
            return;
        };
        self.version += 1;
        self.msgs.remove(&id);
        if ch.is_thread() {
            self.forget_thread(id, ch.parent_id);
            return;
        }
        // Üst kanal silinince thread'leri de gider.
        for kid in self.thread_children.remove(&id).unwrap_or_default() {
            self.channels.remove(&kid);
            self.msgs.remove(&kid);
        }
        self.threads_fetched.remove(&id);
        self.archived_shown.remove(&id);
        if let Some(gid) = ch.guild_id {
            if let Some(g) = self.guilds.get_mut(&gid) {
                g.channel_ids.retain(|c| *c != id);
            }
            self.rebuild_tree(gid);
        } else {
            self.rebuild_dms();
        }
    }

    pub fn add_members(&mut self, chunk: &GuildMembersChunk) {
        self.version += 1;
        let me = self.me.as_ref().map(|u| u.id);
        for m in &chunk.members {
            let Some(uid) = m.user.as_ref().map(|u| u.id).or(m.user_id) else {
                continue;
            };
            if let Some(u) = &m.user {
                self.users.insert(u.id, u.clone());
            }
            self.members.insert(
                (chunk.guild_id, uid),
                MemberInfo {
                    nick: m.nick.clone(),
                },
            );
            if Some(uid) == me
                && let Some(g) = self.guilds.get_mut(&chunk.guild_id)
            {
                g.my_roles = m.roles.clone();
                self.rebuild_tree(chunk.guild_id);
            }
        }
    }

    pub fn message_create(&mut self, msg: Message) {
        self.version += 1;
        self.resolve_pending(&msg);
        self.stop_typing(msg.channel_id, msg.author.id);
        self.count_unread(&msg);
        self.record_author(&msg);
        let mut dm = false;
        if let Some(ch) = self.channels.get_mut(&msg.channel_id) {
            if ch.last_message_id.is_none_or(|l| l < msg.id) {
                ch.last_message_id = Some(msg.id);
            }
            dm = is_dm(ch.kind);
        }
        if let Some(cm) = self.msgs.get_mut(&msg.channel_id).filter(|c| !c.detached) {
            insert_sorted(&mut cm.messages, msg);
            if cm.messages.len() > HARD_CAP {
                cm.messages.pop_front();
                cm.start_reached = false;
            }
        }
        if dm {
            self.rebuild_dms();
        }
    }

    pub fn message_update(&mut self, msg: Message) {
        self.record_author(&msg);
        let Some(cm) = self.msgs.get_mut(&msg.channel_id) else {
            return;
        };
        if let Ok(i) = cm.messages.binary_search_by_key(&msg.id, |m| m.id) {
            cm.messages[i] = msg;
            self.version += 1;
        }
    }

    pub fn message_delete(&mut self, del: &MessageDelete) {
        let Some(cm) = self.msgs.get_mut(&del.channel_id) else {
            return;
        };
        if let Ok(i) = cm.messages.binary_search_by_key(&del.id, |m| m.id) {
            cm.messages.remove(i);
            self.version += 1;
        }
    }

    /// REST geçmişi; `messages` en yeniden eskiye gelir.
    /// `before == None`: ilk yükleme, kanalın içeriği değiştirilir. `Some`: eski sayfa başa eklenir
    /// (kanal hiç yüklenmemişse boşluk oluşmaması için yok sayılır).
    pub fn history_loaded(
        &mut self,
        channel: Snowflake,
        before: Option<Snowflake>,
        requested_limit: usize,
        messages: Vec<Message>,
    ) {
        if before.is_some() && !self.msgs.contains_key(&channel) {
            return;
        }
        self.version += 1;
        for m in &messages {
            self.record_author(m);
        }
        let reached = messages.len() < requested_limit;
        let mut asc = messages;
        asc.sort_by_key(|m| m.id);
        asc.dedup_by_key(|m| m.id);
        match before {
            None => {
                self.msgs.insert(
                    channel,
                    ChannelMessages {
                        messages: asc.into(),
                        start_reached: reached,
                        detached: false,
                    },
                );
            }
            Some(_) => {
                if let Some(cm) = self.msgs.get_mut(&channel) {
                    if let Some(first) = cm.messages.front().map(|m| m.id) {
                        asc.retain(|m| m.id < first);
                    }
                    for m in asc.into_iter().rev() {
                        cm.messages.push_front(m);
                    }
                    cm.start_reached |= reached;
                }
            }
        }
    }

    /// `GET /channels/{id}/messages?around=` sonucu (`messages` en yeniden eskiye). Pencereyi
    /// bozmadan birleştirir:
    /// - yeni parça mevcut pencereyle örtüşüyorsa ikisinin birleşimi tutulur, kopyalar tek kalır
    ///   (yeni veri eskinin yerine geçer);
    /// - örtüşmüyorsa pencere yenisiyle değiştirilir (arada boşluk olmaz).
    ///
    /// Sonuç kanalın en yeni mesajını içermiyorsa pencere "ayrık" işaretlenir ([`State::is_detached`]).
    /// En yeni mesaj, kanalın bilinen `last_message_id`'sine bakılarak; bilinmiyorsa istenenden
    /// az mesaj dönmesine bakılarak anlaşılır. Başlangıca ulaşıldığı bilinemediği için
    /// `start_reached` yeni pencerede `false` bırakılır (eski sayfa isteği boş dönünce düzelir).
    pub fn around_loaded(
        &mut self,
        channel: Snowflake,
        requested_limit: usize,
        messages: Vec<Message>,
    ) {
        self.version += 1;
        for m in &messages {
            self.record_author(m);
        }
        let mut asc = messages;
        asc.sort_by_key(|m| m.id);
        asc.dedup_by_key(|m| m.id);
        let (Some(first), Some(last)) = (asc.first().map(|m| m.id), asc.last().map(|m| m.id))
        else {
            return;
        };
        let at_latest = match self.channels.get(&channel).and_then(|c| c.last_message_id) {
            Some(latest) => last >= latest,
            None => asc.len() < requested_limit,
        };
        if let Some(cm) = self.msgs.get_mut(&channel)
            && let (Some(old_first), Some(old_last)) = (
                cm.messages.front().map(|m| m.id),
                cm.messages.back().map(|m| m.id),
            )
            && last >= old_first
            && first <= old_last
        {
            for m in asc {
                match cm.messages.binary_search_by_key(&m.id, |x| x.id) {
                    Ok(i) => cm.messages[i] = m,
                    Err(i) => cm.messages.insert(i, m),
                }
            }
            cm.detached = cm.detached && !at_latest;
            return;
        }
        self.msgs.insert(
            channel,
            ChannelMessages {
                messages: asc.into(),
                start_reached: false,
                detached: !at_latest,
            },
        );
    }

    /// Pencere kanalın en yeni mesajlarını içermiyor (bkz. [`State::around_loaded`]).
    pub fn is_detached(&self, channel: Snowflake) -> bool {
        self.msgs.get(&channel).is_some_and(|c| c.detached)
    }

    /// Kanaldan çıkınca: yalnızca en yeni `DEFAULT_WINDOW` mesaj kalır. Ayrık pencere (eski
    /// bir mesaja atlanmıştı) en yeni mesajları içermediğinden tamamen atılır.
    pub fn trim_channel(&mut self, channel: Snowflake) {
        if self.is_detached(channel) {
            self.msgs.remove(&channel);
            self.version += 1;
            return;
        }
        let Some(cm) = self.msgs.get_mut(&channel) else {
            return;
        };
        if cm.messages.len() > DEFAULT_WINDOW {
            let excess = cm.messages.len() - DEFAULT_WINDOW;
            cm.messages.drain(..excess);
            cm.start_reached = false;
            self.version += 1;
        }
    }

    // ------------------------------------------------------------------ queries

    pub fn guilds(&self) -> &[GuildInfo] {
        &self.guild_infos
    }

    pub fn guild_tree(&self, guild: Snowflake) -> &[TreeItem] {
        self.guilds.get(&guild).map_or(&[], |g| &g.tree)
    }

    pub fn dm_channels(&self) -> &[Snowflake] {
        &self.dms
    }

    pub fn channel(&self, id: Snowflake) -> Option<&Channel> {
        self.channels.get(&id)
    }

    pub fn channel_title(&self, id: Snowflake) -> String {
        let Some(ch) = self.channels.get(&id) else {
            return "?".to_owned();
        };
        if !is_dm(ch.kind) {
            return ch.name.clone().unwrap_or_else(|| "?".to_owned());
        }
        if ch.kind == channel_kind::GROUP_DM
            && let Some(n) = ch.name.as_deref().filter(|n| !n.is_empty())
        {
            return n.to_owned();
        }
        let mut out = String::new();
        for (i, r) in ch.recipient_ids.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            match self.users.get(r) {
                Some(u) => out.push_str(u.display_name()),
                None => out.push_str(&r.to_string()),
            }
        }
        if out.is_empty() { "?".to_owned() } else { out }
    }

    pub fn messages(&self, channel: Snowflake) -> Option<&VecDeque<Message>> {
        self.msgs.get(&channel).map(|c| &c.messages)
    }

    pub fn history_complete(&self, channel: Snowflake) -> bool {
        self.msgs.get(&channel).is_some_and(|c| c.start_reached)
    }

    /// Kanalın en yeni mesajlarını içeren bir pencere yüklü (ayrık pencere sayılmaz).
    pub fn is_loaded(&self, channel: Snowflake) -> bool {
        self.msgs.get(&channel).is_some_and(|c| !c.detached)
    }

    /// Bilinen kullanıcılar arasında kullanıcı adı ya da görünen ada göre ilk eşleşme
    /// (büyük/küçük harf duyarsız; `@` öneki yok sayılır).
    pub fn find_user_by_name(&self, name: &str) -> Option<&User> {
        let name = name.trim_start_matches('@').to_lowercase();
        if name.is_empty() {
            return None;
        }
        let mut found: Vec<&User> = self
            .users
            .values()
            .filter(|u| {
                u.username.to_lowercase() == name || u.display_name().to_lowercase() == name
            })
            .collect();
        found.sort_by_key(|u| u.id);
        found.into_iter().next()
    }

    pub fn user(&self, id: Snowflake) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn role(&self, guild: Snowflake, id: Snowflake) -> Option<&Role> {
        self.guilds.get(&guild)?.roles.iter().find(|r| r.id == id)
    }

    /// Takma ad (biliniyorsa) > global ad > kullanıcı adı.
    pub fn author_name(&self, guild: Option<Snowflake>, user: &User) -> String {
        guild
            .and_then(|g| self.members.get(&(g, user.id)))
            .and_then(|m| m.nick.as_deref())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| user.display_name())
            .to_owned()
    }

    /// Üye bilgisi bilinmeyen yazarlar (en çok `max`, tekrarsız). DM'de boş.
    pub fn unknown_members(&self, channel: Snowflake, max: usize) -> Vec<Snowflake> {
        let Some(gid) = self.channels.get(&channel).and_then(|c| c.guild_id) else {
            return Vec::new();
        };
        let Some(cm) = self.msgs.get(&channel) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for m in &cm.messages {
            if out.len() >= max {
                break;
            }
            let id = m.author.id;
            if !self.members.contains_key(&(gid, id)) && seen.insert(id) {
                out.push(id);
            }
        }
        out
    }

    // ---------------------------------------------------------------- internals

    fn record_author(&mut self, msg: &Message) {
        self.users.insert(msg.author.id, msg.author.clone());
        if let (Some(m), Some(gid)) = (&msg.member, msg.guild_id) {
            self.members.insert(
                (gid, msg.author.id),
                MemberInfo {
                    nick: m.nick.clone(),
                },
            );
        }
    }

    fn rebuild_guild_infos(&mut self) {
        self.guild_infos = self
            .guild_order
            .iter()
            .filter_map(|id| {
                let g = self.guilds.get(id)?;
                g.available.then(|| GuildInfo {
                    id: *id,
                    name: g.name.clone(),
                    large: g.large,
                })
            })
            .collect();
    }

    /// DM'ler son mesaja göre azalan; mesajı olmayanlar sonda.
    fn rebuild_dms(&mut self) {
        let mut v: Vec<(Option<Snowflake>, Snowflake)> = self
            .channels
            .values()
            .filter(|c| c.guild_id.is_none() && is_dm(c.kind))
            .map(|c| (c.last_message_id, c.id))
            .collect();
        v.sort_by_key(|&(last, id)| Reverse((last, id)));
        self.dms = v.into_iter().map(|(_, id)| id).collect();
    }

    fn rebuild_tree(&mut self, gid: Snowflake) {
        let (Some(g), Some(me)) = (self.guilds.get(&gid), self.me.as_ref()) else {
            if let Some(g) = self.guilds.get_mut(&gid) {
                g.tree.clear();
            }
            return;
        };
        let ctx = MemberContext {
            guild_id: gid,
            owner_id: g.owner_id,
            user_id: me.id,
            member_roles: &g.my_roles,
            roles: &g.roles,
        };
        let chans: Vec<&Channel> = g
            .channel_ids
            .iter()
            .filter_map(|id| self.channels.get(id))
            .collect();
        let sort_key = |c: &&Channel| (is_voice(c.kind), c.position.unwrap_or(0), c.id);
        let is_cat = |c: &&Channel| c.kind == channel_kind::CATEGORY;

        let mut cats: Vec<&Channel> = chans.iter().copied().filter(is_cat).collect();
        cats.sort_by_key(|c| (c.position.unwrap_or(0), c.id));
        let cat_ids: HashSet<Snowflake> = cats.iter().map(|c| c.id).collect();

        let mut visible: Vec<&Channel> = chans
            .iter()
            .copied()
            .filter(|c| {
                !is_cat(c)
                    && ctx
                        .channel(&c.permission_overwrites)
                        .contains(Permissions::VIEW_CHANNEL)
            })
            .collect();
        visible.sort_by_key(sort_key);

        let item = |c: &Channel, kind: TreeKind, depth: u8| TreeItem {
            id: c.id,
            kind,
            channel_kind: c.kind,
            name: c.name.clone().unwrap_or_else(|| "?".to_owned()),
            depth,
        };
        let parent = |c: &Channel| c.parent_id.filter(|p| cat_ids.contains(p));

        let mut tree = Vec::new();
        for c in visible.iter().filter(|c| parent(c).is_none()) {
            tree.push(item(c, TreeKind::Channel, 0));
        }
        for cat in cats {
            let mut kids = visible
                .iter()
                .filter(|c| parent(c) == Some(cat.id))
                .peekable();
            if kids.peek().is_none() {
                continue;
            }
            tree.push(item(cat, TreeKind::Category, 0));
            for k in kids {
                tree.push(item(k, TreeKind::Channel, 1));
            }
        }
        if let Some(g) = self.guilds.get_mut(&gid) {
            g.tree = tree;
        }
    }
}

/// Artan id sırasını koruyarak ekler; aynı id varsa yok sayar.
fn insert_sorted(deque: &mut VecDeque<Message>, msg: Message) {
    match deque.back() {
        Some(last) if last.id >= msg.id => {
            if let Err(i) = deque.binary_search_by_key(&msg.id, |m| m.id) {
                deque.insert(i, msg);
            }
        }
        _ => deque.push_back(msg),
    }
}

mod compose;
mod members;
mod threads;
mod unread;
mod voice;

pub use members::MemberList;
pub use threads::ThreadItem;
pub use unread::NotifyFilter;
pub use voice::{VoiceCall, VoicePhase};

#[cfg(test)]
mod tests;
